use crate::{
    common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed,
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::raydium_cpmm::{
            accounts, get_observation_state_pda, get_pool_pda, get_vault_account,
            SWAP_BASE_IN_DISCRIMINATOR, SWAP_BASE_OUT_DISCRIMINATOR,
        },
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

        let pool_state = if protocol_params.pool_state == Pubkey::default() {
            get_pool_pda(
                &protocol_params.amm_config,
                &protocol_params.base_mint,
                &protocol_params.quote_mint,
            )
            .unwrap()
        } else {
            protocol_params.pool_state
        };

        let is_wsol = protocol_params.base_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT;

        let is_usdc = protocol_params.base_mint == crate::constants::USDC_TOKEN_ACCOUNT
            || protocol_params.quote_mint == crate::constants::USDC_TOKEN_ACCOUNT;

        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_base_in = protocol_params.base_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.base_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        let input_mint =
            if is_base_in { protocol_params.base_mint } else { protocol_params.quote_mint };
        let input_token_program = if is_base_in {
            protocol_params.base_token_program
        } else {
            protocol_params.quote_token_program
        };
        let output_mint =
            if is_base_in { protocol_params.quote_mint } else { protocol_params.base_mint };
        let output_token_program = if is_base_in {
            protocol_params.quote_token_program
        } else {
            protocol_params.base_token_program
        };

        let amount_in: u64 = params.input_amount.unwrap_or(0);

        let input_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &input_mint,
            &input_token_program,
            params.open_seed_optimize,
        );
        let output_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &output_mint,
            &output_token_program,
            params.open_seed_optimize,
        );

        let input_vault_account = get_vault_account(&pool_state, &input_mint, protocol_params);
        let output_vault_account = get_vault_account(&pool_state, &output_mint, protocol_params);

        let observation_state_account = if protocol_params.observation_state == Pubkey::default() {
            get_observation_state_pda(&pool_state).unwrap()
        } else {
            protocol_params.observation_state
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if params.create_input_mint_ata {
            push_create_or_wrap_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &input_mint,
                &input_token_program,
                amount_in,
                params.open_seed_optimize,
            );
        }

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &output_mint,
                &output_token_program,
                params.open_seed_optimize,
            );
        }

        // Create buy instruction
        let accounts: [AccountMeta; 13] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            AccountMeta::new_readonly(protocol_params.amm_config, false), // Amm Config (readonly)
            AccountMeta::new(pool_state, false),           // Pool State
            AccountMeta::new(input_token_account, false),  // Input Token Account
            AccountMeta::new(output_token_account, false), // Output Token Account
            AccountMeta::new(input_vault_account, false),  // Input Vault Account
            AccountMeta::new(output_vault_account, false), // Output Vault Account
            AccountMeta::new_readonly(input_token_program, false), // Input Token Program (readonly)
            AccountMeta::new_readonly(output_token_program, false), // Output Token Program (readonly)
            AccountMeta::new_readonly(input_mint, false),           // Input token mint (readonly)
            AccountMeta::new_readonly(output_mint, false),          // Output token mint (readonly)
            AccountMeta::new(observation_state_account, false),     // Observation State Account
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        if let Some(amount_out) = params.fixed_output_amount {
            data[..8].copy_from_slice(&SWAP_BASE_OUT_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_in.to_le_bytes());
            data[16..24].copy_from_slice(&amount_out.to_le_bytes());
        } else {
            let minimum_amount_out = compute_swap_amount(
                protocol_params.base_reserve,
                protocol_params.quote_reserve,
                is_base_in,
                amount_in,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            )
            .min_amount_out;
            data[..8].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_in.to_le_bytes());
            data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
        }

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_CPMM,
            &data,
            accounts.to_vec(),
        ));

        if params.close_input_mint_ata {
            push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &input_mint);
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
            .downcast_ref::<RaydiumCpmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        if params.input_amount.is_none() || params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Token amount is not set"));
        }

        let pool_state = if protocol_params.pool_state == Pubkey::default() {
            get_pool_pda(
                &protocol_params.amm_config,
                &protocol_params.base_mint,
                &protocol_params.quote_mint,
            )
            .unwrap()
        } else {
            protocol_params.pool_state
        };

        let is_wsol = protocol_params.base_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT;

        let is_usdc = protocol_params.base_mint == crate::constants::USDC_TOKEN_ACCOUNT
            || protocol_params.quote_mint == crate::constants::USDC_TOKEN_ACCOUNT;

        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_quote_out = protocol_params.quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.quote_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        let input_mint =
            if is_quote_out { protocol_params.base_mint } else { protocol_params.quote_mint };
        let input_token_program = if is_quote_out {
            protocol_params.base_token_program
        } else {
            protocol_params.quote_token_program
        };
        let output_mint =
            if is_quote_out { protocol_params.quote_mint } else { protocol_params.base_mint };
        let output_token_program = if is_quote_out {
            protocol_params.quote_token_program
        } else {
            protocol_params.base_token_program
        };

        let output_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &output_mint,
            &output_token_program,
            params.open_seed_optimize,
        );
        let input_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &input_mint,
            &input_token_program,
            params.open_seed_optimize,
        );

        let output_vault_account = get_vault_account(&pool_state, &output_mint, protocol_params);
        let input_vault_account = get_vault_account(&pool_state, &input_mint, protocol_params);

        let observation_state_account = if protocol_params.observation_state == Pubkey::default() {
            get_observation_state_pda(&pool_state).unwrap()
        } else {
            protocol_params.observation_state
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(4);

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &output_mint,
                &output_token_program,
                params.open_seed_optimize,
            );
        }

        // Create sell instruction
        let accounts: [AccountMeta; 13] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            AccountMeta::new_readonly(protocol_params.amm_config, false), // Amm Config (readonly)
            AccountMeta::new(pool_state, false),           // Pool State
            AccountMeta::new(input_token_account, false),  // Input Token Account
            AccountMeta::new(output_token_account, false), // Output Token Account
            AccountMeta::new(input_vault_account, false),  // Input Vault Account
            AccountMeta::new(output_vault_account, false), // Output Vault Account
            AccountMeta::new_readonly(input_token_program, false), // Input Token Program (readonly)
            AccountMeta::new_readonly(output_token_program, false), // Output Token Program (readonly)
            AccountMeta::new_readonly(input_mint, false),           // Input token mint (readonly)
            AccountMeta::new_readonly(output_mint, false),          // Output token mint (readonly)
            AccountMeta::new(observation_state_account, false),     // Observation State Account
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        let amount_in = params.input_amount.unwrap_or(0);
        if let Some(amount_out) = params.fixed_output_amount {
            data[..8].copy_from_slice(&SWAP_BASE_OUT_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_in.to_le_bytes());
            data[16..24].copy_from_slice(&amount_out.to_le_bytes());
        } else {
            let minimum_amount_out = compute_swap_amount(
                protocol_params.base_reserve,
                protocol_params.quote_reserve,
                is_quote_out,
                amount_in,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            )
            .min_amount_out;
            data[..8].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_in.to_le_bytes());
            data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());
        }

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_CPMM,
            &data,
            accounts.to_vec(),
        ));

        if params.close_output_mint_ata {
            push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &output_mint);
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                &input_token_program,
                &input_token_account,
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &[&params.payer.pubkey()],
            )?);
        }

        Ok(instructions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        common::GasFeeStrategy,
        swqos::TradeType,
        trading::core::params::{DexParamEnum, SwapParams},
    };
    use solana_sdk::{pubkey::Pubkey, signature::Keypair};
    use std::sync::Arc;

    fn pk(seed: u8) -> Pubkey {
        Pubkey::new_from_array([seed; 32])
    }

    fn cpmm_params() -> RaydiumCpmmParams {
        RaydiumCpmmParams {
            pool_state: pk(1),
            amm_config: pk(2),
            base_mint: crate::constants::WSOL_TOKEN_ACCOUNT,
            quote_mint: pk(3),
            base_reserve: 1_000_000_000,
            quote_reserve: 2_000_000_000,
            base_vault: pk(4),
            quote_vault: pk(5),
            base_token_program: crate::constants::TOKEN_PROGRAM,
            quote_token_program: crate::constants::TOKEN_PROGRAM,
            observation_state: pk(6),
        }
    }

    fn swap_params(fixed_output_amount: Option<u64>) -> SwapParams {
        SwapParams {
            rpc: None,
            payer: Arc::new(Keypair::new()),
            trade_type: TradeType::Buy,
            input_mint: crate::constants::WSOL_TOKEN_ACCOUNT,
            input_token_program: None,
            output_mint: pk(3),
            output_token_program: None,
            input_amount: Some(100_000),
            slippage_basis_points: Some(100),
            address_lookup_table_account: None,
            recent_blockhash: None,
            wait_tx_confirmed: false,
            protocol_params: DexParamEnum::RaydiumCpmm(cpmm_params()),
            open_seed_optimize: true,
            swqos_clients: Arc::new(Vec::new()),
            middleware_manager: None,
            durable_nonce: None,
            with_tip: false,
            create_input_mint_ata: false,
            close_input_mint_ata: false,
            create_output_mint_ata: false,
            close_output_mint_ata: false,
            fixed_output_amount,
            gas_fee_strategy: GasFeeStrategy::new(),
            simulate: true,
            log_enabled: false,
            wait_for_all_submits: false,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: 0,
            effective_core_ids: Arc::new(Vec::new()),
            check_min_tip: false,
            grpc_recv_us: None,
            use_exact_sol_amount: None,
        }
    }

    #[tokio::test]
    async fn raydium_cpmm_uses_base_in_and_readonly_amm_config_by_default() {
        let instructions =
            RaydiumCpmmInstructionBuilder.build_buy_instructions(&swap_params(None)).await.unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], SWAP_BASE_IN_DISCRIMINATOR);
        assert_eq!(ix.accounts[2].pubkey, pk(2));
        assert!(!ix.accounts[2].is_writable);
    }

    #[tokio::test]
    async fn raydium_cpmm_uses_base_output_when_fixed_output_is_set() {
        let instructions = RaydiumCpmmInstructionBuilder
            .build_buy_instructions(&swap_params(Some(42)))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..8], SWAP_BASE_OUT_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[8..16].try_into().unwrap()), 100_000);
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 42);
    }

    #[tokio::test]
    async fn raydium_cpmm_usdc_buy_create_input_uses_usdc_accounts() {
        let mut protocol_params = cpmm_params();
        protocol_params.base_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        protocol_params.quote_mint = pk(3);

        let mut params = swap_params(Some(42));
        params.protocol_params = DexParamEnum::RaydiumCpmm(protocol_params);
        params.input_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        params.create_input_mint_ata = true;
        params.open_seed_optimize = false;

        let instructions =
            RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let create_ix = instructions.first().unwrap();
        let swap_ix = instructions.last().unwrap();

        assert_eq!(create_ix.program_id, crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID);
        assert_eq!(create_ix.accounts[3].pubkey, crate::constants::USDC_TOKEN_ACCOUNT);
        assert_eq!(swap_ix.accounts[10].pubkey, crate::constants::USDC_TOKEN_ACCOUNT);
    }
}
