use anyhow::anyhow;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::pubkey::Pubkey;
use solana_sdk::signature::Signer;
use solana_streamer::streaming::event_parser::protocols::orca::parser::ORCA_PROGRAM_ID;
use crate::common::{AnyResult, GasFeeStrategy};
use crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed;
use crate::constants::TOKEN_PROGRAM;
use crate::instruction::utils::orca::SWAP_DISCRIMINATOR;
use crate::trading::{InstructionBuilder, SwapParams};
use crate::trading::core::params::{MeteoraDlmmParams, OrcaParams};
use crate::trading::factory::DexType;

pub struct OrcaInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for OrcaInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> AnyResult<Vec<Instruction>> {
        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        };

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<OrcaParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for Orca"))?;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let minimum_amount_out: u64 = params.fixed_output_amount.unwrap_or(0);

        let input_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.input_mint,
            &params.input_token_program.unwrap_or(TOKEN_PROGRAM),
            params.open_seed_optimize,
        );
        let output_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.output_mint,
            &params.output_token_program.unwrap_or(TOKEN_PROGRAM),
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
                    &params.output_token_program.unwrap_or(TOKEN_PROGRAM),
                    params.open_seed_optimize,
                ),
            );
        }

        // Create buy instruction
        let mut accounts: Vec<AccountMeta> = vec![
            AccountMeta::new_readonly(TOKEN_PROGRAM, false),
            AccountMeta::new(params.payer.pubkey(), true),
            AccountMeta::new(protocol_params.pool, false),
            AccountMeta::new(input_token_account, false),
            AccountMeta::new(protocol_params.vault_a, false),
            AccountMeta::new(output_token_account, false),
            AccountMeta::new(protocol_params.vault_b, false),
            AccountMeta::new(protocol_params.tick_array0, false),
            AccountMeta::new(protocol_params.tick_array1, false),
            AccountMeta::new(protocol_params.tick_array2, false),
            AccountMeta::new(protocol_params.oracle, false),
        ];

        // Create instruction data
        let mut data = [0u8; 42];
        data[..8].copy_from_slice(&SWAP_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
        data[24..40].fill(0);
        data[40] = protocol_params.amount_specified_is_input as u8;
        data[41] = protocol_params.a_to_b as u8;

        instructions.push(Instruction::new_with_bytes(
            ORCA_PROGRAM_ID,
            &data,
            accounts.to_vec(),
        ));

        if params.close_input_mint_ata {
            // Close wSOL ATA account, reclaim rent
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }

        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &SwapParams) -> AnyResult<Vec<Instruction>> {
        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        };

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<OrcaParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for Orca"))?;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let minimum_amount_out: u64 = params.fixed_output_amount.unwrap_or(0);

        let token_owner_account_a = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &protocol_params.token_mint_a,
            &params.input_token_program.unwrap_or(TOKEN_PROGRAM),
            params.open_seed_optimize,
        );
        let token_owner_account_b = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &protocol_params.token_mint_b,
            &params.output_token_program.unwrap_or(TOKEN_PROGRAM),
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
                    &params.output_token_program.unwrap_or(TOKEN_PROGRAM),
                    params.open_seed_optimize,
                ),
            );
        }

        // Create buy instruction
        let mut accounts: Vec<AccountMeta> = vec![
            AccountMeta::new_readonly(TOKEN_PROGRAM, false),
            AccountMeta::new(params.payer.pubkey(), true),
            AccountMeta::new(protocol_params.pool, false),
            AccountMeta::new(token_owner_account_a, false),
            AccountMeta::new(protocol_params.vault_a, false),
            AccountMeta::new(token_owner_account_b, false),
            AccountMeta::new(protocol_params.vault_b, false),
            AccountMeta::new(protocol_params.tick_array0, false),
            AccountMeta::new(protocol_params.tick_array1, false),
            AccountMeta::new(protocol_params.tick_array2, false),
            AccountMeta::new(protocol_params.oracle, false),
        ];

        // Create instruction data
        let mut data = [0u8; 42];
        data[..8].copy_from_slice(&SWAP_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
        data[24..40].fill(0);
        data[40] = protocol_params.amount_specified_is_input as u8;
        data[41] = protocol_params.a_to_b as u8;

        instructions.push(Instruction::new_with_bytes(
            ORCA_PROGRAM_ID,
            &data,
            accounts.to_vec(),
        ));

        if params.close_input_mint_ata {
            // Close wSOL ATA account, reclaim rent
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }

        Ok(instructions)
    }
}