use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::{
        token_account_setup::{
            push_close_wsol_if_needed, push_create_or_wrap_user_token_account,
            push_create_user_token_account,
        },
        utils::raydium_amm_v4::{
            accounts, SWAP_BASE_IN_V2_DISCRIMINATOR, SWAP_BASE_OUT_V2_DISCRIMINATOR,
        },
    },
    trading::core::{
        params::{RaydiumAmmV4Params, SwapParams},
        traits::InstructionBuilder,
    },
    utils::calc::raydium_amm_v4::compute_swap_amount_for_pool,
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct RaydiumAmmV4InstructionBuilder;

fn build_swap_accounts(
    protocol_params: &RaydiumAmmV4Params,
    user_source: Pubkey,
    user_destination: Pubkey,
    payer: Pubkey,
) -> Vec<AccountMeta> {
    // Official Raydium 2026-07-22 + raydium-sdk-V2: always SwapBaseIn/Out V2.
    // Owner is signer-only (`isWritable: false`).
    vec![
        crate::constants::TOKEN_PROGRAM_META,
        AccountMeta::new(protocol_params.amm, false),
        accounts::AUTHORITY_META,
        AccountMeta::new(protocol_params.token_coin, false),
        AccountMeta::new(protocol_params.token_pc, false),
        AccountMeta::new(user_source, false),
        AccountMeta::new(user_destination, false),
        AccountMeta::new_readonly(payer, true),
    ]
}

fn resolve_swap_pair(
    params: &SwapParams,
    pool: &RaydiumAmmV4Params,
) -> Result<(Pubkey, Pubkey, bool)> {
    let normalize = |mint| {
        if mint == crate::constants::SOL_TOKEN_ACCOUNT {
            crate::constants::WSOL_TOKEN_ACCOUNT
        } else {
            mint
        }
    };
    let input = normalize(params.input_mint);
    let output = normalize(params.output_mint);
    for program in [params.input_token_program, params.output_token_program].into_iter().flatten() {
        if program != crate::constants::TOKEN_PROGRAM {
            return Err(anyhow!("AMM v4 requires the SPL Token program"));
        }
    }
    let coin_in = if input == pool.coin_mint && output == pool.pc_mint {
        true
    } else if input == pool.pc_mint && output == pool.coin_mint {
        false
    } else {
        return Err(anyhow!("AMM v4 swap pair does not match pool"));
    };
    Ok((input, output, coin_in))
}

fn swap_discriminators() -> (&'static [u8], &'static [u8]) {
    (SWAP_BASE_IN_V2_DISCRIMINATOR, SWAP_BASE_OUT_V2_DISCRIMINATOR)
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

        let (input_mint, output_mint, is_base_in) = resolve_swap_pair(params, protocol_params)?;
        let amount_in = params.input_amount.unwrap_or(0);
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
        let accounts = build_swap_accounts(
            protocol_params,
            user_source_token_account,
            user_destination_token_account,
            params.payer.pubkey(),
        );
        let (disc_in, disc_out) = swap_discriminators();
        // Create instruction data
        let mut data = [0u8; 17];
        if let Some(amount_out) = params.fixed_output_amount {
            data[..1].copy_from_slice(disc_out);
            data[1..9].copy_from_slice(&amount_in.to_le_bytes());
            data[9..17].copy_from_slice(&amount_out.to_le_bytes());
        } else {
            let minimum_amount_out = compute_swap_amount_for_pool(
                protocol_params,
                is_base_in,
                amount_in,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            )?
            .min_amount_out;
            data[..1].copy_from_slice(disc_in);
            data[1..9].copy_from_slice(&amount_in.to_le_bytes());
            data[9..17].copy_from_slice(&minimum_amount_out.to_le_bytes());
        }

        instructions.push(Instruction::new_with_bytes(accounts::RAYDIUM_AMM_V4, &data, accounts));

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

        if params.input_amount.is_none() || params.input_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Token amount is not set"));
        }

        let (input_mint, output_mint, is_base_in) = resolve_swap_pair(params, protocol_params)?;

        let amount_in = params.input_amount.unwrap();
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

        let accounts = build_swap_accounts(
            protocol_params,
            user_source_token_account,
            user_destination_token_account,
            params.payer.pubkey(),
        );
        let (disc_in, disc_out) = swap_discriminators();
        let mut data = [0u8; 17];
        if let Some(amount_out) = params.fixed_output_amount {
            data[..1].copy_from_slice(disc_out);
            data[1..9].copy_from_slice(&amount_in.to_le_bytes());
            data[9..17].copy_from_slice(&amount_out.to_le_bytes());
        } else {
            let minimum_amount_out = compute_swap_amount_for_pool(
                protocol_params,
                is_base_in,
                amount_in,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            )?
            .min_amount_out;
            data[..1].copy_from_slice(disc_in);
            data[1..9].copy_from_slice(&amount_in.to_le_bytes());
            data[9..17].copy_from_slice(&minimum_amount_out.to_le_bytes());
        }

        instructions.push(Instruction::new_with_bytes(accounts::RAYDIUM_AMM_V4, &data, accounts));

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
            address_lookup_table_accounts: Vec::new(),
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
            wait_for_all_submits: false,
            use_dedicated_sender_threads: false,
            sender_thread_cores: None,
            max_sender_concurrency: 0,
            effective_core_ids: Arc::new(Vec::new()),
            check_min_tip: false,
            transaction_version: crate::common::TradeTransactionVersion::V0,
            grpc_recv_us: None,
            use_exact_sol_amount: None,
        }
    }

    #[tokio::test]
    async fn raydium_amm_v4_always_uses_swap_v2_layout() {
        let instructions = RaydiumAmmV4InstructionBuilder
            .build_buy_instructions(&swap_params(market_params(), None))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        // Official 2026-07-22: routers always use V2 (8 accounts), even if OpenBook keys present.
        assert_eq!(ix.accounts.len(), 8);
        assert_eq!(&ix.data[..1], SWAP_BASE_IN_V2_DISCRIMINATOR);
        assert_eq!(ix.accounts[3].pubkey, pk(3)); // coin vault from Params::new
        assert_eq!(ix.accounts[4].pubkey, pk(4)); // pc vault
        assert!(ix.accounts[7].is_signer);
        assert!(!ix.accounts[7].is_writable); // raydium-sdk-V2: owner readonly signer
    }

    #[tokio::test]
    async fn raydium_amm_v4_uses_base_out_when_fixed_output_is_set() {
        let instructions = RaydiumAmmV4InstructionBuilder
            .build_buy_instructions(&swap_params(market_params(), Some(42)))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();

        assert_eq!(&ix.data[..1], SWAP_BASE_OUT_V2_DISCRIMINATOR);
        assert_eq!(u64::from_le_bytes(ix.data[1..9].try_into().unwrap()), 100_000);
        assert_eq!(u64::from_le_bytes(ix.data[9..17].try_into().unwrap()), 42);
    }

    #[tokio::test]
    async fn raydium_amm_v4_swap_v2_when_openbook_accounts_absent() {
        let params = RaydiumAmmV4Params::new(
            pk(1),
            crate::constants::WSOL_TOKEN_ACCOUNT,
            pk(2),
            pk(3),
            pk(4),
            1_000_000_000,
            2_000_000_000,
        );
        let instructions = RaydiumAmmV4InstructionBuilder
            .build_buy_instructions(&swap_params(params, None))
            .await
            .unwrap();
        let ix = instructions.last().unwrap();
        assert_eq!(ix.accounts.len(), 8);
        assert_eq!(&ix.data[..1], SWAP_BASE_IN_V2_DISCRIMINATOR);
        assert_eq!(ix.accounts[3].pubkey, pk(3)); // coin vault
        assert_eq!(ix.accounts[4].pubkey, pk(4)); // pc vault
        assert!(!ix.accounts[7].is_writable);
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
    #[tokio::test]
    async fn amm_v4_direction_follows_requested_pair_including_reverse_buy() {
        let pool = market_params();
        let mut params = swap_params(pool.clone(), None);
        params.input_mint = pool.pc_mint;
        params.output_mint = pool.coin_mint;
        let ixs = RaydiumAmmV4InstructionBuilder.build_buy_instructions(&params).await.unwrap();
        let ix = ixs.iter().find(|ix| ix.program_id == accounts::RAYDIUM_AMM_V4).unwrap();
        let expected =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &pool.pc_mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        assert_eq!(ix.accounts[5].pubkey, expected);
        let minimum = crate::utils::calc::raydium_amm_v4::compute_swap_amount(
            pool.coin_reserve,
            pool.pc_reserve,
            false,
            params.input_amount.unwrap(),
            params.slippage_basis_points.unwrap(),
        )
        .min_amount_out;
        assert_eq!(u64::from_le_bytes(ix.data[9..17].try_into().unwrap()), minimum);
        params.output_mint = Pubkey::new_unique();
        assert!(RaydiumAmmV4InstructionBuilder.build_buy_instructions(&params).await.is_err());
    }
}
