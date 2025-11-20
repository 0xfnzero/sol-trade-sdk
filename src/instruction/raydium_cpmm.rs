use crate::{
    common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed,
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::utils::raydium_cpmm::{
        accounts, get_observation_state_pda, get_pool_pda, get_vault_account,
        SWAP_BASE_IN_DISCRIMINATOR,
    },
    trading::core::{
        params::{RaydiumCpmmParams, SwapParams},
        traits::InstructionBuilder,
    },
    utils::calc::raydium_cpmm::compute_swap_amount,
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct RaydiumCpmmInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumCpmmInstructionBuilder {
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
            .downcast_ref::<RaydiumCpmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        let pool = protocol_params.pool;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let minimum_amount_out = params.fixed_output_amount.unwrap_or(0);

        let input_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.input_mint,
            &params.input_token_program,
            params.open_seed_optimize,
        );
        let output_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.output_mint,
            &params.output_token_program,
            params.open_seed_optimize,
        );

        let input_vault_account = get_vault_account(&pool, &params.input_mint, protocol_params);
        let output_vault_account = get_vault_account(&pool, &params.output_mint, protocol_params);

        let observation_state_account = if protocol_params.observation_state == Pubkey::default() {
            get_observation_state_pda(&pool).unwrap()
        } else {
            protocol_params.observation_state
        };

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
        let accounts: [AccountMeta; 13] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            AccountMeta::new(protocol_params.amm_config, false), // Amm Config (readonly)
            AccountMeta::new(pool, false),           // Pool State
            AccountMeta::new(input_token_account, false),   // Input Token Account
            AccountMeta::new(output_token_account, false),   // Output Token Account
            AccountMeta::new(input_vault_account, false),   // Input Vault Account
            AccountMeta::new(output_vault_account, false),   // Output Vault Account
            AccountMeta::new_readonly(params.input_token_program, false), // Input Token Program (readonly)
            AccountMeta::new_readonly(params.output_token_program, false), // Output Token Program (readonly)
            AccountMeta::new_readonly(params.input_mint, false),    // Input token mint (readonly)
            AccountMeta::new_readonly(params.output_mint, false), // Output token mint (readonly)
            AccountMeta::new(observation_state_account, false), // Observation State Account
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_CPMM,
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
        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumCpmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        let pool = protocol_params.pool;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let minimum_amount_out = params.fixed_output_amount.unwrap_or(0);

        let input_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.input_mint,
            &params.input_token_program,
            params.open_seed_optimize,
        );
        let output_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.output_mint,
            &params.output_token_program,
            params.open_seed_optimize,
        );

        let input_vault_account = get_vault_account(&pool, &params.input_mint, protocol_params);
        let output_vault_account = get_vault_account(&pool, &params.output_mint, protocol_params);

        let observation_state_account = if protocol_params.observation_state == Pubkey::default() {
            get_observation_state_pda(&pool).unwrap()
        } else {
            protocol_params.observation_state
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(3);

        if params.create_output_mint_ata {
            instructions.extend(crate::trading::common::create_wsol_ata(&params.payer.pubkey()));
        }

        // Create sell instruction
        let accounts: [AccountMeta; 13] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            AccountMeta::new(protocol_params.amm_config, false), // Amm Config (readonly)
            AccountMeta::new(pool, false),           // Pool State
            AccountMeta::new(input_token_account, false),   // Input Token Account
            AccountMeta::new(output_token_account, false),   // Output Token Account
            AccountMeta::new(input_vault_account, false),   // Input Vault Account
            AccountMeta::new(output_vault_account, false),   // Output Vault Account
            AccountMeta::new_readonly(params.input_token_program, false), // Input Token Program (readonly)
            AccountMeta::new_readonly(params.output_token_program, false), // Output Token Program (readonly)
            AccountMeta::new_readonly(params.input_mint, false),    // Input token mint (readonly)
            AccountMeta::new_readonly(params.output_mint, false), // Output token mint (readonly)
            AccountMeta::new(observation_state_account, false), // Observation State Account
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_CPMM,
            &data,
            accounts.to_vec(),
        ));

        if params.close_output_mint_ata {
            // Close wSOL ATA account, reclaim rent
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                &params.input_token_program,
                &input_token_account,
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &[&params.payer.pubkey()],
            )?);
        }

        Ok(instructions)
    }
}
