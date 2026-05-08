use crate::{
    common::spl_token::close_account,
    constants::{trade::trade::DEFAULT_SLIPPAGE, TOKEN_PROGRAM_2022},
    trading::core::{
        params::{PumpFunParams, SwapParams},
        traits::InstructionBuilder,
    },
};
use crate::{
    instruction::utils::pumpfun::{
        accounts, get_bonding_curve_pda, get_bonding_curve_v2_pda,
        get_buyback_fee_recipient_random, get_protocol_extra_fee_recipient_random,
        get_quote_token_program, get_user_volume_accumulator_pda,
        pump_fun_fee_recipient_meta, resolve_creator_vault_for_ix,
        global_constants::{self},
        BUY_DISCRIMINATOR, BUY_EXACT_SOL_IN_DISCRIMINATOR, BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR,
        BUY_V2_DISCRIMINATOR, SELL_DISCRIMINATOR, SELL_V2_DISCRIMINATOR,
    },
    utils::calc::{
        common::{calculate_with_slippage_buy, calculate_with_slippage_sell},
        pumpfun::{get_buy_token_amount_from_sol_amount, get_sell_sol_amount_from_token_amount},
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::instruction::AccountMeta;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signer::Signer};

/// Instruction builder for PumpFun protocol
pub struct PumpFunInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for PumpFunInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpFunParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;

        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        let bonding_curve = &protocol_params.bonding_curve;
        // creator_vault must be PDA(creator) per bonding curve. Event vault: use only if == derived;
        // if stream sends a mismatched vault (wrong token / stale), fall back to derived.
        let creator = bonding_curve.creator;
        let creator_vault_pda = resolve_creator_vault_for_ix(
            &creator,
            protocol_params.creator_vault,
            &params.output_mint,
        )
        .ok_or_else(|| {
            anyhow!(
                "creator_vault PDA derivation failed (creator={})",
                creator
            )
        })?;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let buy_token_amount = match params.fixed_output_amount {
            Some(amount) => amount,
            None => get_buy_token_amount_from_sol_amount(
                bonding_curve.virtual_token_reserves as u128,
                bonding_curve.virtual_sol_reserves as u128,
                bonding_curve.real_token_reserves as u128,
                creator,
                params.input_amount.unwrap_or(0),
            ),
        };

        let max_sol_cost = calculate_with_slippage_buy(
            params.input_amount.unwrap_or(0),
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        );

        // 始终用 mint 推导 canonical bonding curve PDA。缓存里的 `bonding_curve.account` 可能指向其它池子，
        // 会导致链上读到错误 `creator`，从而 creator_vault seeds 与传入的 vault 不一致（Anchor 2006）。
        let bonding_curve_addr = get_bonding_curve_pda(&params.output_mint).ok_or_else(|| {
            anyhow!("bonding_curve PDA derivation failed for mint {}", params.output_mint)
        })?;

        // Determine token program based on mayhem mode
        let is_mayhem_mode = bonding_curve.is_mayhem_mode;
        let token_program = protocol_params.token_program;
        let token_program_meta = if protocol_params.token_program == TOKEN_PROGRAM_2022 {
            crate::constants::TOKEN_PROGRAM_2022_META
        } else {
            crate::constants::TOKEN_PROGRAM_META
        };

        let associated_bonding_curve =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &bonding_curve_addr,
                &params.output_mint,
                &token_program,
            );

        let user_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.output_mint,
                &token_program,
                params.open_seed_optimize,
            );

        let user_volume_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey())
            .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;

        // ========================================
        // Build instructions
        // ========================================
        // Hot path: no RPC here (latency). For legacy curves &lt;151 bytes, use
        // `extend_bonding_curve_account_instruction` from a cold path or separate tx.
        let mut instructions = Vec::with_capacity(2);

        // Create associated token account
        if params.create_output_mint_ata {
            instructions.extend(
                crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &params.output_mint,
                    &token_program,
                    params.open_seed_optimize,
                ),
            );
        }

        // IDL: buy/buy_exact_sol_in 第三参数 track_volume: OptionBool，仅代币支持返现时传 Some(true)
        let track_volume = if bonding_curve.is_cashback_coin { [1u8, 1u8] } else { [1u8, 0u8] }; // Some(true) / Some(false)

        // Fee recipient: gRPC/ShredStream 填入的 `PumpFunParams.fee_recipient`（同笔 create_v2+buy 或 trade 日志）优先；热路径无 RPC。
        let fee_recipient_meta =
            pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);

        let bonding_curve_v2 = get_bonding_curve_v2_pda(&params.output_mint).ok_or_else(|| {
            anyhow!("bonding_curve_v2 PDA derivation failed for mint {}", params.output_mint)
        })?;

        // ========================================
        // V2 instructions: unified 27-account layout (quote_mint-aware)
        // ========================================
        if protocol_params.use_v2_ix {
            let quote_mint = if protocol_params.quote_mint != Pubkey::default() {
                protocol_params.quote_mint
            } else {
                crate::constants::WSOL_TOKEN_ACCOUNT
            };
            let quote_token_program = get_quote_token_program(&quote_mint);
            let quote_token_program_meta = if quote_token_program == crate::constants::TOKEN_PROGRAM {
                crate::constants::TOKEN_PROGRAM_META
            } else {
                crate::constants::TOKEN_PROGRAM_2022_META
            };

            let associated_quote_bonding_curve =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &bonding_curve_addr,
                    &quote_mint,
                    &quote_token_program,
                );
            let associated_quote_user =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                    &params.payer.pubkey(),
                    &quote_mint,
                    &quote_token_program,
                    params.open_seed_optimize,
                );
            let fee_recipient_pk = fee_recipient_meta.pubkey;
            let associated_quote_fee_recipient =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &fee_recipient_pk,
                    &quote_mint,
                    &quote_token_program,
                );
            let buyback_fee_recipient = get_buyback_fee_recipient_random();
            let associated_quote_buyback_fee_recipient =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &buyback_fee_recipient,
                    &quote_mint,
                    &quote_token_program,
                );
            let associated_creator_vault =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &creator_vault_pda,
                    &quote_mint,
                    &quote_token_program,
                );
            let user_volume_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey())
                .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
            let associated_user_volume_accumulator =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &user_volume_accumulator,
                    &quote_mint,
                    &quote_token_program,
                );
            let sharing_config =
                crate::instruction::utils::pumpfun::get_fee_sharing_config_pda(&params.output_mint)
                    .ok_or_else(|| anyhow!("sharing_config PDA derivation failed"))?;

            let mut v2_data = [0u8; 24];
            if params.use_exact_sol_amount.unwrap_or(true) {
                // buy_exact_quote_in_v2(spendable_quote_in: u64, min_tokens_out: u64)
                let min_tokens_out = calculate_with_slippage_sell(
                    buy_token_amount,
                    params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                );
                v2_data[..8].copy_from_slice(&BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR);
                v2_data[8..16].copy_from_slice(&params.input_amount.unwrap_or(0).to_le_bytes());
                v2_data[16..24].copy_from_slice(&min_tokens_out.to_le_bytes());
            } else {
                // buy_v2(amount: u64, max_sol_cost: u64)
                v2_data[..8].copy_from_slice(&BUY_V2_DISCRIMINATOR);
                v2_data[8..16].copy_from_slice(&buy_token_amount.to_le_bytes());
                v2_data[16..24].copy_from_slice(&max_sol_cost.to_le_bytes());
            }

            let v2_accounts: Vec<AccountMeta> = vec![
                global_constants::GLOBAL_ACCOUNT_META,                 // 1: global
                AccountMeta::new_readonly(params.output_mint, false),  // 2: base_mint
                AccountMeta::new_readonly(quote_mint, false),          // 3: quote_mint
                token_program_meta,                                    // 4: base_token_program
                quote_token_program_meta,                              // 5: quote_token_program
                crate::constants::ASSOCIATED_TOKEN_PROGRAM_META,       // 6: associated_token_program
                fee_recipient_meta,                                    // 7: fee_recipient
                AccountMeta::new(associated_quote_fee_recipient, false), // 8: associated_quote_fee_recipient
                AccountMeta::new_readonly(buyback_fee_recipient, false), // 9: buyback_fee_recipient
                AccountMeta::new(associated_quote_buyback_fee_recipient, false), // 10: associated_quote_buyback_fee_recipient
                AccountMeta::new(bonding_curve_addr, false),           // 11: bonding_curve
                AccountMeta::new(associated_bonding_curve, false),     // 12: associated_base_bonding_curve
                AccountMeta::new(associated_quote_bonding_curve, false), // 13: associated_quote_bonding_curve
                AccountMeta::new(params.payer.pubkey(), true),         // 14: user
                AccountMeta::new(user_token_account, false),           // 15: associated_base_user
                AccountMeta::new(associated_quote_user, false),        // 16: associated_quote_user
                AccountMeta::new(creator_vault_pda, false),            // 17: creator_vault
                AccountMeta::new(associated_creator_vault, false),     // 18: associated_creator_vault
                AccountMeta::new_readonly(sharing_config, false),      // 19: sharing_config
                accounts::GLOBAL_VOLUME_ACCUMULATOR_META,              // 20: global_volume_accumulator
                AccountMeta::new(user_volume_accumulator, false),      // 21: user_volume_accumulator
                AccountMeta::new(associated_user_volume_accumulator, false), // 22: associated_user_volume_accumulator
                accounts::FEE_CONFIG_META,                             // 23: fee_config
                accounts::FEE_PROGRAM_META,                            // 24: fee_program
                crate::constants::SYSTEM_PROGRAM_META,                 // 25: system_program
                accounts::EVENT_AUTHORITY_META,                        // 26: event_authority
                accounts::PUMPFUN_META,                                // 27: program
            ];

            if params.create_output_mint_ata {
                instructions.extend(
                    crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                        &params.payer.pubkey(),
                        &params.payer.pubkey(),
                        &params.output_mint,
                        &token_program,
                        params.open_seed_optimize,
                    ),
                );
            }
            // Also create quote ATA if it doesn't exist (SOL-paired: no-op since SOL is native)
            if quote_mint != crate::constants::WSOL_TOKEN_ACCOUNT {
                let create_quote_ata = crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &quote_mint,
                    &quote_token_program,
                    params.open_seed_optimize,
                );
                if !create_quote_ata.is_empty() {
                    instructions.extend(create_quote_ata);
                }
            }

            instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, &v2_data, v2_accounts));
            return Ok(instructions);
        }

        // ========================================
        // Legacy instructions (backward compatible)
        // ========================================
        let mut buy_data = [0u8; 26];
        if params.use_exact_sol_amount.unwrap_or(true) {
            // buy_exact_sol_in(spendable_sol_in: u64, min_tokens_out: u64, track_volume)
            let min_tokens_out = calculate_with_slippage_sell(
                buy_token_amount,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            );
            buy_data[..8].copy_from_slice(&BUY_EXACT_SOL_IN_DISCRIMINATOR);
            buy_data[8..16].copy_from_slice(&params.input_amount.unwrap_or(0).to_le_bytes());
            buy_data[16..24].copy_from_slice(&min_tokens_out.to_le_bytes());
            buy_data[24..26].copy_from_slice(&track_volume);
        } else {
            // buy(token_amount: u64, max_sol_cost: u64, track_volume)
            buy_data[..8].copy_from_slice(&BUY_DISCRIMINATOR);
            buy_data[8..16].copy_from_slice(&buy_token_amount.to_le_bytes());
            buy_data[16..24].copy_from_slice(&max_sol_cost.to_le_bytes());
            buy_data[24..26].copy_from_slice(&track_volume);
        }

        let mut accounts: Vec<AccountMeta> = vec![
            global_constants::GLOBAL_ACCOUNT_META,
            fee_recipient_meta,
            AccountMeta::new_readonly(params.output_mint, false),
            AccountMeta::new(bonding_curve_addr, false),
            AccountMeta::new(associated_bonding_curve, false),
            AccountMeta::new(user_token_account, false),
            AccountMeta::new(params.payer.pubkey(), true),
            crate::constants::SYSTEM_PROGRAM_META,
            token_program_meta,
            AccountMeta::new(creator_vault_pda, false),
            accounts::EVENT_AUTHORITY_META,
            accounts::PUMPFUN_META,
            accounts::GLOBAL_VOLUME_ACCUMULATOR_META,
            AccountMeta::new(user_volume_accumulator, false),
            accounts::FEE_CONFIG_META,
            accounts::FEE_PROGRAM_META,
        ];
        accounts.push(AccountMeta::new_readonly(bonding_curve_v2, false)); // remainingAccounts: @pump-fun/pump-sdk 要求末尾传 bondingCurveV2Pda(mint)，勿删
        // Apr 2026: extra protocol fee recipient after bonding-curve-v2 (writable)
        accounts.push(AccountMeta::new(get_protocol_extra_fee_recipient_random(), false));

        instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, &buy_data, accounts));

        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpFunParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;

        let token_amount = if let Some(amount) = params.input_amount {
            if amount == 0 {
                return Err(anyhow!("Amount cannot be zero"));
            }
            amount
        } else {
            return Err(anyhow!("Amount token is required"));
        };

        let bonding_curve = &protocol_params.bonding_curve;
        let creator = bonding_curve.creator;
        let creator_vault_pda = resolve_creator_vault_for_ix(
            &creator,
            protocol_params.creator_vault,
            &params.input_mint,
        )
        .ok_or_else(|| {
            anyhow!(
                "creator_vault PDA derivation failed (creator={})",
                creator
            )
        })?;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let sol_amount = get_sell_sol_amount_from_token_amount(
            bonding_curve.virtual_token_reserves as u128,
            bonding_curve.virtual_sol_reserves as u128,
            creator,
            token_amount,
        );

        let min_sol_output = match params.fixed_output_amount {
            Some(fixed) => fixed,
            None => calculate_with_slippage_sell(
                sol_amount,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            ),
        };

        let bonding_curve_addr = get_bonding_curve_pda(&params.input_mint).ok_or_else(|| {
            anyhow!("bonding_curve PDA derivation failed for mint {}", params.input_mint)
        })?;

        // Determine token program based on mayhem mode
        let is_mayhem_mode = bonding_curve.is_mayhem_mode;
        let token_program = protocol_params.token_program;
        let token_program_meta = if protocol_params.token_program == TOKEN_PROGRAM_2022 {
            crate::constants::TOKEN_PROGRAM_2022_META
        } else {
            crate::constants::TOKEN_PROGRAM_META
        };

        let associated_bonding_curve =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                &bonding_curve_addr,
                &params.input_mint,
                &token_program,
            );

        let user_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.input_mint,
                &token_program,
                params.open_seed_optimize,
            );

        // ========================================
        // V2 sell instruction: unified 26-account layout (quote_mint-aware)
        // ========================================
        let mut instructions = Vec::with_capacity(2);

        if protocol_params.use_v2_ix {
            let quote_mint = if protocol_params.quote_mint != Pubkey::default() {
                protocol_params.quote_mint
            } else {
                crate::constants::WSOL_TOKEN_ACCOUNT
            };
            let quote_token_program = get_quote_token_program(&quote_mint);
            let quote_token_program_meta = if quote_token_program == crate::constants::TOKEN_PROGRAM {
                crate::constants::TOKEN_PROGRAM_META
            } else {
                crate::constants::TOKEN_PROGRAM_2022_META
            };
            let sell_fee_recipient_meta =
                pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);

            let associated_quote_bonding_curve =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &bonding_curve_addr,
                    &quote_mint,
                    &quote_token_program,
                );
            let associated_quote_user =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                    &params.payer.pubkey(),
                    &quote_mint,
                    &quote_token_program,
                    params.open_seed_optimize,
                );
            let fee_recipient_pk = sell_fee_recipient_meta.pubkey;
            let associated_quote_fee_recipient =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &fee_recipient_pk,
                    &quote_mint,
                    &quote_token_program,
                );
            let buyback_fee_recipient = get_buyback_fee_recipient_random();
            let associated_quote_buyback_fee_recipient =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &buyback_fee_recipient,
                    &quote_mint,
                    &quote_token_program,
                );
            let associated_creator_vault =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &creator_vault_pda,
                    &quote_mint,
                    &quote_token_program,
                );
            let user_volume_accumulator = get_user_volume_accumulator_pda(&params.payer.pubkey())
                .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
            let associated_user_volume_accumulator =
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &user_volume_accumulator,
                    &quote_mint,
                    &quote_token_program,
                );
            let sharing_config =
                crate::instruction::utils::pumpfun::get_fee_sharing_config_pda(&params.input_mint)
                    .ok_or_else(|| anyhow!("sharing_config PDA derivation failed"))?;

            let mut v2_sell_data = [0u8; 24];
            v2_sell_data[..8].copy_from_slice(&SELL_V2_DISCRIMINATOR);
            v2_sell_data[8..16].copy_from_slice(&token_amount.to_le_bytes());
            v2_sell_data[16..24].copy_from_slice(&min_sol_output.to_le_bytes());

            let v2_sell_accounts: Vec<AccountMeta> = vec![
                global_constants::GLOBAL_ACCOUNT_META,                 // 1: global
                AccountMeta::new_readonly(params.input_mint, false),   // 2: base_mint
                AccountMeta::new_readonly(quote_mint, false),          // 3: quote_mint
                token_program_meta,                                    // 4: base_token_program
                quote_token_program_meta,                              // 5: quote_token_program
                crate::constants::ASSOCIATED_TOKEN_PROGRAM_META,       // 6: associated_token_program
                sell_fee_recipient_meta,                               // 7: fee_recipient
                AccountMeta::new(associated_quote_fee_recipient, false), // 8: associated_quote_fee_recipient
                AccountMeta::new_readonly(buyback_fee_recipient, false), // 9: buyback_fee_recipient
                AccountMeta::new(associated_quote_buyback_fee_recipient, false), // 10: associated_quote_buyback_fee_recipient
                AccountMeta::new(bonding_curve_addr, false),           // 11: bonding_curve
                AccountMeta::new(associated_bonding_curve, false),     // 12: associated_base_bonding_curve
                AccountMeta::new(associated_quote_bonding_curve, false), // 13: associated_quote_bonding_curve
                AccountMeta::new(params.payer.pubkey(), true),         // 14: user
                AccountMeta::new(user_token_account, false),           // 15: associated_base_user
                AccountMeta::new(associated_quote_user, false),        // 16: associated_quote_user
                AccountMeta::new(creator_vault_pda, false),            // 17: creator_vault
                AccountMeta::new(associated_creator_vault, false),     // 18: associated_creator_vault
                AccountMeta::new_readonly(sharing_config, false),      // 19: sharing_config
                AccountMeta::new(user_volume_accumulator, false),      // 20: user_volume_accumulator
                AccountMeta::new(associated_user_volume_accumulator, false), // 21: associated_user_volume_accumulator
                accounts::FEE_CONFIG_META,                             // 22: fee_config
                accounts::FEE_PROGRAM_META,                            // 23: fee_program
                crate::constants::SYSTEM_PROGRAM_META,                 // 24: system_program
                accounts::EVENT_AUTHORITY_META,                        // 25: event_authority
                accounts::PUMPFUN_META,                                // 26: program
            ];

            instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, &v2_sell_data, v2_sell_accounts));

            if protocol_params.close_token_account_when_sell.unwrap_or(false)
                || params.close_input_mint_ata
            {
                instructions.push(close_account(
                    &token_program,
                    &user_token_account,
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &[&params.payer.pubkey()],
                )?);
            }

            return Ok(instructions);
        }

        // ========================================
        // Legacy sell instructions (backward compatible)
        // ========================================

        let mut sell_data = [0u8; 24];
        sell_data[..8].copy_from_slice(&SELL_DISCRIMINATOR);
        sell_data[8..16].copy_from_slice(&token_amount.to_le_bytes());
        sell_data[16..24].copy_from_slice(&min_sol_output.to_le_bytes());

        let fee_recipient_meta =
            pump_fun_fee_recipient_meta(protocol_params.fee_recipient, is_mayhem_mode);

        let mut accounts: Vec<AccountMeta> = vec![
            global_constants::GLOBAL_ACCOUNT_META,
            fee_recipient_meta,
            AccountMeta::new_readonly(params.input_mint, false),
            AccountMeta::new(bonding_curve_addr, false),
            AccountMeta::new(associated_bonding_curve, false),
            AccountMeta::new(user_token_account, false),
            AccountMeta::new(params.payer.pubkey(), true),
            crate::constants::SYSTEM_PROGRAM_META,
            AccountMeta::new(creator_vault_pda, false),
            token_program_meta,
            accounts::EVENT_AUTHORITY_META,
            accounts::PUMPFUN_META,
            accounts::FEE_CONFIG_META,
            accounts::FEE_PROGRAM_META,
        ];

        // Cashback: Bonding Curve Sell expects UserVolumeAccumulator PDA at 0th remaining account (writable)
        if bonding_curve.is_cashback_coin {
            let user_volume_accumulator =
                get_user_volume_accumulator_pda(&params.payer.pubkey())
                    .ok_or_else(|| anyhow!("user_volume_accumulator PDA derivation failed"))?;
            accounts.push(AccountMeta::new(user_volume_accumulator, false));
        }
        // remainingAccounts: @pump-fun/pump-sdk sell 要求末尾传 bondingCurveV2Pda(mint)（cashback 时在 user_volume_accumulator 之后），勿删
        let bonding_curve_v2 = get_bonding_curve_v2_pda(&params.input_mint).ok_or_else(|| {
            anyhow!("bonding_curve_v2 PDA derivation failed for mint {}", params.input_mint)
        })?;
        accounts.push(AccountMeta::new_readonly(bonding_curve_v2, false));
        accounts.push(AccountMeta::new(get_protocol_extra_fee_recipient_random(), false));

        instructions.push(Instruction::new_with_bytes(accounts::PUMPFUN, &sell_data, accounts));

        // Optional: Close token account
        if protocol_params.close_token_account_when_sell.unwrap_or(false)
            || params.close_input_mint_ata
        {
            instructions.push(close_account(
                &token_program,
                &user_token_account,
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &[&params.payer.pubkey()],
            )?);
        }

        Ok(instructions)
    }
}

/// Claim cashback for Bonding Curve (Pump program). Transfers native lamports from UserVolumeAccumulator to user.
pub fn claim_cashback_pumpfun_instruction(payer: &Pubkey) -> Option<Instruction> {
    const CLAIM_CASHBACK_DISCRIMINATOR: [u8; 8] = [37, 58, 35, 126, 190, 53, 228, 197];
    let user_volume_accumulator = get_user_volume_accumulator_pda(payer)?;
    let accounts = vec![
        AccountMeta::new(*payer, true), // user (signer, writable)
        AccountMeta::new(user_volume_accumulator, false), // user_volume_accumulator (writable, not signer)
        crate::constants::SYSTEM_PROGRAM_META,
        accounts::EVENT_AUTHORITY_META,
        accounts::PUMPFUN_META,
    ];
    Some(Instruction::new_with_bytes(accounts::PUMPFUN, &CLAIM_CASHBACK_DISCRIMINATOR, accounts))
}
