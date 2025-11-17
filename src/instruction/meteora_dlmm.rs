use anyhow::anyhow;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_sdk::signature::Signer;
use solana_streamer::streaming::event_parser::protocols::meteora_dlmm::parser::METEORA_DLMM_PROGRAM_ID;
use crate::common::AnyResult;
use crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed;
use crate::constants::TOKEN_PROGRAM;
use crate::instruction::utils::meteora_dlmm::meteora_dlmm::accounts::EVENT_AUTHORITY_META;
use crate::instruction::utils::meteora_dlmm::meteora_dlmm::SWAP_DISCRIMINATOR;
use crate::trading::{InstructionBuilder, SwapParams};
use crate::trading::core::params::{MeteoraDlmmParams};

pub struct MeteoraDlmmInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for MeteoraDlmmInstructionBuilder {
    async fn build_buy_instructions(&self, params: &SwapParams) -> AnyResult<Vec<Instruction>> {
        if params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        };

        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<MeteoraDlmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for MeteoraDlmm"))?;

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
            AccountMeta::new(protocol_params.lb_pair, false),
            AccountMeta::new(METEORA_DLMM_PROGRAM_ID, false),
            AccountMeta::new(protocol_params.reserve_x, false),
            AccountMeta::new(protocol_params.reserve_y, false),
            AccountMeta::new(input_token_account, false),
            AccountMeta::new(output_token_account, false),
            AccountMeta::new_readonly(protocol_params.token_x_mint, false),
            AccountMeta::new_readonly(protocol_params.token_y_mint, false),
            AccountMeta::new(protocol_params.oracle, false),
            AccountMeta::new(METEORA_DLMM_PROGRAM_ID, false),
            AccountMeta::new(params.payer.pubkey(), true),
            AccountMeta::new_readonly(protocol_params.token_x_program, false),
            AccountMeta::new_readonly(protocol_params.token_y_program, false),
            EVENT_AUTHORITY_META,
            AccountMeta::new_readonly(METEORA_DLMM_PROGRAM_ID, false),
        ];

        // 追加bin_array
        for bin_array in protocol_params.bin_array.iter() {
            accounts.push(AccountMeta::new(*bin_array, false))
        }

        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&SWAP_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            METEORA_DLMM_PROGRAM_ID,
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
        todo!()
    }
}
