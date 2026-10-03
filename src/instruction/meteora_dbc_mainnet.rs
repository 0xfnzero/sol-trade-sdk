//! Mainnet simulation for Meteora Dynamic Bonding Curve pools: a buy and a
//! sale built from live pool state, with the amounts the simulation moved
//! checked against the quotes.
//!
//! Curve pools complete or die within hours, so the pools come from
//! `DBC_SIM_POOLS` (comma separated); a completed pool is skipped.
//! `DBC_SIM_REQUIRE=1` fails the test when a simulation fails, instead of
//! reporting it: a transfer hook may refuse wallets it does not know.

#![cfg(test)]

use std::sync::Arc;

use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{
    RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_message::{v1, VersionedMessage};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_instruction;
use solana_transaction_status_client_types::UiTransactionEncoding;

use crate::{
    common::{
        mainnet_sim::{self, fixtures},
        SolanaRpcClient,
    },
    instruction::{
        meteora_dbc::MeteoraDbcInstructionBuilder,
        utils::meteora_dbc::{accounts, fetch_pool},
    },
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, MeteoraDbcParams},
        traits::InstructionBuilder,
    },
};

fn pools() -> Vec<Pubkey> {
    std::env::var("DBC_SIM_POOLS")
        .unwrap_or_default()
        .split(',')
        .filter(|pool| !pool.trim().is_empty())
        .map(|pool| pool.trim().parse().expect("DBC_SIM_POOLS holds pool addresses"))
        .collect()
}

fn token_account(owner: &Pubkey, mint: &Pubkey, program: &Pubkey) -> Pubkey {
    crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
        owner, mint, program, false,
    )
}

struct Simulated {
    error: Option<String>,
    units: Option<u64>,
    logs: Vec<String>,
    /// Token amounts of the accounts asked for; `None` for a missing account.
    amounts: Vec<Option<u64>>,
}

async fn simulate(
    rpc: &SolanaRpcClient,
    wallet: &Keypair,
    business: Vec<solana_sdk::instruction::Instruction>,
    read: &[Pubkey],
) -> Simulated {
    let funder = mainnet_sim::pick_funder(rpc).await;
    let blockhash = rpc.get_latest_blockhash().await.expect("blockhash");
    // A v1 message, as SolBot sends routed trades: a hook pool's swap behind
    // a hop does not fit a legacy one.
    let mut instructions = vec![system_instruction::transfer(
        &funder,
        &wallet.pubkey(),
        mainnet_sim::SIM_FUND_LAMPORTS,
    )];
    instructions.extend(business);
    let config = v1::TransactionConfig::empty()
        .with_compute_unit_limit(1_400_000)
        .with_loaded_accounts_data_size_limit(64 * 1024 * 1024);
    let message = v1::Message::try_compile_with_config(&funder, &instructions, blockhash, config)
        .expect("compile simulation message");
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); message.header.num_required_signatures as usize],
        message: VersionedMessage::V1(message),
    };
    let response = rpc
        .simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig { commitment: CommitmentLevel::Processed }),
                encoding: Some(UiTransactionEncoding::Base64),
                accounts: Some(RpcSimulateTransactionAccountsConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    addresses: read.iter().map(Pubkey::to_string).collect(),
                }),
                min_context_slot: None,
                inner_instructions: false,
            },
        )
        .await;
    let response = match response {
        Ok(response) => response,
        Err(err) => {
            return Simulated {
                error: Some(format!("simulateTransaction: {err}")),
                units: None,
                logs: Vec::new(),
                amounts: Vec::new(),
            }
        }
    };
    let amounts = response
        .value
        .accounts
        .unwrap_or_default()
        .into_iter()
        .map(|account| {
            let data = account?.data.decode()?;
            Some(u64::from_le_bytes(data.get(64..72)?.try_into().ok()?))
        })
        .collect();
    Simulated {
        error: response.value.err.map(|err| format!("{err:?}")),
        units: response.value.units_consumed,
        logs: response.value.logs.unwrap_or_default(),
        amounts,
    }
}

/// A SOL to USDC swap through a Meteora DAMM v2 pool, with the USDC it
/// buys at least: few accounts, so a hook pool's swap still fits the
/// simulation's size limit behind it.
async fn sol_to_usdc_hop(
    rpc: &SolanaRpcClient,
    wallet: Arc<Keypair>,
    lamports: u64,
) -> Option<(Vec<solana_sdk::instruction::Instruction>, u64)> {
    let pool = mainnet_sim::load_meteora_damm_v2(rpc, &fixtures::METEORA_DAMM_V2_SOL_USDC).await?;
    let hop = mainnet_sim::swap_params(
        wallet,
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        lamports,
        500,
        DexParamEnum::MeteoraDammV2(pool.with_rate_limiter_sysvar(true)),
    );
    let instructions = crate::instruction::meteora_damm_v2::MeteoraDammV2InstructionBuilder
        .build_buy_instructions(&hop)
        .await
        .ok()?;
    let swap = instructions.iter().find(|ix| {
        ix.program_id == crate::instruction::utils::meteora_damm_v2::accounts::METEORA_DAMM_V2
    })?;
    let minimum_out = u64::from_le_bytes(swap.data.get(16..24)?.try_into().ok()?);
    (minimum_out > 0).then_some((instructions, minimum_out))
}

/// One pool: a buy alone, then a buy followed by a sale of half of it.
/// Returns what failed, if anything.
async fn simulate_pool(rpc: &SolanaRpcClient, pool_address: Pubkey) -> Result<(), String> {
    let state = fetch_pool(rpc, &pool_address).await.map_err(|err| err.to_string())?;
    let pool = MeteoraDbcParams::from_pool_by_rpc(rpc, &pool_address, &state)
        .await
        .map_err(|err| err.to_string())?;
    let curve = pool.quote.clone().expect("loaded with its curve");
    println!(
        "pool {pool_address} base {} quote {} hook {} fee {} reserve {}/{}",
        pool.base_mint,
        pool.quote_mint,
        pool.transfer_hook.is_some(),
        curve.fee_numerator,
        state.quote_reserve,
        curve.config.migration_quote_threshold,
    );
    if state.is_migrated || state.quote_reserve >= curve.config.migration_quote_threshold {
        println!("  skipped: the curve is complete");
        return Ok(());
    }

    let wallet = Arc::new(Keypair::new());
    let owner = wallet.pubkey();
    let base_account = token_account(&owner, &pool.base_mint, &pool.base_token_program);
    let quote_account = token_account(&owner, &pool.quote_mint, &pool.quote_token_program);

    // Pay 0.01 SOL: wrapped for a pool priced in SOL, swapped to USDC first
    // for one priced in USDC.
    let (mut business, quote_in, wraps) = if pool.quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT
    {
        (Vec::new(), 10_000_000u64, true)
    } else if pool.quote_mint == fixtures::USDC_MINT {
        let Some((hop, usdc)) = sol_to_usdc_hop(rpc, wallet.clone(), 10_000_000).await else {
            println!("  skipped: no SOL to USDC hop");
            return Ok(());
        };
        (hop, usdc, false)
    } else {
        println!("  skipped: no route to quote {}", pool.quote_mint);
        return Ok(());
    };

    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        pool.quote_mint,
        pool.base_mint,
        quote_in,
        500,
        DexParamEnum::MeteoraDbc(pool.clone()),
    );
    buy.create_input_mint_ata = wraps;
    let buy_quote = curve.quote_exact_in(true, quote_in).map_err(|err| err.to_string())?;
    let buy_ixs = MeteoraDbcInstructionBuilder
        .build_buy_instructions(&buy)
        .await
        .map_err(|err| err.to_string())?;
    business.extend(buy_ixs);

    let bought = simulate(rpc, &wallet, business.clone(), &[base_account]).await;
    println!(
        "  buy: err={:?} cu={:?} received={:?} quoted={}",
        bought.error, bought.units, bought.amounts, buy_quote.amount_out
    );
    if let Some(error) = bought.error {
        mainnet_sim::print_tail_logs(&bought.logs, 12);
        return Err(format!("buy failed: {error}"));
    }
    let received = bought.amounts.first().copied().flatten().ok_or("buy left no base account")?;
    if received != buy_quote.amount_out {
        return Err(format!("buy received {received}, quoted {}", buy_quote.amount_out));
    }

    // Sell half of it in the same transaction, quoted on the curve as the buy
    // leaves it.
    let sell_amount = received / 2;
    let mut after_buy = pool.clone();
    let mut curve_after = curve.clone();
    curve_after.sqrt_price = buy_quote.next_sqrt_price;
    after_buy.quote = Some(curve_after.clone());
    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        pool.base_mint,
        pool.quote_mint,
        sell_amount,
        500,
        DexParamEnum::MeteoraDbc(after_buy),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    let sell_quote =
        curve_after.quote_exact_in(false, sell_amount).map_err(|err| err.to_string())?;
    business.extend(
        MeteoraDbcInstructionBuilder
            .build_sell_instructions(&sell)
            .await
            .map_err(|err| err.to_string())?,
    );
    let sold = simulate(rpc, &wallet, business, &[base_account, quote_account]).await;
    println!(
        "  buy+sell: err={:?} cu={:?} accounts={:?} sale quoted={}",
        sold.error, sold.units, sold.amounts, sell_quote.amount_out
    );
    if let Some(error) = sold.error {
        mainnet_sim::print_tail_logs(&sold.logs, 12);
        return Err(format!("sale failed: {error}"));
    }
    let base_left = sold.amounts.first().copied().flatten().ok_or("sale left no base account")?;
    if base_left != received - sell_amount {
        return Err(format!("sale left {base_left} of {received} after selling {sell_amount}"));
    }
    if wraps {
        // The wrapped account holds what the buy did not take plus the sale.
        let quote_left =
            sold.amounts.get(1).copied().flatten().ok_or("sale left no quote account")?;
        let expected = quote_in - buy_quote.amount_in + sell_quote.amount_out;
        if quote_left != expected {
            return Err(format!("sale left {quote_left} quote, expected {expected}"));
        }
    }
    Ok(())
}

#[tokio::test]
async fn meteora_dbc_mainnet_simulates_buy_and_sale_matching_its_quotes() {
    if !mainnet_sim::enabled() {
        return;
    }
    assert_eq!(accounts::METEORA_DBC.to_string(), "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN");
    let rpc = mainnet_sim::rpc_client();
    let mut failures = Vec::new();
    for pool in pools() {
        if let Err(failure) = simulate_pool(&rpc, pool).await {
            println!("  FAILED: {failure}");
            failures.push(format!("{pool}: {failure}"));
        }
    }
    println!("failures: {failures:#?}");
    if std::env::var("DBC_SIM_REQUIRE").as_deref() == Ok("1") {
        assert!(failures.is_empty(), "{failures:#?}");
    }
}

/// A buy paid in SOL through the route of a pool priced in another token:
/// `DBC_SIM_ROUTED_POOL` is the DBC pool, `DBC_SIM_QUOTE_HOP_CLMM` a Raydium
/// CLMM pool trading USDC against its quote. SOL goes to USDC, to the quote,
/// then into the curve, in one transaction.
#[tokio::test]
async fn meteora_dbc_mainnet_simulates_a_buy_through_a_sol_route() {
    if !mainnet_sim::enabled() {
        return;
    }
    let (Ok(pool_address), Ok(quote_hop)) =
        (std::env::var("DBC_SIM_ROUTED_POOL"), std::env::var("DBC_SIM_QUOTE_HOP_CLMM"))
    else {
        return;
    };
    let pool_address: Pubkey = pool_address.parse().expect("DBC_SIM_ROUTED_POOL");
    let quote_hop: Pubkey = quote_hop.parse().expect("DBC_SIM_QUOTE_HOP_CLMM");

    let rpc = mainnet_sim::rpc_client();
    let pool = MeteoraDbcParams::from_pool_address_by_rpc(&rpc, &pool_address)
        .await
        .expect("load the DBC pool");
    let curve = pool.quote.clone().expect("loaded with its curve");
    let Some(sol_hop) = mainnet_sim::load_amm_v4(&rpc, fixtures::AMM_V4_WSOL_USDC).await else {
        return;
    };
    let Some(quote_hop) =
        mainnet_sim::load_raydium_clmm(&rpc, &quote_hop, &fixtures::USDC_MINT, &pool.quote_mint)
            .await
    else {
        return;
    };

    let wallet = Arc::new(Keypair::new());
    let base_account = token_account(&wallet.pubkey(), &pool.base_mint, &pool.base_token_program);
    let via = crate::StonkFunViaSolParams::meteora_dbc(pool.clone(), sol_hop)
        .with_quote_hop(quote_hop)
        .with_hop_slippage_basis_points(100);
    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        pool.base_mint,
        10_000_000,
        500,
        DexParamEnum::StonkFunViaSol(via),
    );
    params.close_input_mint_ata = true;
    let business = crate::instruction::stonkfun::StonkFunInstructionBuilder
        .build_buy_instructions(&params)
        .await
        .expect("build the routed buy");
    let swap =
        business.iter().find(|ix| ix.program_id == accounts::METEORA_DBC).expect("the curve leg");
    // The curve leg spends the last hop's minimum output.
    let quote_in = u64::from_le_bytes(swap.data[8..16].try_into().unwrap());
    let quoted = curve.quote_exact_in(true, quote_in).expect("quote the curve leg");

    let result = simulate(&rpc, &wallet, business, &[base_account]).await;
    println!(
        "routed buy of {pool_address}: err={:?} cu={:?} quote_in={quote_in} received={:?} quoted={}",
        result.error, result.units, result.amounts, quoted.amount_out
    );
    if result.error.is_some() {
        mainnet_sim::print_tail_logs(&result.logs, 16);
    }
    assert_eq!(result.error, None);
    assert_eq!(result.amounts, vec![Some(quoted.amount_out)]);
}

/// The DAMM v2 pools DBC curves migrate to (`DAMM_V2_SIM_POOLS`, comma
/// separated; token against WSOL or USDC): a buy, then a sale of half of it,
/// with the minimum outputs quoted from the pool's state and the amounts the
/// simulation moved checked against the quotes.
#[tokio::test]
async fn meteora_damm_v2_mainnet_buy_and_sale_match_their_quotes() {
    use crate::instruction::meteora_damm_v2::MeteoraDammV2InstructionBuilder;
    use crate::utils::calc::meteora_damm_v2::quote_exact_in;

    if !mainnet_sim::enabled() {
        return;
    }
    let rpc = mainnet_sim::rpc_client();
    let pools: Vec<Pubkey> = std::env::var("DAMM_V2_SIM_POOLS")
        .unwrap_or_default()
        .split(',')
        .filter(|pool| !pool.trim().is_empty())
        .map(|pool| pool.trim().parse().expect("DAMM_V2_SIM_POOLS holds pool addresses"))
        .collect();
    for pool_address in pools {
        let Some(pool) = mainnet_sim::load_meteora_damm_v2(&rpc, &pool_address).await else {
            continue;
        };
        let state = pool.quote.expect("loaded with its state");
        let is_currency = |mint: &Pubkey| {
            *mint == crate::constants::WSOL_TOKEN_ACCOUNT || *mint == fixtures::USDC_MINT
        };
        // The token is the side that is not WSOL or USDC.
        let token_is_a = !is_currency(&pool.token_a_mint);
        let (token, token_program, quote, quote_program) = if token_is_a {
            (pool.token_a_mint, pool.token_a_program, pool.token_b_mint, pool.token_b_program)
        } else {
            (pool.token_b_mint, pool.token_b_program, pool.token_a_mint, pool.token_a_program)
        };
        println!(
            "damm v2 pool {pool_address} token {token} quote {quote} fee {}",
            state.fee_numerator
        );

        let wallet = Arc::new(Keypair::new());
        let owner = wallet.pubkey();
        let token_account_key = token_account(&owner, &token, &token_program);
        let quote_account_key = token_account(&owner, &quote, &quote_program);
        let wraps = quote == crate::constants::WSOL_TOKEN_ACCOUNT;
        let (mut business, quote_in) = if wraps {
            (Vec::new(), 10_000_000u64)
        } else {
            let Some((hop, usdc)) = sol_to_usdc_hop(&rpc, wallet.clone(), 10_000_000).await else {
                continue;
            };
            (hop, usdc)
        };

        let mut buy = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Buy,
            quote,
            token,
            quote_in,
            500,
            DexParamEnum::MeteoraDammV2(pool.clone()),
        );
        buy.create_input_mint_ata = wraps;
        // Buying the token spends the other side: B when the token is A.
        let buy_quote = quote_exact_in(&state, !token_is_a, quote_in).expect("quote the buy");
        business
            .extend(MeteoraDammV2InstructionBuilder.build_buy_instructions(&buy).await.unwrap());

        let bought = simulate(&rpc, &wallet, business.clone(), &[token_account_key]).await;
        println!(
            "  buy: err={:?} cu={:?} received={:?} quoted={}",
            bought.error, bought.units, bought.amounts, buy_quote.amount_out
        );
        if bought.error.is_some() {
            mainnet_sim::print_tail_logs(&bought.logs, 12);
        }
        assert_eq!(bought.error, None);
        assert_eq!(bought.amounts, vec![Some(buy_quote.amount_out)]);

        let sell_amount = buy_quote.amount_out / 2;
        let mut state_after = state;
        state_after.sqrt_price = buy_quote.next_sqrt_price;
        let mut sell = mainnet_sim::swap_params(
            wallet.clone(),
            TradeType::Sell,
            token,
            quote,
            sell_amount,
            500,
            DexParamEnum::MeteoraDammV2(pool.clone().with_quote(state_after)),
        );
        sell.create_input_mint_ata = false;
        sell.create_output_mint_ata = false;
        let sell_quote =
            quote_exact_in(&state_after, token_is_a, sell_amount).expect("quote the sale");
        business
            .extend(MeteoraDammV2InstructionBuilder.build_sell_instructions(&sell).await.unwrap());
        let sold = simulate(&rpc, &wallet, business, &[token_account_key, quote_account_key]).await;
        println!(
            "  buy+sell: err={:?} cu={:?} accounts={:?} sale quoted={}",
            sold.error, sold.units, sold.amounts, sell_quote.amount_out
        );
        if sold.error.is_some() {
            mainnet_sim::print_tail_logs(&sold.logs, 12);
        }
        assert_eq!(sold.error, None);
        assert_eq!(sold.amounts[0], Some(buy_quote.amount_out - sell_amount));
        if wraps {
            assert_eq!(sold.amounts[1], Some(sell_quote.amount_out));
        }
    }
}

/// Migrated DBC pools (`DBC_SIM_MIGRATED_POOLS`, comma separated): the DAMM v2
/// pool derived from each one's config exists and pairs its base token, as
/// token A, with its quote.
#[tokio::test]
async fn meteora_dbc_mainnet_migrated_pools_derive_their_damm_v2_pool() {
    use crate::instruction::utils::meteora_dbc::{fetch_config_cached, get_migrated_damm_v2_pool};

    if !mainnet_sim::enabled() {
        return;
    }
    let rpc = mainnet_sim::rpc_client();
    let pools: Vec<Pubkey> = std::env::var("DBC_SIM_MIGRATED_POOLS")
        .unwrap_or_default()
        .split(',')
        .filter(|pool| !pool.trim().is_empty())
        .map(|pool| pool.trim().parse().expect("DBC_SIM_MIGRATED_POOLS holds pool addresses"))
        .collect();
    for pool_address in pools {
        let pool = fetch_pool(&rpc, &pool_address).await.expect("load the DBC pool");
        assert!(pool.is_migrated, "{pool_address} has not migrated");
        let config = fetch_config_cached(&rpc, &pool.config).await.expect("load its config");
        let damm = get_migrated_damm_v2_pool(&config, &pool.base_mint)
            .expect("the config migrates to DAMM v2");
        let damm_pool = crate::instruction::utils::meteora_damm_v2::fetch_pool(&rpc, &damm)
            .await
            .expect("the derived DAMM v2 pool exists");
        println!(
            "{pool_address} (fee option {}) -> {damm} {}/{}",
            config.migration_fee_option, damm_pool.token_a_mint, damm_pool.token_b_mint
        );
        assert_eq!(damm_pool.token_a_mint, pool.base_mint);
        assert_eq!(damm_pool.token_b_mint, config.quote_mint);
    }
}
