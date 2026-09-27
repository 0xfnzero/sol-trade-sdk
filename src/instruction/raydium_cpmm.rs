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
    utils::calc::raydium_cpmm::compute_swap_amount_for_pool,
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct RaydiumCpmmInstructionBuilder;

struct CpmmSwapContext {
    pool_state: Pubkey,
    input_mint: Pubkey,
    output_mint: Pubkey,
    input_token_program: Pubkey,
    output_token_program: Pubkey,
    input_vault: Pubkey,
    output_vault: Pubkey,
    observation_state: Pubkey,
    is_base_in: bool,
}

fn normalize_native_sol(mint: Pubkey) -> Pubkey {
    if mint == crate::constants::SOL_TOKEN_ACCOUNT {
        crate::constants::WSOL_TOKEN_ACCOUNT
    } else {
        mint
    }
}

fn resolve_swap_context(
    params: &SwapParams,
    protocol_params: &RaydiumCpmmParams,
) -> Result<CpmmSwapContext> {
    if protocol_params.base_mint == protocol_params.quote_mint {
        return Err(anyhow!("Raydium CPMM pool mints must be distinct"));
    }

    let input_mint = normalize_native_sol(params.input_mint);
    let output_mint = normalize_native_sol(params.output_mint);
    let is_base_in = if input_mint == protocol_params.base_mint
        && output_mint == protocol_params.quote_mint
    {
        true
    } else if input_mint == protocol_params.quote_mint && output_mint == protocol_params.base_mint {
        false
    } else {
        return Err(anyhow!(
            "Requested swap pair {}/{} does not match Raydium CPMM pool {}/{}",
            input_mint,
            output_mint,
            protocol_params.base_mint,
            protocol_params.quote_mint
        ));
    };

    let (input_token_program, output_token_program) = if is_base_in {
        (protocol_params.base_token_program, protocol_params.quote_token_program)
    } else {
        (protocol_params.quote_token_program, protocol_params.base_token_program)
    };
    if let Some(requested) = params.input_token_program {
        if requested != input_token_program {
            return Err(anyhow!("Input token program does not match Raydium CPMM pool state"));
        }
    }
    if let Some(requested) = params.output_token_program {
        if requested != output_token_program {
            return Err(anyhow!("Output token program does not match Raydium CPMM pool state"));
        }
    }

    let pool_state = if protocol_params.pool_state == Pubkey::default() {
        get_pool_pda(
            &protocol_params.amm_config,
            &protocol_params.base_mint,
            &protocol_params.quote_mint,
        )
        .ok_or_else(|| anyhow!("Failed to derive Raydium CPMM pool address"))?
    } else {
        protocol_params.pool_state
    };
    let input_vault = get_vault_account(&pool_state, &input_mint, protocol_params)?;
    let output_vault = get_vault_account(&pool_state, &output_mint, protocol_params)?;
    let observation_state = if protocol_params.observation_state == Pubkey::default() {
        get_observation_state_pda(&pool_state)
            .ok_or_else(|| anyhow!("Failed to derive Raydium CPMM observation address"))?
    } else {
        protocol_params.observation_state
    };

    Ok(CpmmSwapContext {
        pool_state,
        input_mint,
        output_mint,
        input_token_program,
        output_token_program,
        input_vault,
        output_vault,
        observation_state,
        is_base_in,
    })
}

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

        let context = resolve_swap_context(params, protocol_params)?;

        let amount_in: u64 = params.input_amount.unwrap_or(0);

        let input_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &context.input_mint,
            &context.input_token_program,
            params.open_seed_optimize,
        );
        let output_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &context.output_mint,
            &context.output_token_program,
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
                &context.input_mint,
                &context.input_token_program,
                amount_in,
                params.open_seed_optimize,
            );
        }

        if params.create_output_mint_ata {
            push_create_user_token_account(
                &mut instructions,
                &params.payer.pubkey(),
                &context.output_mint,
                &context.output_token_program,
                params.open_seed_optimize,
            );
        }

        // Create buy instruction
        let accounts: [AccountMeta; 13] = [
            AccountMeta::new_readonly(params.payer.pubkey(), true), // Payer (signer, IDL not writable)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            AccountMeta::new_readonly(protocol_params.amm_config, false), // Amm Config (readonly)
            AccountMeta::new(context.pool_state, false),   // Pool State
            AccountMeta::new(input_token_account, false),  // Input Token Account
            AccountMeta::new(output_token_account, false), // Output Token Account
            AccountMeta::new(context.input_vault, false),  // Input Vault Account
            AccountMeta::new(context.output_vault, false), // Output Vault Account
            AccountMeta::new_readonly(context.input_token_program, false),
            AccountMeta::new_readonly(context.output_token_program, false),
            AccountMeta::new_readonly(context.input_mint, false),
            AccountMeta::new_readonly(context.output_mint, false),
            AccountMeta::new(context.observation_state, false),
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        if let Some(amount_out) = params.fixed_output_amount {
            data[..8].copy_from_slice(&SWAP_BASE_OUT_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_in.to_le_bytes());
            data[16..24].copy_from_slice(&amount_out.to_le_bytes());
        } else {
            let minimum_amount_out = compute_swap_amount_for_pool(
                protocol_params,
                context.is_base_in,
                amount_in,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            )?
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
            push_close_wsol_if_needed(
                &mut instructions,
                &params.payer.pubkey(),
                &context.input_mint,
            );
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

        let context = resolve_swap_context(params, protocol_params)?;

        let output_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &context.output_mint,
            &context.output_token_program,
            params.open_seed_optimize,
        );
        let input_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &context.input_mint,
            &context.input_token_program,
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
                &context.output_mint,
                &context.output_token_program,
                params.open_seed_optimize,
            );
        }

        // Create sell instruction
        let accounts: [AccountMeta; 13] = [
            AccountMeta::new_readonly(params.payer.pubkey(), true), // Payer (signer, IDL not writable)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            AccountMeta::new_readonly(protocol_params.amm_config, false), // Amm Config (readonly)
            AccountMeta::new(context.pool_state, false),   // Pool State
            AccountMeta::new(input_token_account, false),  // Input Token Account
            AccountMeta::new(output_token_account, false), // Output Token Account
            AccountMeta::new(context.input_vault, false),  // Input Vault Account
            AccountMeta::new(context.output_vault, false), // Output Vault Account
            AccountMeta::new_readonly(context.input_token_program, false),
            AccountMeta::new_readonly(context.output_token_program, false),
            AccountMeta::new_readonly(context.input_mint, false),
            AccountMeta::new_readonly(context.output_mint, false),
            AccountMeta::new(context.observation_state, false),
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        let amount_in = params.input_amount.unwrap_or(0);
        if let Some(amount_out) = params.fixed_output_amount {
            data[..8].copy_from_slice(&SWAP_BASE_OUT_DISCRIMINATOR);
            data[8..16].copy_from_slice(&amount_in.to_le_bytes());
            data[16..24].copy_from_slice(&amount_out.to_le_bytes());
        } else {
            let minimum_amount_out = compute_swap_amount_for_pool(
                protocol_params,
                context.is_base_in,
                amount_in,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
            )?
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
            push_close_wsol_if_needed(
                &mut instructions,
                &params.payer.pubkey(),
                &context.output_mint,
            );
        }
        if params.close_input_mint_ata {
            instructions.push(crate::common::spl_token::close_account(
                &context.input_token_program,
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
    use solana_sdk::{pubkey, pubkey::Pubkey, signature::Keypair};
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
            trade_fee_rate: accounts::TRADE_FEE_RATE,
            protocol_fee_rate: accounts::PROTOCOL_FEE_RATE,
            fund_fee_rate: accounts::FUND_FEE_RATE,
            creator_fee_rate: 0,
            creator_fee_on: 0,
            enable_creator_fee: false,
            base_transfer_fee: Default::default(),
            quote_transfer_fee: Default::default(),
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
            address_lookup_table_accounts: Vec::new(),
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
            transaction_version: crate::common::TradeTransactionVersion::V0,
            grpc_recv_us: None,
            use_exact_sol_amount: None,
            precheck: None,
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

    #[tokio::test]
    async fn raydium_cpmm_supports_arbitrary_token_pair_in_both_directions() {
        let base_mint = pk(20);
        let quote_mint = pk(21);
        let base_program = pk(22);
        let quote_program = pk(23);
        let base_vault = pk(24);
        let quote_vault = pk(25);
        let mut protocol_params = cpmm_params();
        protocol_params.base_mint = base_mint;
        protocol_params.quote_mint = quote_mint;
        protocol_params.base_token_program = base_program;
        protocol_params.quote_token_program = quote_program;
        protocol_params.base_vault = base_vault;
        protocol_params.quote_vault = quote_vault;

        let mut base_to_quote = swap_params(Some(1));
        base_to_quote.input_mint = base_mint;
        base_to_quote.output_mint = quote_mint;
        base_to_quote.protocol_params = DexParamEnum::RaydiumCpmm(protocol_params.clone());
        let instructions =
            RaydiumCpmmInstructionBuilder.build_sell_instructions(&base_to_quote).await.unwrap();
        let ix = instructions.last().unwrap();
        assert_eq!(ix.accounts[6].pubkey, base_vault);
        assert_eq!(ix.accounts[7].pubkey, quote_vault);
        assert_eq!(ix.accounts[8].pubkey, base_program);
        assert_eq!(ix.accounts[9].pubkey, quote_program);
        assert_eq!(ix.accounts[10].pubkey, base_mint);
        assert_eq!(ix.accounts[11].pubkey, quote_mint);

        let mut quote_to_base = swap_params(Some(1));
        quote_to_base.input_mint = quote_mint;
        quote_to_base.output_mint = base_mint;
        quote_to_base.protocol_params = DexParamEnum::RaydiumCpmm(protocol_params);
        let instructions =
            RaydiumCpmmInstructionBuilder.build_buy_instructions(&quote_to_base).await.unwrap();
        let ix = instructions.last().unwrap();
        assert_eq!(ix.accounts[6].pubkey, quote_vault);
        assert_eq!(ix.accounts[7].pubkey, base_vault);
        assert_eq!(ix.accounts[8].pubkey, quote_program);
        assert_eq!(ix.accounts[9].pubkey, base_program);
        assert_eq!(ix.accounts[10].pubkey, quote_mint);
        assert_eq!(ix.accounts[11].pubkey, base_mint);
    }

    #[tokio::test]
    async fn raydium_cpmm_rejects_mints_outside_the_pool() {
        let mut params = swap_params(Some(1));
        params.output_mint = pk(99);
        let error =
            RaydiumCpmmInstructionBuilder.build_buy_instructions(&params).await.unwrap_err();
        assert!(error.to_string().contains("does not match Raydium CPMM pool"));
    }

    #[tokio::test]
    async fn current_stonkfun_graduated_pool_decodes_builds_and_simulates_swap() {
        if !crate::common::mainnet_sim::enabled() {
            return;
        }

        let rpc = crate::common::mainnet_sim::rpc_client();
        let wallet = crate::common::mainnet_sim::create_wallet();
        let pool = crate::common::mainnet_sim::fixtures::GRAD_POOL;
        let knots = crate::common::mainnet_sim::fixtures::GRAD_MEME_KNOTS;
        let stonk = crate::common::mainnet_sim::fixtures::GRAD_QUOTE_STONK;
        let sol_hop_pool = crate::common::mainnet_sim::fixtures::WSOL_STONK_CPMM;
        let token_2022 = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

        let protocol_params = RaydiumCpmmParams::from_pool_address_by_rpc(&rpc, &pool)
            .await
            .expect("decode current StonkFun graduated CPMM pool");
        assert_eq!(protocol_params.pool_state, pool);
        assert_eq!(protocol_params.base_mint, stonk);
        assert_eq!(protocol_params.quote_mint, knots);
        assert_eq!(protocol_params.base_token_program, crate::constants::TOKEN_PROGRAM);
        assert_eq!(protocol_params.quote_token_program, token_2022);

        // Acquire STONK via SOL hop, then STONK→KNOTS through StonkFunSwap, and simulate.
        let sol_hop = RaydiumCpmmParams::from_pool_address_by_rpc(&rpc, &sol_hop_pool)
            .await
            .expect("decode WSOL/STONK hop");
        let hop = crate::common::mainnet_sim::swap_params(
            wallet.clone(),
            crate::swqos::TradeType::Buy,
            crate::constants::WSOL_TOKEN_ACCOUNT,
            stonk,
            50_000,
            300,
            DexParamEnum::RaydiumCpmm(sol_hop),
        );
        let mut business = RaydiumCpmmInstructionBuilder.build_buy_instructions(&hop).await.unwrap();
        let stonk_min = {
            let swap = business
                .iter()
                .find(|ix| ix.program_id == accounts::RAYDIUM_CPMM)
                .unwrap();
            u64::from_le_bytes(swap.data[16..24].try_into().unwrap())
        };
        let mut meme_buy = crate::common::mainnet_sim::swap_params(
            wallet.clone(),
            crate::swqos::TradeType::Buy,
            stonk,
            knots,
            stonk_min,
            300,
            DexParamEnum::StonkFunSwap(protocol_params),
        );
        meme_buy.create_input_mint_ata = false;
        let meme_ixs = crate::instruction::stonkfun::StonkFunInstructionBuilder
            .build_buy_instructions(&meme_buy)
            .await
            .unwrap();
        business.extend(meme_ixs);

        let alt = crate::common::mainnet_sim::load_alt(
            &rpc,
            &crate::common::mainnet_sim::fixtures::WSOL_STONK_LUT,
        )
        .await;
        crate::common::mainnet_sim::run_business_sim(
            &rpc,
            &wallet,
            business,
            &[alt],
            "graduated decode+sim stonk→knots",
        )
        .await;
    }
}
