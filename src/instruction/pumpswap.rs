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
        let pool_base_token_reserves = protocol_params.pool_base_token_reserves;
        let pool_quote_token_reserves = protocol_params.pool_quote_token_reserves;
        let params_coin_creator_vault_ata = protocol_params.coin_creator_vault_ata;
        let params_coin_creator_vault_authority = protocol_params.coin_creator_vault_authority;
        let create_wsol_ata = params.create_input_mint_ata;
        let close_wsol_ata = params.close_input_mint_ata;
        let base_token_program = protocol_params.base_token_program;
        let quote_token_program = protocol_params.quote_token_program;
        let pool_base_token_account = protocol_params.pool_base_token_account;
        let pool_quote_token_account = protocol_params.pool_quote_token_account;

        let is_wsol = (base_mint == crate::constants::WSOL_TOKEN_ACCOUNT && quote_mint != crate::constants::USDC_TOKEN_ACCOUNT)
            || (quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT && base_mint != crate::constants::USDC_TOKEN_ACCOUNT);
        let is_usdc = (base_mint == crate::constants::USDC_TOKEN_ACCOUNT && quote_mint != crate::constants::WSOL_TOKEN_ACCOUNT)
            || (quote_mint == crate::constants::USDC_TOKEN_ACCOUNT && base_mint != crate::constants::WSOL_TOKEN_ACCOUNT);
        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let quote_is_wsol_or_usdc = quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT 
            || quote_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        let mut creator = Pubkey::default();
        if params_coin_creator_vault_authority != accounts::DEFAULT_COIN_CREATOR_VAULT_AUTHORITY {
            creator = params_coin_creator_vault_authority;
        }

        let (mut token_amount, sol_amount) = if quote_is_wsol_or_usdc {
            let result = buy_quote_input_internal(
                params.input_amount.unwrap_or(0),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                &creator,
            )
            .unwrap();
            // base_amount_out, max_quote_amount_in
            (result.base, result.max_quote)
        } else {
            let result = sell_base_input_internal(
                params.input_amount.unwrap_or(0),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                &creator,
            )
            .unwrap();
            // min_quote_amount_out, base_amount_in
            (result.min_quote, params.input_amount.unwrap_or(0))
        };

        if params.fixed_output_amount.is_some() {
            token_amount = params.fixed_output_amount.unwrap();
        }

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
        let fee_recipient_ata = fee_recipient_ata(accounts::FEE_RECIPIENT, quote_mint);

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if create_wsol_ata {
            instructions
                .extend(crate::trading::common::handle_wsol(&params.payer.pubkey(), sol_amount));
        }

        if params.create_output_mint_ata {
            instructions.extend(
                crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    if quote_is_wsol_or_usdc { &base_mint } else { &quote_mint },
                    if quote_is_wsol_or_usdc { &base_token_program } else { &quote_token_program },
                    params.open_seed_optimize,
                ),
            );
        }

        // Create buy instruction
        let mut accounts = Vec::with_capacity(23);
        accounts.extend([
            AccountMeta::new_readonly(pool, false), // pool_id (readonly)
            AccountMeta::new(params.payer.pubkey(), true), // user (signer)
            accounts::GLOBAL_ACCOUNT_META,          // global (readonly)
            AccountMeta::new_readonly(base_mint, false), // base_mint (readonly)
            AccountMeta::new_readonly(quote_mint, false), // quote_mint (readonly)
            AccountMeta::new(user_base_token_account, false), // user_base_token_account
            AccountMeta::new(user_quote_token_account, false), // user_quote_token_account
            AccountMeta::new(pool_base_token_account, false), // pool_base_token_account
            AccountMeta::new(pool_quote_token_account, false), // pool_quote_token_account
            accounts::FEE_RECIPIENT_META,           // fee_recipient (readonly)
            AccountMeta::new(fee_recipient_ata, false), // fee_recipient_ata
            AccountMeta::new_readonly(base_token_program, false), // TOKEN_PROGRAM_ID (readonly)
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
            accounts.push(AccountMeta::new(
                get_user_volume_accumulator_pda(&params.payer.pubkey()).unwrap(),
                false,
            ));
        }
        accounts.push(accounts::FEE_CONFIG_META);
        accounts.push(accounts::FEE_PROGRAM_META);

        // Create instruction data
        let mut data = [0u8; 24];
        if quote_is_wsol_or_usdc {
            data[..8].copy_from_slice(&BUY_DISCRIMINATOR);
            // base_amount_out
            data[8..16].copy_from_slice(&token_amount.to_le_bytes());
            // max_quote_amount_in
            data[16..24].copy_from_slice(&sol_amount.to_le_bytes());
        } else {
            data[..8].copy_from_slice(&SELL_DISCRIMINATOR);
            // base_amount_in
            data[8..16].copy_from_slice(&sol_amount.to_le_bytes());
            // min_quote_amount_out
            data[16..24].copy_from_slice(&token_amount.to_le_bytes());
        }

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

        let pool = protocol_params.pool;
        let base_mint = protocol_params.base_mint;
        let quote_mint = protocol_params.quote_mint;
        let pool_base_token_reserves = protocol_params.pool_base_token_reserves;
        let pool_quote_token_reserves = protocol_params.pool_quote_token_reserves;
        let pool_base_token_account = protocol_params.pool_base_token_account;
        let pool_quote_token_account = protocol_params.pool_quote_token_account;
        let params_coin_creator_vault_ata = protocol_params.coin_creator_vault_ata;
        let params_coin_creator_vault_authority = protocol_params.coin_creator_vault_authority;
        let create_wsol_ata = params.create_output_mint_ata;
        let close_wsol_ata = params.close_output_mint_ata;
        let base_token_program = protocol_params.base_token_program;
        let quote_token_program = protocol_params.quote_token_program;

        let is_wsol = (base_mint == crate::constants::WSOL_TOKEN_ACCOUNT && quote_mint != crate::constants::USDC_TOKEN_ACCOUNT)
            || (quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT && base_mint != crate::constants::USDC_TOKEN_ACCOUNT);
        let is_usdc = (base_mint == crate::constants::USDC_TOKEN_ACCOUNT && quote_mint != crate::constants::WSOL_TOKEN_ACCOUNT)
            || (quote_mint == crate::constants::USDC_TOKEN_ACCOUNT && base_mint != crate::constants::WSOL_TOKEN_ACCOUNT);
        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        if params.input_amount.is_none() {
            return Err(anyhow!("Token amount is not set"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let quote_is_wsol_or_usdc = quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT 
            || quote_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        let mut creator = Pubkey::default();
        if params_coin_creator_vault_authority != accounts::DEFAULT_COIN_CREATOR_VAULT_AUTHORITY {
            creator = params_coin_creator_vault_authority;
        }

        let (token_amount, mut sol_amount) = if quote_is_wsol_or_usdc {
            let result = sell_base_input_internal(
                params.input_amount.unwrap(),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                &creator,
            )
            .unwrap();
            // base_amount_in, min_quote_amount_out
            (params.input_amount.unwrap(), result.min_quote)
        } else {
            let result = buy_quote_input_internal(
                params.input_amount.unwrap(),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                &creator,
            )
            .unwrap();
            // max_quote_amount_in, base_amount_out
            (result.max_quote, result.base)
        };

        if params.fixed_output_amount.is_some() {
            sol_amount = params.fixed_output_amount.unwrap();
        }

        let fee_recipient_ata = fee_recipient_ata(accounts::FEE_RECIPIENT, quote_mint);
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
            AccountMeta::new_readonly(pool, false), // pool_id (readonly)
            AccountMeta::new(params.payer.pubkey(), true), // user (signer)
            accounts::GLOBAL_ACCOUNT_META,          // global (readonly)
            AccountMeta::new_readonly(base_mint, false), // mint (readonly)
            AccountMeta::new_readonly(quote_mint, false), // WSOL_TOKEN_ACCOUNT (readonly)
            AccountMeta::new(user_base_token_account, false), // user_base_token_account
            AccountMeta::new(user_quote_token_account, false), // user_quote_token_account
            AccountMeta::new(pool_base_token_account, false), // pool_base_token_account
            AccountMeta::new(pool_quote_token_account, false), // pool_quote_token_account
            accounts::FEE_RECIPIENT_META,           // fee_recipient (readonly)
            AccountMeta::new(fee_recipient_ata, false), // fee_recipient_ata
            AccountMeta::new_readonly(base_token_program, false), // TOKEN_PROGRAM_ID (readonly)
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
            accounts.push(AccountMeta::new(
                get_user_volume_accumulator_pda(&params.payer.pubkey()).unwrap(),
                false,
            ));
        }

        accounts.push(accounts::FEE_CONFIG_META);
        accounts.push(accounts::FEE_PROGRAM_META);

        // Create instruction data
        let mut data = [0u8; 24];
        if quote_is_wsol_or_usdc {
            data[..8].copy_from_slice(&SELL_DISCRIMINATOR);
            // base_amount_in
            data[8..16].copy_from_slice(&token_amount.to_le_bytes());
            // min_quote_amount_out
            data[16..24].copy_from_slice(&sol_amount.to_le_bytes());
        } else {
            data[..8].copy_from_slice(&BUY_DISCRIMINATOR);
            // base_amount_out
            data[8..16].copy_from_slice(&sol_amount.to_le_bytes());
            // max_quote_amount_in
            data[16..24].copy_from_slice(&token_amount.to_le_bytes());
        }

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
                if quote_is_wsol_or_usdc {
                    &base_token_program
                } else {
                    &quote_token_program
                },
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
