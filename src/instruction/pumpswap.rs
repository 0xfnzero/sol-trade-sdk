use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::utils::pumpswap::{
        accounts, fee_recipient_ata, get_user_volume_accumulator_pda, BUY_DISCRIMINATOR,
        SELL_DISCRIMINATOR,
    },
    trading::{
        common::wsol_manager,
        core::{
            params::{PumpSwapParams, SwapParams},
            traits::InstructionBuilder,
        },
    },
    utils::calc::pumpswap::{buy_quote_input_internal, sell_base_input_internal},
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for PumpSwap protocol
pub struct PumpSwapInstructionBuilder;

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

        let pool = protocol_params.pool;
        let base_mint = protocol_params.base_mint;
        let quote_mint = protocol_params.quote_mint;
        let params_coin_creator_vault_ata = protocol_params.coin_creator_vault_ata;
        let params_coin_creator_vault_authority = protocol_params.coin_creator_vault_authority;
        let create_wsol_ata = params.create_input_mint_ata;
        let close_wsol_ata = params.close_input_mint_ata;
        let base_token_program = protocol_params.base_token_program;
        let quote_token_program = protocol_params.quote_token_program;
        let pool_base_token_account = protocol_params.pool_base_token_account;
        let pool_quote_token_account = protocol_params.pool_quote_token_account;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let mut creator = Pubkey::default();
        if params_coin_creator_vault_authority != accounts::DEFAULT_COIN_CREATOR_VAULT_AUTHORITY {
            creator = params_coin_creator_vault_authority;
        }

        let output_amount = params.fixed_output_amount.unwrap_or(0);

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

        // Determine fee recipient based on mayhem mode
        let is_mayhem_mode = protocol_params.is_mayhem_mode;
        let fee_recipient =
            if is_mayhem_mode { accounts::MAYHEM_FEE_RECIPIENT } else { accounts::FEE_RECIPIENT };
        let fee_recipient_meta = if is_mayhem_mode {
            accounts::MAYHEM_FEE_RECIPIENT_META
        } else {
            accounts::FEE_RECIPIENT_META
        };
        let fee_recipient_ata = fee_recipient_ata(fee_recipient, quote_mint);

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if create_wsol_ata {
            instructions
                .extend(crate::trading::common::handle_wsol(&params.payer.pubkey(), params.input_amount.unwrap()));
        }

        if params.create_output_mint_ata {
            instructions.extend(
                crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &base_mint,
                    &base_token_program,
                    params.open_seed_optimize,
                ),
            );
        }

        // Create buy instruction
        let mut accounts = Vec::with_capacity(23);
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
            accounts::GLOBAL_VOLUME_ACCUMULATOR_META,
            AccountMeta::new(
                get_user_volume_accumulator_pda(&params.payer.pubkey()).unwrap(),
                false,
            ),
            accounts::FEE_CONFIG_META,
            accounts::FEE_PROGRAM_META,
        ]);

        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&BUY_DISCRIMINATOR);
        // base_amount_out
        data[8..16].copy_from_slice(&output_amount.to_le_bytes());
        // max_quote_amount_in
        data[16..24].copy_from_slice(&params.input_amount.unwrap().to_le_bytes());

        instructions.push(Instruction {
            program_id: accounts::AMM_PROGRAM,
            accounts,
            data: data.to_vec(),
        });
        if close_wsol_ata {
            // Close wSOL ATA account, reclaim rent
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
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

        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        let pool = protocol_params.pool;
        let base_mint = protocol_params.base_mint;
        let quote_mint = protocol_params.quote_mint;
        let pool_base_token_account = protocol_params.pool_base_token_account;
        let pool_quote_token_account = protocol_params.pool_quote_token_account;
        let params_coin_creator_vault_ata = protocol_params.coin_creator_vault_ata;
        let params_coin_creator_vault_authority = protocol_params.coin_creator_vault_authority;
        let create_wsol_ata = params.create_output_mint_ata;
        let close_wsol_ata = params.close_output_mint_ata;
        let base_token_program = protocol_params.base_token_program;
        let quote_token_program = protocol_params.quote_token_program;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let mut creator = Pubkey::default();
        if params_coin_creator_vault_authority != accounts::DEFAULT_COIN_CREATOR_VAULT_AUTHORITY {
            creator = params_coin_creator_vault_authority;
        }

        let output_amount = params.fixed_output_amount.unwrap_or(0);

        // Determine fee recipient based on mayhem mode
        let is_mayhem_mode = protocol_params.is_mayhem_mode;
        let fee_recipient =
            if is_mayhem_mode { accounts::MAYHEM_FEE_RECIPIENT } else { accounts::FEE_RECIPIENT };
        let fee_recipient_meta = if is_mayhem_mode {
            accounts::MAYHEM_FEE_RECIPIENT_META
        } else {
            accounts::FEE_RECIPIENT_META
        };
        let fee_recipient_ata = fee_recipient_ata(fee_recipient, quote_mint);

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
        let mut instructions = Vec::with_capacity(3);

        if create_wsol_ata {
            instructions.extend(wsol_manager::create_wsol_ata(&params.payer.pubkey()));
        }

        // Create sell instruction
        let mut accounts = Vec::with_capacity(23);
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
            accounts::FEE_CONFIG_META,
            accounts::FEE_PROGRAM_META,
        ]);

        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&SELL_DISCRIMINATOR);
        // base_amount_in
        data[8..16].copy_from_slice(&params.input_amount.unwrap().to_le_bytes());
        // min_quote_amount_out
        data[16..24].copy_from_slice(&output_amount.to_le_bytes());

        instructions.push(Instruction {
            program_id: accounts::AMM_PROGRAM,
            accounts,
            data: data.to_vec(),
        });

        if close_wsol_ata {
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                &quote_token_program,
                &user_quote_token_account,
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &[&params.payer.pubkey()],
            )?);
        }
        Ok(instructions)
    }
}
