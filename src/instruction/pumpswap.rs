use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::pumpswap_ix_data::{
        encode_pumpswap_buy_exact_quote_in_ix_data, encode_pumpswap_buy_ix_data,
        encode_pumpswap_sell_ix_data,
    },
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::pumpswap::{
            accounts, fee_recipient_ata, get_mayhem_fee_recipient_random, get_pool_v2_pda,
            get_protocol_extra_fee_recipient_random, get_protocol_fee_recipient_random,
            get_user_volume_accumulator_pda, get_user_volume_accumulator_quote_ata,
            get_user_volume_accumulator_wsol_ata,
        },
    },
    trading::core::{
        params::{PumpSwapParams, SwapParams},
        traits::InstructionBuilder,
    },
    utils::calc::pumpswap::{
        buy_quote_input_internal_with_fees, sell_base_input_internal_with_fees,
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for PumpSwap protocol
pub struct PumpSwapInstructionBuilder;

#[inline]
fn request_mint_matches_pool(actual: Pubkey, expected: Pubkey) -> bool {
    actual == expected
        || (expected == crate::constants::WSOL_TOKEN_ACCOUNT
            && actual == crate::constants::SOL_TOKEN_ACCOUNT)
}

fn push_cashback_remaining_accounts(
    accounts: &mut Vec<AccountMeta>,
    user: &Pubkey,
    quote_mint: &Pubkey,
    quote_token_program: &Pubkey,
    is_cashback_coin: bool,
    is_buy_instruction: bool,
) -> Result<()> {
    if !is_cashback_coin {
        return Ok(());
    }

    let quote_ata = get_user_volume_accumulator_quote_ata(user, quote_mint, quote_token_program)
        .ok_or_else(|| anyhow!("user volume accumulator quote ATA derivation failed"))?;
    accounts.push(AccountMeta::new(quote_ata, false));

    if !is_buy_instruction {
        let accumulator = get_user_volume_accumulator_pda(user)
            .ok_or_else(|| anyhow!("user volume accumulator PDA derivation failed"))?;
        accounts.push(AccountMeta::new(accumulator, false));
    }

    Ok(())
}

#[async_trait::async_trait]
impl InstructionBuilder for PumpSwapInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpSwapParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpSwap"))?;

        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }
        if params.fixed_output_amount == Some(0) {
            return Err(anyhow!("Fixed output amount cannot be zero"));
        }

        let pool = protocol_params.pool;
        let base_mint = protocol_params.base_mint;
        let quote_mint = protocol_params.quote_mint;
        let pool_base_token_reserves = protocol_params.pool_base_token_reserves;
        let pool_quote_token_reserves = protocol_params.pool_quote_token_reserves;
        let virtual_quote_reserves = protocol_params.virtual_quote_reserves;
        protocol_params.effective_quote_reserves()?;
        let params_coin_creator_vault_ata = protocol_params.coin_creator_vault_ata;
        let params_coin_creator_vault_authority = protocol_params.coin_creator_vault_authority;
        let create_input_ata = params.create_input_mint_ata;
        let close_wsol_ata = params.close_input_mint_ata;
        let base_token_program = protocol_params.base_token_program;
        let quote_token_program = protocol_params.quote_token_program;
        let pool_base_token_account = protocol_params.pool_base_token_account;
        let pool_quote_token_account = protocol_params.pool_quote_token_account;

        let is_wsol = (base_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            && quote_mint != crate::constants::USDC_TOKEN_ACCOUNT)
            || (quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
                && base_mint != crate::constants::USDC_TOKEN_ACCOUNT);
        let is_usdc = (base_mint == crate::constants::USDC_TOKEN_ACCOUNT
            && quote_mint != crate::constants::WSOL_TOKEN_ACCOUNT)
            || (quote_mint == crate::constants::USDC_TOKEN_ACCOUNT
                && base_mint != crate::constants::WSOL_TOKEN_ACCOUNT);
        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let quote_is_wsol_or_usdc = quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || quote_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        if params.fixed_output_amount.is_some() && !quote_is_wsol_or_usdc {
            return Err(anyhow!(
                "PumpSwap exact-output buy is unsupported when the pool requires a sell instruction"
            ));
        }
        let input_stable_mint = if quote_is_wsol_or_usdc { quote_mint } else { base_mint };
        let input_stable_token_program =
            if quote_is_wsol_or_usdc { quote_token_program } else { base_token_program };
        let output_trade_mint = if quote_is_wsol_or_usdc { base_mint } else { quote_mint };
        let output_trade_token_program =
            if quote_is_wsol_or_usdc { base_token_program } else { quote_token_program };
        if !request_mint_matches_pool(params.input_mint, input_stable_mint)
            || !request_mint_matches_pool(params.output_mint, output_trade_mint)
        {
            return Err(anyhow!("PumpSwap buy request mints do not match the supplied pool"));
        }
        let fee_basis_points = protocol_params.fee_basis_points;

        let (token_amount, sol_amount) = if let Some(output_amount) = params.fixed_output_amount {
            if output_amount >= pool_base_token_reserves {
                return Err(anyhow!("Exact base output must be below the pool base reserve"));
            }
            (output_amount, params.input_amount.unwrap_or(0))
        } else if quote_is_wsol_or_usdc {
            let result = buy_quote_input_internal_with_fees(
                params.input_amount.unwrap_or(0),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                virtual_quote_reserves,
                &fee_basis_points,
            )
            .map_err(anyhow::Error::msg)?;
            // base_amount_out, max_quote_amount_in
            (result.base, result.max_quote)
        } else {
            let result = sell_base_input_internal_with_fees(
                params.input_amount.unwrap_or(0),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                virtual_quote_reserves,
                &fee_basis_points,
            )
            .map_err(anyhow::Error::msg)?;
            // min_quote_amount_out, base_amount_in
            (result.min_quote, params.input_amount.unwrap_or(0))
        };

        let user_base_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &base_mint,
                &base_token_program,
                params.open_seed_optimize,
            );
        let user_quote_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &quote_mint,
                &quote_token_program,
                params.open_seed_optimize,
            );

        // Determine fee recipient based on mayhem mode (pump-public-docs: 10th = Mayhem fee recipient, 11th = WSOL ATA of Mayhem; use any one randomly)
        let is_mayhem_mode = protocol_params.is_mayhem_mode;
        let (fee_recipient, fee_recipient_meta) = if is_mayhem_mode {
            get_mayhem_fee_recipient_random()
        } else {
            let recipient = get_protocol_fee_recipient_random();
            (recipient, AccountMeta::new_readonly(recipient, false))
        };
        let fee_recipient_ata = fee_recipient_ata(fee_recipient, quote_mint, quote_token_program);

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if create_input_ata {
            // Determine wrap amount based on instruction type:
            // - buy_exact_quote_in: program spends exactly input_amount, wrap input_amount
            // - buy: program may spend up to max_quote, wrap max_quote
            let wrap_amount =
                if quote_is_wsol_or_usdc && params.use_exact_sol_amount.unwrap_or(true) {
                    params.input_amount.unwrap_or(0)
                } else {
                    sol_amount
                };
            push_create_or_wrap_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &input_stable_mint,
                &input_stable_token_program,
                wrap_amount,
                params.open_seed_optimize,
            );
        }

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &output_trade_mint,
                &output_trade_token_program,
                params.open_seed_optimize,
            );
        }

        // Create buy instruction
        let mut accounts = Vec::with_capacity(28);
        accounts.extend([
            AccountMeta::new(pool, false),                          // pool_id
            AccountMeta::new(params.payer.pubkey(), true),          // user (signer)
            accounts::GLOBAL_ACCOUNT_META,                          // global (readonly)
            AccountMeta::new_readonly(base_mint, false),            // base_mint (readonly)
            AccountMeta::new_readonly(quote_mint, false),           // quote_mint (readonly)
            AccountMeta::new(user_base_token_account, false),       // user_base_token_account
            AccountMeta::new(user_quote_token_account, false),      // user_quote_token_account
            AccountMeta::new(pool_base_token_account, false),       // pool_base_token_account
            AccountMeta::new(pool_quote_token_account, false),      // pool_quote_token_account
            fee_recipient_meta,                                     // fee_recipient (readonly)
            AccountMeta::new(fee_recipient_ata, false),             // fee_recipient_ata
            AccountMeta::new_readonly(base_token_program, false),   // TOKEN_PROGRAM_ID (readonly)
            AccountMeta::new_readonly(quote_token_program, false), // TOKEN_PROGRAM_ID (readonly, duplicated as in JS)
            crate::constants::SYSTEM_PROGRAM_META,                 // System Program (readonly)
            accounts::ASSOCIATED_TOKEN_PROGRAM_META, // ASSOCIATED_TOKEN_PROGRAM_ID (readonly)
            accounts::EVENT_AUTHORITY_META,          // event_authority (readonly)
            accounts::AMM_PROGRAM_META,              // PUMP_AMM_PROGRAM_ID (readonly)
            AccountMeta::new(params_coin_creator_vault_ata, false), // coin_creator_vault_ata
            AccountMeta::new_readonly(params_coin_creator_vault_authority, false), // coin_creator_vault_authority (readonly)
        ]);
        if quote_is_wsol_or_usdc {
            accounts.push(accounts::GLOBAL_VOLUME_ACCUMULATOR_META);
            let uva = get_user_volume_accumulator_pda(&params.payer.pubkey())
                .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
            accounts.push(AccountMeta::new(uva, false));
        }
        accounts.push(accounts::FEE_CONFIG_META);
        accounts.push(accounts::FEE_PROGRAM_META);
        push_cashback_remaining_accounts(
            &mut accounts,
            &params.payer.pubkey(),
            &quote_mint,
            &quote_token_program,
            protocol_params.is_cashback_coin,
            quote_is_wsol_or_usdc,
        )?;
        // `pool-v2` only when coin_creator ≠ default (@pump-fun/pump-swap-sdk remainingAccounts)；
        // 否则多出的一格会把 buyback pubkey 错位，触发 BuybackFeeRecipientNotAuthorized（6053）。
        if protocol_params.coin_creator != Pubkey::default() {
            let pool_v2 = get_pool_v2_pda(&base_mint).ok_or_else(|| {
                anyhow!("pool_v2 PDA derivation failed for base_mint {}", base_mint)
            })?;
            accounts.push(AccountMeta::new_readonly(pool_v2, false));
        }
        // Trailing accounts: GlobalConfig.buyback_fee_recipients 中任 pubkey + quote ATA（与 pump-swap-sdk 静态池对齐；轮换时需查链上）。
        let protocol_extra = get_protocol_extra_fee_recipient_random();
        accounts.push(AccountMeta::new_readonly(protocol_extra, false));
        accounts.push(AccountMeta::new(
            crate::instruction::utils::pumpswap::fee_recipient_ata(
                protocol_extra,
                quote_mint,
                quote_token_program,
            ),
            false,
        ));

        // buy / buy_exact_quote_in：栈上 `[u8;25]` + `new_with_bytes`，避免每笔 `Vec` 堆分配。
        let track_volume = 1_u8;
        if quote_is_wsol_or_usdc {
            let ix_data = if params.fixed_output_amount.is_some() {
                encode_pumpswap_buy_ix_data(token_amount, sol_amount, track_volume)
            } else if params.use_exact_sol_amount.unwrap_or(true) {
                let min_base_amount_out = crate::utils::calc::common::calculate_with_slippage_sell(
                    token_amount,
                    params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                );
                encode_pumpswap_buy_exact_quote_in_ix_data(
                    params.input_amount.unwrap_or(0),
                    min_base_amount_out,
                    track_volume,
                )
            } else {
                encode_pumpswap_buy_ix_data(token_amount, sol_amount, track_volume)
            };
            instructions.push(Instruction::new_with_bytes(
                accounts::AMM_PROGRAM,
                &ix_data,
                accounts,
            ));
        } else {
            let ix_data = encode_pumpswap_sell_ix_data(sol_amount, token_amount);
            instructions.push(Instruction::new_with_bytes(
                accounts::AMM_PROGRAM,
                &ix_data,
                accounts,
            ));
        }
        if close_wsol_ata {
            push_close_wsol_if_needed(
                &mut instructions,
                &params.payer.pubkey(),
                &input_stable_mint,
            );
        }
        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpSwapParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpSwap"))?;

        let pool = protocol_params.pool;
        let base_mint = protocol_params.base_mint;
        let quote_mint = protocol_params.quote_mint;
        let pool_base_token_reserves = protocol_params.pool_base_token_reserves;
        let pool_quote_token_reserves = protocol_params.pool_quote_token_reserves;
        let virtual_quote_reserves = protocol_params.virtual_quote_reserves;
        protocol_params.effective_quote_reserves()?;
        let pool_base_token_account = protocol_params.pool_base_token_account;
        let pool_quote_token_account = protocol_params.pool_quote_token_account;
        let params_coin_creator_vault_ata = protocol_params.coin_creator_vault_ata;
        let params_coin_creator_vault_authority = protocol_params.coin_creator_vault_authority;
        let create_output_ata = params.create_output_mint_ata;
        let close_wsol_ata = params.close_output_mint_ata;
        let base_token_program = protocol_params.base_token_program;
        let quote_token_program = protocol_params.quote_token_program;

        let is_wsol = (base_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            && quote_mint != crate::constants::USDC_TOKEN_ACCOUNT)
            || (quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
                && base_mint != crate::constants::USDC_TOKEN_ACCOUNT);
        let is_usdc = (base_mint == crate::constants::USDC_TOKEN_ACCOUNT
            && quote_mint != crate::constants::WSOL_TOKEN_ACCOUNT)
            || (quote_mint == crate::constants::USDC_TOKEN_ACCOUNT
                && base_mint != crate::constants::WSOL_TOKEN_ACCOUNT);
        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        if params.input_amount.unwrap_or_default() == 0 {
            return Err(anyhow!("Token amount must be greater than zero"));
        }
        if params.fixed_output_amount == Some(0) {
            return Err(anyhow!("Fixed output amount cannot be zero"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let quote_is_wsol_or_usdc = quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || quote_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        if params.fixed_output_amount.is_some() && quote_is_wsol_or_usdc {
            return Err(anyhow!(
                "PumpSwap exact-output sell is unsupported when the pool requires a sell instruction"
            ));
        }
        let output_stable_mint = if quote_is_wsol_or_usdc { quote_mint } else { base_mint };
        let output_stable_token_program =
            if quote_is_wsol_or_usdc { quote_token_program } else { base_token_program };
        let input_trade_mint = if quote_is_wsol_or_usdc { base_mint } else { quote_mint };
        if !request_mint_matches_pool(params.input_mint, input_trade_mint)
            || !request_mint_matches_pool(params.output_mint, output_stable_mint)
        {
            return Err(anyhow!("PumpSwap sell request mints do not match the supplied pool"));
        }
        let fee_basis_points = protocol_params.fee_basis_points;

        let (token_amount, sol_amount) = if let Some(output_amount) = params.fixed_output_amount {
            if output_amount >= pool_base_token_reserves {
                return Err(anyhow!("Exact base output must be below the pool base reserve"));
            }
            (params.input_amount.unwrap(), output_amount)
        } else if quote_is_wsol_or_usdc {
            let result = sell_base_input_internal_with_fees(
                params.input_amount.unwrap(),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                virtual_quote_reserves,
                &fee_basis_points,
            )
            .map_err(anyhow::Error::msg)?;
            // base_amount_in, min_quote_amount_out
            (params.input_amount.unwrap(), result.min_quote)
        } else {
            let result = buy_quote_input_internal_with_fees(
                params.input_amount.unwrap(),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                virtual_quote_reserves,
                &fee_basis_points,
            )
            .map_err(anyhow::Error::msg)?;
            // max_quote_amount_in, base_amount_out
            (result.max_quote, result.base)
        };

        // Determine fee recipient based on mayhem mode (pump-public-docs: 10th = Mayhem fee recipient, 11th = WSOL ATA of Mayhem; use any one randomly)
        let is_mayhem_mode = protocol_params.is_mayhem_mode;
        let (fee_recipient, fee_recipient_meta) = if is_mayhem_mode {
            get_mayhem_fee_recipient_random()
        } else {
            let recipient = get_protocol_fee_recipient_random();
            (recipient, AccountMeta::new_readonly(recipient, false))
        };
        let fee_recipient_ata = fee_recipient_ata(fee_recipient, quote_mint, quote_token_program);

        let user_base_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &base_mint,
                &base_token_program,
                params.open_seed_optimize,
            );
        let user_quote_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &quote_mint,
                &quote_token_program,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(4);

        if create_output_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &output_stable_mint,
                &output_stable_token_program,
                params.open_seed_optimize,
            );
        }

        // Create sell instruction
        let mut accounts = Vec::with_capacity(28);
        accounts.extend([
            AccountMeta::new(pool, false),                          // pool_id
            AccountMeta::new(params.payer.pubkey(), true),          // user (signer)
            accounts::GLOBAL_ACCOUNT_META,                          // global (readonly)
            AccountMeta::new_readonly(base_mint, false),            // mint (readonly)
            AccountMeta::new_readonly(quote_mint, false),           // WSOL_TOKEN_ACCOUNT (readonly)
            AccountMeta::new(user_base_token_account, false),       // user_base_token_account
            AccountMeta::new(user_quote_token_account, false),      // user_quote_token_account
            AccountMeta::new(pool_base_token_account, false),       // pool_base_token_account
            AccountMeta::new(pool_quote_token_account, false),      // pool_quote_token_account
            fee_recipient_meta,                                     // fee_recipient (readonly)
            AccountMeta::new(fee_recipient_ata, false),             // fee_recipient_ata
            AccountMeta::new_readonly(base_token_program, false),   // TOKEN_PROGRAM_ID (readonly)
            AccountMeta::new_readonly(quote_token_program, false), // TOKEN_PROGRAM_ID (readonly, duplicated as in JS)
            crate::constants::SYSTEM_PROGRAM_META,                 // System Program (readonly)
            accounts::ASSOCIATED_TOKEN_PROGRAM_META, // ASSOCIATED_TOKEN_PROGRAM_ID (readonly)
            accounts::EVENT_AUTHORITY_META,          // event_authority (readonly)
            accounts::AMM_PROGRAM_META,              // PUMP_AMM_PROGRAM_ID (readonly)
            AccountMeta::new(params_coin_creator_vault_ata, false), // coin_creator_vault_ata
            AccountMeta::new_readonly(params_coin_creator_vault_authority, false), // coin_creator_vault_authority (readonly)
        ]);
        if !quote_is_wsol_or_usdc {
            accounts.push(accounts::GLOBAL_VOLUME_ACCUMULATOR_META);
            let uva = get_user_volume_accumulator_pda(&params.payer.pubkey())
                .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
            accounts.push(AccountMeta::new(uva, false));
        }
        accounts.push(accounts::FEE_CONFIG_META);
        accounts.push(accounts::FEE_PROGRAM_META);
        push_cashback_remaining_accounts(
            &mut accounts,
            &params.payer.pubkey(),
            &quote_mint,
            &quote_token_program,
            protocol_params.is_cashback_coin,
            !quote_is_wsol_or_usdc,
        )?;
        if protocol_params.coin_creator != Pubkey::default() {
            let pool_v2 = get_pool_v2_pda(&base_mint).ok_or_else(|| {
                anyhow!("pool_v2 PDA derivation failed for base_mint {}", base_mint)
            })?;
            accounts.push(AccountMeta::new_readonly(pool_v2, false));
        }
        let protocol_extra = get_protocol_extra_fee_recipient_random();
        accounts.push(AccountMeta::new_readonly(protocol_extra, false));
        accounts.push(AccountMeta::new(
            crate::instruction::utils::pumpswap::fee_recipient_ata(
                protocol_extra,
                quote_mint,
                quote_token_program,
            ),
            false,
        ));

        // 栈数组 + `new_with_bytes`，避免 `data.to_vec()`。
        let track_volume = 1_u8;
        if quote_is_wsol_or_usdc {
            let ix_data = encode_pumpswap_sell_ix_data(token_amount, sol_amount);
            instructions.push(Instruction::new_with_bytes(
                accounts::AMM_PROGRAM,
                &ix_data,
                accounts,
            ));
        } else if params.fixed_output_amount.is_some() {
            let ix_data = encode_pumpswap_buy_ix_data(sol_amount, token_amount, track_volume);
            instructions.push(Instruction::new_with_bytes(
                accounts::AMM_PROGRAM,
                &ix_data,
                accounts,
            ));
        } else {
            let min_base_amount_out = crate::utils::calc::common::calculate_with_slippage_sell(
                sol_amount,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            );
            let ix_data = encode_pumpswap_buy_exact_quote_in_ix_data(
                params.input_amount.unwrap_or(0),
                min_base_amount_out,
                track_volume,
            );
            instructions.push(Instruction::new_with_bytes(
                accounts::AMM_PROGRAM,
                &ix_data,
                accounts,
            ));
        }

        if close_wsol_ata {
            push_close_wsol_if_needed(
                &mut instructions,
                &params.payer.pubkey(),
                &output_stable_mint,
            );
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                if quote_is_wsol_or_usdc { &base_token_program } else { &quote_token_program },
                if quote_is_wsol_or_usdc {
                    &user_base_token_account
                } else {
                    &user_quote_token_account
                },
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &[&params.payer.pubkey()],
            )?);
        }
        Ok(instructions)
    }
}

/// Claim cashback for PumpSwap (AMM). Transfers WSOL from UserVolumeAccumulator's WSOL ATA to user's WSOL ATA.
/// Caller should ensure user's WSOL ATA exists (e.g. create idempotent ATA instruction) before this instruction.
pub fn claim_cashback_pumpswap_instruction(
    payer: &Pubkey,
    quote_mint: Pubkey,
    quote_token_program: Pubkey,
) -> Option<solana_sdk::instruction::Instruction> {
    const CLAIM_CASHBACK_DISCRIMINATOR: [u8; 8] = [37, 58, 35, 126, 190, 53, 228, 197];
    let user_volume_accumulator = get_user_volume_accumulator_pda(payer)?;
    let user_volume_accumulator_wsol_ata = get_user_volume_accumulator_wsol_ata(payer)?;
    let user_wsol_ata = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        payer,
        &quote_mint,
        &quote_token_program,
    );
    // IDL order: user, user_volume_accumulator, quote_mint, quote_token_program,
    // user_volume_accumulator_wsol_token_account, user_wsol_token_account, system_program, event_authority, program
    let accounts = vec![
        AccountMeta::new(*payer, true), // user (signer, writable)
        AccountMeta::new(user_volume_accumulator, false), // user_volume_accumulator (writable)
        AccountMeta::new_readonly(quote_mint, false),
        AccountMeta::new_readonly(quote_token_program, false),
        AccountMeta::new(user_volume_accumulator_wsol_ata, false), // writable
        AccountMeta::new(user_wsol_ata, false),                    // writable
        crate::constants::SYSTEM_PROGRAM_META,
        accounts::EVENT_AUTHORITY_META,
        accounts::AMM_PROGRAM_META,
    ];
    Some(solana_sdk::instruction::Instruction::new_with_bytes(
        accounts::AMM_PROGRAM,
        &CLAIM_CASHBACK_DISCRIMINATOR,
        accounts,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        common::GasFeeStrategy,
        swqos::TradeType,
        trading::core::params::{DexParamEnum, PumpSwapParams, SwapParams},
    };
    use solana_sdk::{pubkey::Pubkey, signature::Keypair};
    use std::sync::Arc;

    fn pk(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    fn pumpswap_params() -> PumpSwapParams {
        PumpSwapParams::new(
            pk(1),
            pk(2),
            crate::constants::WSOL_TOKEN_ACCOUNT,
            pk(3),
            pk(4),
            1_000_000_000,
            2_000_000_000,
            0,
            pk(5),
            accounts::DEFAULT_COIN_CREATOR_VAULT_AUTHORITY,
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            accounts::PROTOCOL_FEE_RECIPIENT,
            Pubkey::default(),
            false,
            0,
        )
    }

    fn reverse_pumpswap_params() -> PumpSwapParams {
        PumpSwapParams::new(
            pk(1),
            crate::constants::USDC_TOKEN_ACCOUNT,
            pk(2),
            pk(3),
            pk(4),
            1_000_000_000,
            2_000_000_000,
            0,
            pk(5),
            accounts::DEFAULT_COIN_CREATOR_VAULT_AUTHORITY,
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            accounts::PROTOCOL_FEE_RECIPIENT,
            Pubkey::default(),
            false,
            0,
        )
    }

    fn swap_params(trade_type: TradeType, fixed_output_amount: Option<u64>) -> SwapParams {
        let (input_mint, output_mint) = if trade_type == TradeType::Sell {
            (pk(2), crate::constants::WSOL_TOKEN_ACCOUNT)
        } else {
            (crate::constants::WSOL_TOKEN_ACCOUNT, pk(2))
        };
        SwapParams {
            rpc: None,
            payer: Arc::new(Keypair::new()),
            trade_type,
            input_mint,
            input_token_program: None,
            output_mint,
            output_token_program: None,
            input_amount: Some(100_000),
            slippage_basis_points: Some(100),
            address_lookup_table_accounts: Vec::new(),
            recent_blockhash: None,
            wait_tx_confirmed: false,
            protocol_params: DexParamEnum::PumpSwap(pumpswap_params()),
            open_seed_optimize: true,
            swqos_clients: Arc::new(Vec::new()),
            middleware_manager: None,
            durable_nonce: None,
            with_tip: false,
            create_input_mint_ata: false,
            close_input_mint_ata: false,
            create_output_mint_ata: false,
            close_output_mint_ata: false,
            fixed_output_amount,
            gas_fee_strategy: GasFeeStrategy::new(),
            simulate: true,
            log_enabled: false,
            wait_for_all_submits: false,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: 0,
            effective_core_ids: Arc::new(Vec::new()),
            check_min_tip: false,
            grpc_recv_us: None,
            use_exact_sol_amount: Some(true),
        }
    }

    #[tokio::test]
    async fn pumpswap_fixed_output_uses_buy_with_max_input_budget() {
        let instructions = PumpSwapInstructionBuilder
            .build_buy_instructions(&swap_params(TradeType::Buy, Some(42)))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpswap::BUY_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 42);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 100_000);
    }

    #[tokio::test]
    async fn pumpswap_sell_fixed_output_rejects_unsupported_sell_instruction() {
        let error = PumpSwapInstructionBuilder
            .build_sell_instructions(&swap_params(TradeType::Sell, Some(42)))
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "PumpSwap exact-output sell is unsupported when the pool requires a sell instruction"
        );
    }

    #[tokio::test]
    async fn pumpswap_reverse_sell_fixed_output_uses_current_buy_layout() {
        let mut params = swap_params(TradeType::Sell, Some(42));
        params.protocol_params = DexParamEnum::PumpSwap(reverse_pumpswap_params());
        params.output_mint = crate::constants::USDC_TOKEN_ACCOUNT;

        let instructions =
            PumpSwapInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpswap::BUY_DISCRIMINATOR);
        assert_eq!(ix.data.len(), 25);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 42);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 100_000);
        assert_eq!(ix.data[24], 1);
    }

    #[tokio::test]
    async fn pumpswap_reverse_sell_exact_input_never_increases_token_spend() {
        let mut params = swap_params(TradeType::Sell, None);
        params.protocol_params = DexParamEnum::PumpSwap(reverse_pumpswap_params());
        params.output_mint = crate::constants::USDC_TOKEN_ACCOUNT;

        let instructions =
            PumpSwapInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(
            &ix.data[..8],
            crate::instruction::utils::pumpswap::BUY_EXACT_QUOTE_IN_DISCRIMINATOR
        );
        assert_eq!(ix.data.len(), 25);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 100_000);

        let quote = crate::utils::calc::pumpswap::buy_quote_input_internal_with_fees(
            100_000,
            100,
            1_000_000_000,
            2_000_000_000,
            0,
            &crate::instruction::utils::pumpswap::PumpSwapFeeBasisPoints::new(25, 5, 0),
        )
        .unwrap();
        let expected_min =
            crate::utils::calc::common::calculate_with_slippage_sell(quote.base, 100);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), expected_min);
        assert_eq!(ix.data[24], 1);
    }

    #[tokio::test]
    async fn pumpswap_buy_cashback_accounts_match_actual_instruction_direction() {
        let mut params = swap_params(TradeType::Buy, None);
        let mut protocol_params = pumpswap_params();
        protocol_params.is_cashback_coin = true;
        params.protocol_params = DexParamEnum::PumpSwap(protocol_params);

        let instructions =
            PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let ix = instructions.last().unwrap();
        let expected_ata = get_user_volume_accumulator_quote_ata(
            &params.payer.pubkey(),
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        )
        .unwrap();

        assert_eq!(
            &ix.data[..8],
            crate::instruction::utils::pumpswap::BUY_EXACT_QUOTE_IN_DISCRIMINATOR
        );
        assert_eq!(ix.accounts[23].pubkey, expected_ata);
        assert_eq!(ix.data[24], 1);
    }

    #[tokio::test]
    async fn pumpswap_reverse_buy_uses_sell_cashback_account_layout() {
        let mut params = swap_params(TradeType::Buy, None);
        let mut protocol_params = reverse_pumpswap_params();
        protocol_params.is_cashback_coin = true;
        params.protocol_params = DexParamEnum::PumpSwap(protocol_params);
        params.input_mint = crate::constants::USDC_TOKEN_ACCOUNT;

        let instructions =
            PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let ix = instructions.last().unwrap();
        let expected_ata = get_user_volume_accumulator_quote_ata(
            &params.payer.pubkey(),
            &pk(2),
            &crate::constants::TOKEN_PROGRAM,
        )
        .unwrap();
        let expected_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey()).unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpswap::SELL_DISCRIMINATOR);
        assert_eq!(ix.accounts[21].pubkey, expected_ata);
        assert_eq!(ix.accounts[22].pubkey, expected_accumulator);
    }

    #[tokio::test]
    async fn pumpswap_fee_atas_use_the_quote_token_program() {
        let quote_token_program = pk(99);
        let mut params = swap_params(TradeType::Sell, None);
        let mut protocol_params = reverse_pumpswap_params();
        protocol_params.quote_token_program = quote_token_program;
        params.protocol_params = DexParamEnum::PumpSwap(protocol_params);
        params.output_mint = crate::constants::USDC_TOKEN_ACCOUNT;

        let instructions =
            PumpSwapInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        let ix = instructions.last().unwrap();
        let expected_fee_ata =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &ix.accounts[9].pubkey,
                &pk(2),
                &quote_token_program,
            );
        let buyback_recipient_index = ix.accounts.len() - 2;
        let expected_buyback_ata =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &ix.accounts[buyback_recipient_index].pubkey,
                &pk(2),
                &quote_token_program,
            );

        assert_eq!(ix.accounts[10].pubkey, expected_fee_ata);
        assert_eq!(ix.accounts[buyback_recipient_index + 1].pubkey, expected_buyback_ata);
    }

    #[tokio::test]
    async fn pumpswap_rejects_request_mints_from_another_pool() {
        let mut params = swap_params(TradeType::Buy, None);
        params.output_mint = pk(88);

        let error = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap_err();

        assert_eq!(error.to_string(), "PumpSwap buy request mints do not match the supplied pool");
    }

    #[tokio::test]
    async fn pumpswap_sell_rejects_zero_input() {
        let mut params = swap_params(TradeType::Sell, None);
        params.input_amount = Some(0);

        let error = PumpSwapInstructionBuilder.build_sell_instructions(&params).await.unwrap_err();

        assert_eq!(error.to_string(), "Token amount must be greater than zero");
    }

    #[tokio::test]
    async fn pumpswap_usdc_buy_create_input_builds_usdc_ata() {
        let mut params = swap_params(TradeType::Buy, Some(42));
        params.protocol_params = DexParamEnum::PumpSwap(PumpSwapParams::new(
            pk(1),
            pk(2),
            crate::constants::USDC_TOKEN_ACCOUNT,
            pk(3),
            pk(4),
            1_000_000_000,
            2_000_000_000,
            0,
            pk(5),
            accounts::DEFAULT_COIN_CREATOR_VAULT_AUTHORITY,
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            accounts::PROTOCOL_FEE_RECIPIENT,
            Pubkey::default(),
            false,
            0,
        ));
        params.input_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        params.create_input_mint_ata = true;
        params.open_seed_optimize = false;

        let instructions =
            PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let create_ix = instructions.first().unwrap();

        assert_eq!(create_ix.program_id, crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID);
        assert_eq!(create_ix.accounts[3].pubkey, crate::constants::USDC_TOKEN_ACCOUNT);
    }

    #[tokio::test]
    async fn pumpswap_buy_uses_fee_basis_points_from_params_without_rpc() {
        let mut params = swap_params(TradeType::Buy, None);
        params.input_amount = Some(1_000_000);
        params.use_exact_sol_amount = Some(false);
        let mut protocol_params = pumpswap_params().with_fee_basis_points(20, 5, 75);
        protocol_params.virtual_quote_reserves = 500_000_000;
        params.protocol_params = DexParamEnum::PumpSwap(protocol_params);

        let instructions =
            PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], crate::instruction::utils::pumpswap::BUY_DISCRIMINATOR);
        let base_amount_out = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());

        let expected = crate::utils::calc::pumpswap::buy_quote_input_internal_with_fees(
            1_000_000,
            100,
            1_000_000_000,
            2_000_000_000,
            500_000_000,
            &crate::instruction::utils::pumpswap::PumpSwapFeeBasisPoints::new(20, 5, 0),
        )
        .unwrap();
        assert_eq!(base_amount_out, expected.base);
    }

    #[tokio::test]
    async fn pumpswap_sell_prices_with_effective_quote_reserves() {
        let mut params = swap_params(TradeType::Sell, None);
        let mut protocol_params = pumpswap_params().with_fee_basis_points(20, 5, 0);
        protocol_params.virtual_quote_reserves = 500_000_000;
        params.protocol_params = DexParamEnum::PumpSwap(protocol_params);

        let instructions =
            PumpSwapInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        let ix = instructions.last().unwrap();
        let min_quote_amount_out = u64::from_le_bytes(ix.data[16..24].try_into().unwrap());

        let expected = crate::utils::calc::pumpswap::sell_base_input_internal_with_fees(
            100_000,
            100,
            1_000_000_000,
            2_000_000_000,
            500_000_000,
            &crate::instruction::utils::pumpswap::PumpSwapFeeBasisPoints::new(20, 5, 0),
        )
        .unwrap();
        assert_eq!(min_quote_amount_out, expected.min_quote);
    }
}
