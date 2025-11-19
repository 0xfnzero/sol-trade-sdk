use anyhow::anyhow;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_sdk::signature::Signer;
use solana_streamer::streaming::event_parser::protocols::raydium_clmm::parser::RAYDIUM_CLMM_PROGRAM_ID;
use crate::common::{AnyResult, GasFeeStrategy};
use crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed;
use crate::constants::{MEMO_PROGRAM_META, TOKEN_PROGRAM, TOKEN_PROGRAM_2022_META, TOKEN_PROGRAM_META};
use crate::instruction::utils::raydium_clmm::SWAP_DISCRIMINATOR;
use crate::trading::{InstructionBuilder, SwapParams};
use crate::trading::core::params::RaydiumClmmParams;

pub struct RaydiumClmmInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumClmmInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> AnyResult<Vec<Instruction>> {
        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        };

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumClmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumClmm"))?;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let other_amount_threshold: u64 = params.fixed_output_amount.unwrap_or(0);

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
            AccountMeta::new(params.payer.pubkey(), true),
            AccountMeta::new_readonly(protocol_params.amm_config, false),
            AccountMeta::new(protocol_params.pool, false),
            AccountMeta::new(input_token_account, false),
            AccountMeta::new(output_token_account, false),
            AccountMeta::new(protocol_params.input_token_vault, false),
            AccountMeta::new(protocol_params.output_token_vault, false),
            AccountMeta::new(protocol_params.observation_key, false),
            TOKEN_PROGRAM_META,
            TOKEN_PROGRAM_2022_META,
            MEMO_PROGRAM_META,
            AccountMeta::new_readonly(params.input_mint, false),
            AccountMeta::new_readonly(params.output_mint, false),
        ];

        // 追加tick_array
        for tick_array in protocol_params.tick_arrays.iter() {
            accounts.push(AccountMeta::new(*tick_array, false))
        }

        // Create instruction data
        let mut data = [0u8; 41];
        data[..8].copy_from_slice(&SWAP_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&other_amount_threshold.to_le_bytes());
        data[24..40].fill(0);
        data[40] = protocol_params.is_base_input as u8;

        instructions.push(Instruction::new_with_bytes(
            RAYDIUM_CLMM_PROGRAM_ID,
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
            .downcast_ref::<RaydiumClmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumClmm"))?;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let other_amount_threshold: u64 = params.fixed_output_amount.unwrap_or(0);

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
            AccountMeta::new(params.payer.pubkey(), true),
            AccountMeta::new_readonly(protocol_params.amm_config, false),
            AccountMeta::new(protocol_params.pool, false),
            AccountMeta::new(input_token_account, false),
            AccountMeta::new(output_token_account, false),
            AccountMeta::new(protocol_params.input_token_vault, false),
            AccountMeta::new(protocol_params.output_token_vault, false),
            AccountMeta::new(protocol_params.observation_key, false),
            TOKEN_PROGRAM_META,
            TOKEN_PROGRAM_2022_META,
            MEMO_PROGRAM_META,
            AccountMeta::new_readonly(params.input_mint, false),
            AccountMeta::new_readonly(params.output_mint, false),
        ];

        // 追加tick_array
        for tick_array in protocol_params.tick_arrays.iter() {
            accounts.push(AccountMeta::new(*tick_array, false))
        }

        // Create instruction data
        let mut data = [0u8; 41];
        data[..8].copy_from_slice(&SWAP_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&other_amount_threshold.to_le_bytes());
        data[24..40].fill(0);
        data[40] = protocol_params.is_base_input as u8;

        instructions.push(Instruction::new_with_bytes(
            RAYDIUM_CLMM_PROGRAM_ID,
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