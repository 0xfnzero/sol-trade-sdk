use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::utils::raydium_amm_v4::{accounts, SWAP_BASE_IN_DISCRIMINATOR},
    trading::core::{
        params::{RaydiumAmmV4Params, SwapParams},
        traits::InstructionBuilder,
    },
    utils::calc::raydium_amm_v4::compute_swap_amount,
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct RaydiumAmmV4InstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumAmmV4InstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumAmmV4Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let minimum_amount_out = params.fixed_output_amount.unwrap_or(0);

        let user_source_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.input_mint,
                &params.input_token_program,
                params.open_seed_optimize,
            );
        let user_destination_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.output_mint,
                &params.output_token_program,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if params.create_input_mint_ata {
            instructions
                .extend(crate::trading::common::handle_wsol(&params.payer.pubkey(), amount_in));
        }

        if params.create_output_mint_ata {
            instructions.extend(
                crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &params.output_mint,
                    &params.output_token_program,
                    params.open_seed_optimize,
                ),
            );
        }

        // Create buy instruction
        let accounts: [AccountMeta; 17] = [
            crate::constants::TOKEN_PROGRAM_META, // Token Program (readonly)
            AccountMeta::new(protocol_params.amm, false), // Amm
            accounts::AUTHORITY_META,             // Authority (readonly)
            AccountMeta::new(protocol_params.amm, false), // Amm Open Orders
            AccountMeta::new(protocol_params.base_vault, false), // Pool Coin Token Account
            AccountMeta::new(protocol_params.quote_vault, false), // Pool Pc Token Account
            AccountMeta::new(protocol_params.amm, false), // Serum Program
            AccountMeta::new(protocol_params.amm, false), // Serum Market
            AccountMeta::new(protocol_params.amm, false), // Serum Bids
            AccountMeta::new(protocol_params.amm, false), // Serum Asks
            AccountMeta::new(protocol_params.amm, false), // Serum Event Queue
            AccountMeta::new(protocol_params.amm, false), // Serum Coin Vault Account
            AccountMeta::new(protocol_params.amm, false), // Serum Pc Vault Account
            AccountMeta::new(protocol_params.amm, false), // Serum Vault Signer
            AccountMeta::new(user_source_token_account, false), // User Source Token Account
            AccountMeta::new(user_destination_token_account, false), // User Destination Token Account
            AccountMeta::new(params.payer.pubkey(), true),           // User Source Owner
        ];
        // Create instruction data
        let mut data = [0u8; 17];
        data[..1].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data[1..9].copy_from_slice(&amount_in.to_le_bytes());
        data[9..17].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_AMM_V4,
            &data,
            accounts.to_vec(),
        ));

        if params.close_input_mint_ata {
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
            .downcast_ref::<RaydiumAmmV4Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        if params.input_amount.is_none() || params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Token amount is not set"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in = params.input_amount.unwrap_or(0);
        let minimum_amount_out = params.fixed_output_amount.unwrap_or(0);

        let user_source_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.input_mint,
                &params.input_token_program,
                params.open_seed_optimize,
            );
        let user_destination_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.output_mint,
                &params.output_token_program,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(3);

        if params.create_output_mint_ata {
            instructions.extend(crate::trading::common::create_wsol_ata(&params.payer.pubkey()));
        }

        // Create buy instruction
        let accounts: [AccountMeta; 17] = [
            crate::constants::TOKEN_PROGRAM_META, // Token Program (readonly)
            AccountMeta::new(protocol_params.amm, false), // Amm
            accounts::AUTHORITY_META,             // Authority (readonly)
            AccountMeta::new(protocol_params.amm, false), // Amm Open Orders
            AccountMeta::new(protocol_params.base_vault, false), // Pool Coin Token Account
            AccountMeta::new(protocol_params.quote_vault, false), // Pool Pc Token Account
            AccountMeta::new(protocol_params.amm, false), // Serum Program
            AccountMeta::new(protocol_params.amm, false), // Serum Market
            AccountMeta::new(protocol_params.amm, false), // Serum Bids
            AccountMeta::new(protocol_params.amm, false), // Serum Asks
            AccountMeta::new(protocol_params.amm, false), // Serum Event Queue
            AccountMeta::new(protocol_params.amm, false), // Serum Coin Vault Account
            AccountMeta::new(protocol_params.amm, false), // Serum Pc Vault Account
            AccountMeta::new(protocol_params.amm, false), // Serum Vault Signer
            AccountMeta::new(user_source_token_account, false), // User Source Token Account
            AccountMeta::new(user_destination_token_account, false), // User Destination Token Account
            AccountMeta::new(params.payer.pubkey(), true),           // User Source Owner
        ];
        // Create instruction data
        let mut data = [0u8; 17];
        data[..1].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data[1..9].copy_from_slice(&params.input_amount.unwrap_or(0).to_le_bytes());
        data[9..17].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_AMM_V4,
            &data,
            accounts.to_vec(),
        ));

        if params.close_output_mint_ata {
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                &crate::constants::TOKEN_PROGRAM,
                &user_source_token_account,
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &[&params.payer.pubkey()],
            )?);
        }

        Ok(instructions)
    }
}
