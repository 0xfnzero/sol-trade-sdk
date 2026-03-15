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
        accounts, get_bonding_curve_pda, get_bonding_curve_v2_pda, get_creator,
        get_mayhem_fee_recipient_meta_random, get_user_volume_accumulator_pda,
        global_constants::{self},
        BUY_DISCRIMINATOR, BUY_EXACT_SOL_IN_DISCRIMINATOR, SELL_DISCRIMINATOR,
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
        let creator_vault_pda = protocol_params.creator_vault;
        let creator = get_creator(&creator_vault_pda);

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

        let bonding_curve_addr = if bonding_curve.account == Pubkey::default() {
            get_bonding_curve_pda(&params.output_mint).ok_or_else(|| {
                anyhow!("bonding_curve PDA derivation failed for mint {}", params.output_mint)
            })?
        } else {
            bonding_curve.account
        };

        // Determine token program based on mayhem mode
        let is_mayhem_mode = bonding_curve.is_mayhem_mode;
        let token_program = protocol_params.token_program;
        let token_program_meta = if protocol_params.token_program == TOKEN_PROGRAM_2022 {
            crate::constants::TOKEN_PROGRAM_2022_META
        } else {
            crate::constants::TOKEN_PROGRAM_META
        };

        let associated_bonding_curve =
            if protocol_params.associated_bonding_curve == Pubkey::default() {
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &bonding_curve_addr,
                    &params.output_mint,
                    &token_program,
                )
            } else {
                protocol_params.associated_bonding_curve
            };

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

        // Determine fee recipient based on mayhem mode (pump-public-docs: 2nd account = Mayhem fee recipient; use any one randomly)
        let fee_recipient_meta = if is_mayhem_mode {
            get_mayhem_fee_recipient_meta_random()
        } else {
            global_constants::FEE_RECIPIENT_META
        };

        let bonding_curve_v2 = get_bonding_curve_v2_pda(&params.output_mint).ok_or_else(|| {
            anyhow!("bonding_curve_v2 PDA derivation failed for mint {}", params.output_mint)
        })?;
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
        let creator_vault_pda = protocol_params.creator_vault;
        let creator = get_creator(&creator_vault_pda);

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

        let bonding_curve_addr = if bonding_curve.account == Pubkey::default() {
            get_bonding_curve_pda(&params.input_mint).ok_or_else(|| {
                anyhow!("bonding_curve PDA derivation failed for mint {}", params.input_mint)
            })?
        } else {
            bonding_curve.account
        };

        // Determine token program based on mayhem mode
        let is_mayhem_mode = bonding_curve.is_mayhem_mode;
        let token_program = protocol_params.token_program;
        let token_program_meta = if protocol_params.token_program == TOKEN_PROGRAM_2022 {
            crate::constants::TOKEN_PROGRAM_2022_META
        } else {
            crate::constants::TOKEN_PROGRAM_META
        };

        let associated_bonding_curve =
            if protocol_params.associated_bonding_curve == Pubkey::default() {
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &bonding_curve_addr,
                    &params.input_mint,
                    &token_program,
                )
            } else {
                protocol_params.associated_bonding_curve
            };

        let user_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.input_mint,
                &token_program,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(2);

        let mut sell_data = [0u8; 24];
        sell_data[..8].copy_from_slice(&SELL_DISCRIMINATOR);
        sell_data[8..16].copy_from_slice(&token_amount.to_le_bytes());
        sell_data[16..24].copy_from_slice(&min_sol_output.to_le_bytes());

        // Determine fee recipient based on mayhem mode (pump-public-docs: 2nd account = Mayhem fee recipient; use any one randomly)
        let fee_recipient_meta = if is_mayhem_mode {
            get_mayhem_fee_recipient_meta_random()
        } else {
            global_constants::FEE_RECIPIENT_META
        };

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
