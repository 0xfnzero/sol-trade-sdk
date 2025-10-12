use crate::{
    instruction::utils::meteora_damm_v2::{accounts, get_event_authority_pda, SWAP_DISCRIMINATOR},
    trading::core::{
        params::{MeteoraDammV2Params, SwapParams},
        traits::InstructionBuilder,
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct MeteoraDammV2InstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for MeteoraDammV2InstructionBuilder {
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
            .downcast_ref::<MeteoraDammV2Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        let is_wsol = protocol_params.token_a_mint == crate::constants::WSOL_TOKEN_ACCOUNT || protocol_params.token_b_mint == crate::constants::WSOL_TOKEN_ACCOUNT;
        let is_usdc = protocol_params.token_a_mint == crate::constants::USDC_TOKEN_ACCOUNT || protocol_params.token_b_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_a_in = protocol_params.token_a_mint == crate::constants::WSOL_TOKEN_ACCOUNT || protocol_params.token_a_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let minimum_amount_out: u64 = match params.fixed_output_amount {
            Some(fixed) => fixed,
            None => return Err(anyhow!("fixed_output_amount must be set for MeteoraDammV2 swap")),
        };

        let input_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.input_mint,
                if is_a_in {
                    &protocol_params.token_a_program
                } else {
                    &protocol_params.token_b_program
                },
                params.open_seed_optimize,
            );
        let output_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.output_mint,
                if is_a_in {
                    &protocol_params.token_b_program
                } else {
                    &protocol_params.token_a_program
                },
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
                    &crate::constants::TOKEN_PROGRAM,
                    params.open_seed_optimize,
                ),
            );
        }

        // Create buy instruction
        let accounts: [AccountMeta; 14] = [
            accounts::AUTHORITY_META,                      // Pool Authority (readonly)
            AccountMeta::new(protocol_params.pool, false), // Pool
            AccountMeta::new(input_token_account, false),  // Input Token Account
            AccountMeta::new(output_token_account, false), // Output Token Account
            AccountMeta::new(protocol_params.token_a_vault, false), // Token A Vault
            AccountMeta::new(protocol_params.token_b_vault, false), // Token B Vault
            AccountMeta::new_readonly(protocol_params.token_a_mint, false), // Token A Mint (readonly)
            AccountMeta::new_readonly(protocol_params.token_b_mint, false), // Token B Mint (readonly)
            AccountMeta::new(params.payer.pubkey(), true), // User Transfer Authority
            AccountMeta::new_readonly(protocol_params.token_a_program, false), // Token Program (readonly)
            AccountMeta::new_readonly(protocol_params.token_b_program, false), // Token Program (readonly)
            accounts::METEORA_DAMM_V2_META, // Referral Token Account (readonly)
            AccountMeta::new_readonly(get_event_authority_pda(), false), // Event Authority (readonly)
            accounts::METEORA_DAMM_V2_META,                              // Program (readonly)
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&SWAP_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::METEORA_DAMM_V2,
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
            .downcast_ref::<MeteoraDammV2Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        if params.input_amount.is_none() || params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Token amount is not set"));
        }

        let is_wsol = protocol_params.token_b_mint == crate::constants::WSOL_TOKEN_ACCOUNT || protocol_params.token_a_mint == crate::constants::WSOL_TOKEN_ACCOUNT;
        let is_usdc = protocol_params.token_b_mint == crate::constants::USDC_TOKEN_ACCOUNT || protocol_params.token_a_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_a_in = protocol_params.token_b_mint == crate::constants::WSOL_TOKEN_ACCOUNT || protocol_params.token_b_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        let minimum_amount_out: u64 = match params.fixed_output_amount {
            Some(fixed) => fixed,
            None => return Err(anyhow!("fixed_output_amount must be set for MeteoraDammV2 swap")),
        };

        let input_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.input_mint,
                if is_a_in {
                    &protocol_params.token_a_program
                } else {
                    &protocol_params.token_b_program
                },
                params.open_seed_optimize,
            );
        let output_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.output_mint,
                if is_a_in {
                    &protocol_params.token_b_program
                } else {
                    &protocol_params.token_a_program
                },
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
        let accounts: [AccountMeta; 14] = [
            accounts::AUTHORITY_META,                      // Pool Authority (readonly)
            AccountMeta::new(protocol_params.pool, false), // Pool
            AccountMeta::new(input_token_account, false),  // Input Token Account
            AccountMeta::new(output_token_account, false), // Output Token Account
            AccountMeta::new(protocol_params.token_a_vault, false), // Token A Vault
            AccountMeta::new(protocol_params.token_b_vault, false), // Token B Vault
            AccountMeta::new_readonly(protocol_params.token_a_mint, false), // Token A Mint (readonly)
            AccountMeta::new_readonly(protocol_params.token_b_mint, false), // Token B Mint (readonly)
            AccountMeta::new(params.payer.pubkey(), true), // User Transfer Authority
            AccountMeta::new_readonly(protocol_params.token_a_program, false), // Token Program (readonly)
            AccountMeta::new_readonly(protocol_params.token_b_program, false), // Token Program (readonly)
            accounts::METEORA_DAMM_V2_META, // Referral Token Account (readonly)
            AccountMeta::new_readonly(get_event_authority_pda(), false), // Event Authority (readonly)
            accounts::METEORA_DAMM_V2_META,                              // Program (readonly)
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&SWAP_DISCRIMINATOR);
        data[8..16].copy_from_slice(&params.input_amount.unwrap_or_default().to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::METEORA_DAMM_V2,
            &data,
            accounts.to_vec(),
        ));

        if params.close_output_mint_ata {
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }

        Ok(instructions)
    }
}
