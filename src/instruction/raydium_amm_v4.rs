use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::raydium_amm_v4::{
            accounts, SWAP_BASE_IN_DISCRIMINATOR, SWAP_BASE_OUT_DISCRIMINATOR,
        },
    },
    trading::core::{
        params::{RaydiumAmmV4Params, SwapParams},
        traits::InstructionBuilder,
    },
    utils::calc::raydium_amm_v4::compute_swap_amount,
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct RaydiumAmmV4InstructionBuilder;

fn ensure_market_accounts(params: &RaydiumAmmV4Params) -> Result<()> {
    let required = [
        ("amm_open_orders", params.amm_open_orders),
        ("amm_target_orders", params.amm_target_orders),
        ("serum_program", params.serum_program),
        ("serum_market", params.serum_market),
        ("serum_bids", params.serum_bids),
        ("serum_asks", params.serum_asks),
        ("serum_event_queue", params.serum_event_queue),
        ("serum_coin_vault_account", params.serum_coin_vault_account),
        ("serum_pc_vault_account", params.serum_pc_vault_account),
        ("serum_vault_signer", params.serum_vault_signer),
    ];

    for (name, account) in required {
        if account == Pubkey::default() {
            return Err(anyhow!(
                "Raydium AMM v4 requires {}; use RaydiumAmmV4Params::from_amm_address_by_rpc or with_market_accounts",
                name
            ));
        }
    }
    Ok(())
}

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
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumAmmV4"))?;
        ensure_market_accounts(protocol_params)?;

        let is_wsol = protocol_params.coin_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.pc_mint == crate::constants::WSOL_TOKEN_ACCOUNT;

        let is_usdc = protocol_params.coin_mint == crate::constants::USDC_TOKEN_ACCOUNT
            || protocol_params.pc_mint == crate::constants::USDC_TOKEN_ACCOUNT;

        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_base_in = protocol_params.coin_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.coin_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        let amount_in: u64 = params.input_amount.unwrap_or(0);
        let input_mint =
            if is_base_in { protocol_params.coin_mint } else { protocol_params.pc_mint };
        let output_mint =
            if is_base_in { protocol_params.pc_mint } else { protocol_params.coin_mint };

        let user_source_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &input_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        let user_destination_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &output_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if params.create_input_mint_ata {
            push_create_or_wrap_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &input_mint,
                &crate::constants::TOKEN_PROGRAM,
                amount_in,
                params.open_seed_optimize,
            );
        }

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &output_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        }

        // Create buy instruction
        let accounts: [AccountMeta; 18] = [
            crate::constants::TOKEN_PROGRAM_META, // Token Program (readonly)
            AccountMeta::new(protocol_params.amm, false), // Amm
            accounts::AUTHORITY_META,             // Authority (readonly)
            AccountMeta::new(protocol_params.amm_open_orders, false), // Amm Open Orders
            AccountMeta::new(protocol_params.amm_target_orders, false), // Amm Target Orders
            AccountMeta::new(protocol_params.token_coin, false), // Pool Coin Token Account
            AccountMeta::new(protocol_params.token_pc, false), // Pool Pc Token Account
            AccountMeta::new_readonly(protocol_params.serum_program, false), // Serum Program
            AccountMeta::new(protocol_params.serum_market, false), // Serum Market
            AccountMeta::new(protocol_params.serum_bids, false), // Serum Bids
            AccountMeta::new(protocol_params.serum_asks, false), // Serum Asks
            AccountMeta::new(protocol_params.serum_event_queue, false), // Serum Event Queue
            AccountMeta::new(protocol_params.serum_coin_vault_account, false), // Serum Coin Vault Account
            AccountMeta::new(protocol_params.serum_pc_vault_account, false), // Serum Pc Vault Account
            AccountMeta::new_readonly(protocol_params.serum_vault_signer, false), // Serum Vault Signer
            AccountMeta::new(user_source_token_account, false), // User Source Token Account
            AccountMeta::new(user_destination_token_account, false), // User Destination Token Account
            AccountMeta::new(params.payer.pubkey(), true),           // User Source Owner
        ];
        // Create instruction data
        let mut data = [0u8; 17];
        if let Some(amount_out) = params.fixed_output_amount {
            data[..1].copy_from_slice(&SWAP_BASE_OUT_DISCRIMINATOR);
            data[1..9].copy_from_slice(&amount_in.to_le_bytes());
            data[9..17].copy_from_slice(&amount_out.to_le_bytes());
        } else {
            let minimum_amount_out = compute_swap_amount(
                protocol_params.coin_reserve,
                protocol_params.pc_reserve,
                is_base_in,
                amount_in,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            )
            .min_amount_out;
            data[..1].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
            data[1..9].copy_from_slice(&amount_in.to_le_bytes());
            data[9..17].copy_from_slice(&minimum_amount_out.to_le_bytes());
        }

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_AMM_V4,
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
            .downcast_ref::<RaydiumAmmV4Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumAmmV4"))?;
        ensure_market_accounts(protocol_params)?;

        if params.input_amount.is_none() || params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Token amount is not set"));
        }

        let is_wsol = protocol_params.coin_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.pc_mint == crate::constants::WSOL_TOKEN_ACCOUNT;

        let is_usdc = protocol_params.coin_mint == crate::constants::USDC_TOKEN_ACCOUNT
            || protocol_params.pc_mint == crate::constants::USDC_TOKEN_ACCOUNT;

        if !is_wsol && !is_usdc {
            return Err(anyhow!("Pool must contain WSOL or USDC"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_base_in = protocol_params.pc_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || protocol_params.pc_mint == crate::constants::USDC_TOKEN_ACCOUNT;
        let input_mint =
            if is_base_in { protocol_params.coin_mint } else { protocol_params.pc_mint };
        let output_mint =
            if is_base_in { protocol_params.pc_mint } else { protocol_params.coin_mint };

        let user_source_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &input_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        let user_destination_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &output_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(4);

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &output_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        }

        // Create buy instruction
        let accounts: [AccountMeta; 18] = [
            crate::constants::TOKEN_PROGRAM_META, // Token Program (readonly)
            AccountMeta::new(protocol_params.amm, false), // Amm
            accounts::AUTHORITY_META,             // Authority (readonly)
            AccountMeta::new(protocol_params.amm_open_orders, false), // Amm Open Orders
            AccountMeta::new(protocol_params.amm_target_orders, false), // Amm Target Orders
            AccountMeta::new(protocol_params.token_coin, false), // Pool Coin Token Account
            AccountMeta::new(protocol_params.token_pc, false), // Pool Pc Token Account
            AccountMeta::new_readonly(protocol_params.serum_program, false), // Serum Program
            AccountMeta::new(protocol_params.serum_market, false), // Serum Market
            AccountMeta::new(protocol_params.serum_bids, false), // Serum Bids
            AccountMeta::new(protocol_params.serum_asks, false), // Serum Asks
            AccountMeta::new(protocol_params.serum_event_queue, false), // Serum Event Queue
            AccountMeta::new(protocol_params.serum_coin_vault_account, false), // Serum Coin Vault Account
            AccountMeta::new(protocol_params.serum_pc_vault_account, false), // Serum Pc Vault Account
            AccountMeta::new_readonly(protocol_params.serum_vault_signer, false), // Serum Vault Signer
            AccountMeta::new(user_source_token_account, false), // User Source Token Account
            AccountMeta::new(user_destination_token_account, false), // User Destination Token Account
            AccountMeta::new(params.payer.pubkey(), true),           // User Source Owner
        ];
        // Create instruction data
        let mut data = [0u8; 17];
        let amount_in = params.input_amount.unwrap_or(0);
        if let Some(amount_out) = params.fixed_output_amount {
            data[..1].copy_from_slice(&SWAP_BASE_OUT_DISCRIMINATOR);
            data[1..9].copy_from_slice(&amount_in.to_le_bytes());
            data[9..17].copy_from_slice(&amount_out.to_le_bytes());
        } else {
            let minimum_amount_out = compute_swap_amount(
                protocol_params.coin_reserve,
                protocol_params.pc_reserve,
                is_base_in,
                amount_in,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            )
            .min_amount_out;
            data[..1].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
            data[1..9].copy_from_slice(&amount_in.to_le_bytes());
            data[9..17].copy_from_slice(&minimum_amount_out.to_le_bytes());
        }

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_AMM_V4,
            &data,
            accounts.to_vec(),
        ));

        if params.close_output_mint_ata {
            push_close_wsol_if_needed(&mut instructions, &params.payer.pubkey(), &output_mint);
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

    fn market_params() -> RaydiumAmmV4Params {
        RaydiumAmmV4Params::new(
            pk(1),
            crate::constants::WSOL_TOKEN_ACCOUNT,
            pk(2),
            pk(3),
            pk(4),
            1_000_000_000,
            2_000_000_000,
        )
        .with_market_accounts(
            pk(5),
            pk(6),
            pk(7),
            pk(8),
            pk(9),
            pk(10),
            pk(11),
            pk(12),
            pk(13),
            pk(14),
        )
    }

    fn swap_params(
        protocol_params: RaydiumAmmV4Params,
        fixed_output_amount: Option<u64>,
    ) -> SwapParams {
        SwapParams {
            rpc: None,
            payer: Arc::new(Keypair::new()),
            trade_type: TradeType::Buy,
            input_mint: crate::constants::WSOL_TOKEN_ACCOUNT,
            input_token_program: None,
            output_mint: pk(2),
            output_token_program: None,
            input_amount: Some(100_000),
            slippage_basis_points: Some(100),
            address_lookup_table_account: None,
            recent_blockhash: None,
            wait_tx_confirmed: false,
            protocol_params: DexParamEnum::RaydiumAmmV4(protocol_params),
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
    async fn raydium_amm_v4_uses_idl_market_account_order() {
        let instructions = RaydiumAmmV4InstructionBuilder
            .build_buy_instructions(&swap_params(market_params(), None))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(ix.accounts.len(), 18);
        assert_eq!(&ix.data[..1], SWAP_BASE_IN_DISCRIMINATOR);
        assert_eq!(ix.accounts[3].pubkey, pk(5));
        assert_eq!(ix.accounts[4].pubkey, pk(6));
        assert_eq!(ix.accounts[7].pubkey, pk(7));
        assert_eq!(ix.accounts[8].pubkey, pk(8));
        assert_eq!(ix.accounts[9].pubkey, pk(9));
        assert_eq!(ix.accounts[10].pubkey, pk(10));
        assert_eq!(ix.accounts[11].pubkey, pk(11));
        assert_eq!(ix.accounts[12].pubkey, pk(12));
        assert_eq!(ix.accounts[13].pubkey, pk(13));
        assert_eq!(ix.accounts[14].pubkey, pk(14));
        assert!(!ix.accounts[7].is_writable);
        assert!(!ix.accounts[14].is_writable);
    }

    #[tokio::test]
    async fn raydium_amm_v4_uses_base_out_when_fixed_output_is_set() {
        let instructions = RaydiumAmmV4InstructionBuilder
            .build_buy_instructions(&swap_params(market_params(), Some(42)))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..1], SWAP_BASE_OUT_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[1..9].try_into().unwrap()), 100_000);
        assert_eq!(u64::from_le_bytes(ix.data[9..17].try_into().unwrap()), 42);
    }

    #[tokio::test]
    async fn raydium_amm_v4_rejects_placeholder_market_accounts() {
        let err = RaydiumAmmV4InstructionBuilder
            .build_buy_instructions(&swap_params(
                RaydiumAmmV4Params::new(
                    pk(1),
                    crate::constants::WSOL_TOKEN_ACCOUNT,
                    pk(2),
                    pk(3),
                    pk(4),
                    1_000_000_000,
                    2_000_000_000,
                ),
                None,
            ))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("amm_open_orders"));
    }

    #[tokio::test]
    async fn raydium_amm_v4_usdc_buy_create_input_builds_usdc_ata() {
        let mut protocol_params = market_params();
        protocol_params.coin_mint = crate::constants::USDC_TOKEN_ACCOUNT;

        let mut params = swap_params(protocol_params, Some(42));
        params.input_mint = crate::constants::USDC_TOKEN_ACCOUNT;
        params.create_input_mint_ata = true;
        params.open_seed_optimize = false;

        let instructions =
            RaydiumAmmV4InstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let create_ix = instructions.first().unwrap();

        assert_eq!(create_ix.program_id, crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID);
        assert_eq!(create_ix.accounts[3].pubkey, crate::constants::USDC_TOKEN_ACCOUNT);
    }
}
