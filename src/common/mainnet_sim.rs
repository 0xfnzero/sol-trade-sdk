//! Shared helpers for gated mainnet `simulateTransaction` tests.
//!
//! Pattern:
//! 1. `Keypair::new()` ephemeral wallet
//! 2. Virtually fund it from a high-balance mainnet account (`sigVerify=false`)
//! 3. Build real DEX instructions from live pool state
//! 4. `simulateTransaction` — never submit

#![cfg(test)]

use crate::{
    common::{address_lookup::fetch_address_lookup_table_account, GasFeeStrategy, SolanaRpcClient},
    swqos::TradeType,
    trading::core::{
        params::{DexParamEnum, SwapParams},
        traits::InstructionBuilder,
    },
};
use solana_client::rpc_config::RpcSimulateTransactionConfig;
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_message::{v0, AddressLookupTableAccount, VersionedMessage};
use solana_sdk::{
    hash::Hash,
    instruction::Instruction,
    pubkey,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_instruction;
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::sync::Arc;

/// Known high-balance mainnet accounts used only as simulation fee-payer / funder.
pub const SIM_FUNDER_CANDIDATES: [Pubkey; 4] = [
    pubkey!("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM"),
    pubkey!("5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9"),
    pubkey!("2ojv9BAiHUrvsm9gxDeBFNCuoK1xKdcHGa5Y6XD7Tcuv"),
    pubkey!("FWznbcNXWQuHTawe9RxvQ2LdCENssh12dsznf4RiouN5"),
];

pub const SIM_FUND_LAMPORTS: u64 = 100_000_000; // 0.1 SOL virtual fund

/// Shared fixtures for gated mainnet simulation tests.
pub mod fixtures {
    use solana_sdk::{pubkey, pubkey::Pubkey};

    // --- StonkFun / stock-quote CPMM ---
    pub const CURVE_POOL: Pubkey = pubkey!("84XZdJNyBVVBqGe3BHY8n6x1jbcnxNWA5x4GetwQsjgp");
    pub const CURVE_MEME: Pubkey = pubkey!("BJ56gcrMNKDzVwjQXKToya9cAcMZvN9pz6ZzUejxQary");
    pub const CURVE_QUOTE_CARDS: Pubkey = pubkey!("CARDSccUMFKoPRZxt5vt3ksUbxEFEcnZ3H2pd3dKxYjp");
    pub const WSOL_CARDS_CPMM: Pubkey = pubkey!("3kMBV4dFBLoAaNFBcXcnaY2sY6k4k45Pcuo2zXSJHXQx");
    pub const WSOL_CARDS_LUT: Pubkey = pubkey!("2gyDuEaj39reuVjrQxGtFsK7625MbA9qDzNmHZUibrN2");

    pub const GRAD_POOL: Pubkey = pubkey!("BUVzsLLLG7GWoyJVoU31pXiBveazA6GXTavZ9VD3CwS9");
    pub const GRAD_MEME_KNOTS: Pubkey = pubkey!("8RVBk8vxLiUHueLUW1f4izFVqN3nWippLhkohKg6EGkS");
    pub const GRAD_QUOTE_STONK: Pubkey = pubkey!("6GmAFSYs4gk3FDao5FzzySQpPZaWsa4rUJHacpMpUNgx");
    pub const WSOL_STONK_CPMM: Pubkey = pubkey!("EKPjNvowpSFPaZcroeUcgAPtdUTZZrP3v8sCKKyfpe5x");
    pub const WSOL_STONK_LUT: Pubkey = pubkey!("8vp6JD2W19rM6Vbs3cBRCuQ3nXc8aykRMRrMEA6zoigR");

    // --- PumpSwap (WSOL quote) from examples/pumpswap_direct_trading ---
    pub const PUMPSWAP_POOL: Pubkey = pubkey!("539m4mVWt6iduB6W8rDGPMarzNCMesuqY5eUTiiYHAgR");
    pub const PUMPSWAP_BASE: Pubkey = pubkey!("pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn");

    // --- PumpSwap (WSOL quote) from examples/seed_trading ---
    pub const PUMPSWAP_SEED_POOL: Pubkey =
        pubkey!("9qKxzRejsV6Bp2zkefXWCbGvg61c3hHei7ShXJ4FythA");
    pub const PUMPSWAP_SEED_BASE: Pubkey =
        pubkey!("2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv");

    // --- PumpSwap PUMP/USDC ---
    pub const PUMPSWAP_USDC_POOL: Pubkey =
        pubkey!("2uF4Xh61rDwxnG9woyxsVQP7zuA6kLFpb3NvnRQeoiSd");

    // --- Raydium AMM v4 WSOL pairs ---
    pub const AMM_V4_WSOL_USDT: Pubkey = pubkey!("7XawhbbxtsRcQA8KTkHT9f9nc6d69UwqCDh6U5EEbEmX");
    pub const USDT_MINT: Pubkey = pubkey!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");
    /// High-liquidity WSOL/USDC AMM v4 (Raydium API id).
    pub const AMM_V4_WSOL_USDC: Pubkey = pubkey!("58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2");
    pub const USDC_MINT: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

    // --- Meteora DAMM v2 USDC pool ---
    pub const METEORA_DAMM_V2_POOL: Pubkey =
        pubkey!("7dVri3qjYD3uobSZL3Zth8vSCgU6r6R2nvFsh7uVfDte");
    pub const METEORA_DAMM_V2_TOKEN: Pubkey =
        pubkey!("PRVT6TB7uss3FrUd2D9xs2zqDBsa3GbMJMwCQsgmeta");
    /// Meteora DAMM v2 SOL/USDC (direct WSOL quote — no hop required).
    pub const METEORA_DAMM_V2_SOL_USDC: Pubkey =
        pubkey!("8Pm2kZpnxD3hoMmt4bjStX2Pw2Z9abpbHzZxMPqxPmie");

    // --- Concentrated liquidity (SOL/USDC) ---
    /// Raydium CLMM SOL/USDC (highest TVL on Raydium API).
    pub const RAYDIUM_CLMM_SOL_USDC: Pubkey =
        pubkey!("3ucNos4NbumPLZNWztqGHNFFgkHeRMBQAVemeeomsUxv");
    /// Raydium CLMM SOL/USDT (extra venue coverage).
    pub const RAYDIUM_CLMM_SOL_USDT: Pubkey =
        pubkey!("3nMFwZXwY1s1M5s8vYAHqd4wGs4iSxXE4LRoUMMYqEgF");
    /// Orca Whirlpool SOL/USDC.
    pub const ORCA_WHIRLPOOL_SOL_USDC: Pubkey =
        pubkey!("HJPjoWUrhoZzkNfRpHuieeFk9WcZWjwy6PBjZ81ngndJ");
    /// Orca Whirlpool SOL/USDT.
    pub const ORCA_WHIRLPOOL_SOL_USDT: Pubkey =
        pubkey!("FwewVm8u6tFPGewAyHmWAqad9hmF7mvqxK4mJ7iNqqGC");
    /// Meteora DLMM SOL/USDC.
    pub const METEORA_DLMM_SOL_USDC: Pubkey =
        pubkey!("BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y");
    /// Meteora DLMM SOL/USDC (high-TVL alternate venue).
    pub const METEORA_DLMM_SOL_USDC_ALT: Pubkey =
        pubkey!("5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6");
}

#[derive(Debug)]
pub struct SimResult {
    pub ok: bool,
    pub units_consumed: Option<u64>,
    pub err: Option<String>,
    pub logs: Vec<String>,
}

pub fn enabled() -> bool {
    std::env::var("RUN_MAINNET_TESTS").as_deref() == Ok("1")
}

pub fn rpc_url() -> String {
    std::env::var("SOLANA_RPC_URL")
        .or_else(|_| std::env::var("RPC_URL"))
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_owned())
}

pub fn rpc_client() -> SolanaRpcClient {
    SolanaRpcClient::new_with_commitment(rpc_url(), CommitmentConfig::processed())
}

pub fn create_wallet() -> Arc<Keypair> {
    let wallet = Arc::new(Keypair::new());
    println!("created test wallet={}", wallet.pubkey());
    wallet
}

fn is_transient_rpc_error(err: &impl std::fmt::Display) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("error sending request")
        || msg.contains("429")
        || msg.contains("403")
        || msg.contains("forbidden")
        || msg.contains("rate limit")
        || msg.contains("timeout")
        || msg.contains("temporar")
        || msg.contains("connection reset")
        || msg.contains("broken pipe")
        || msg.contains("cloudflare")
        || msg.contains("502")
        || msg.contains("503")
        || msg.contains("504")
        // PublicNode/Allnodes free tier gates some methods behind a personal token.
        || msg.contains("personal token")
        || msg.contains("indexed requests")
}

/// Retry transient public-RPC failures (rate limits / transport).
pub async fn rpc_retry<T, E, F, Fut>(label: &str, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_err: Option<E> = None;
    for attempt in 0..6u32 {
        match f().await {
            Ok(v) => return Ok(v),
            Err(err) => {
                let transient = is_transient_rpc_error(&err);
                println!("{label} rpc attempt={} err={err}", attempt + 1);
                if !transient {
                    return Err(err);
                }
                last_err = Some(err);
                let backoff_ms = 400u64.saturating_mul(1u64 << attempt.min(4));
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            }
        }
    }
    Err(last_err.expect("rpc_retry exhausted without error"))
}

/// Like [`rpc_retry`], but skip the test on exhausted transient failures.
#[macro_export]
macro_rules! mainnet_rpc_or_skip {
    ($label:expr, $expr:expr) => {{
        match $crate::common::mainnet_sim::rpc_retry($label, || async { $expr }).await {
            Ok(v) => v,
            Err(err) => {
                if $crate::common::mainnet_sim::is_transient_rpc_error_pub(&err) {
                    println!("skip {}: transient RPC failure after retries: {err}", $label);
                    return;
                }
                panic!("{} failed: {err}", $label);
            }
        }
    }};
}

pub fn is_transient_rpc_error_pub(err: &impl std::fmt::Display) -> bool {
    is_transient_rpc_error(err)
}

pub async fn pick_funder(rpc: &SolanaRpcClient) -> Pubkey {
    for candidate in SIM_FUNDER_CANDIDATES {
        match rpc.get_balance(&candidate).await {
            Ok(lamports) if lamports >= SIM_FUND_LAMPORTS * 10 => {
                println!("simulation funder={candidate} lamports={lamports}");
                return candidate;
            }
            Ok(lamports) => println!("skip funder {candidate}: lamports={lamports}"),
            Err(err) => println!("skip funder {candidate}: {err}"),
        }
    }
    panic!("could not find a mainnet account with enough SOL to fund simulation");
}

pub async fn load_alt(rpc: &SolanaRpcClient, key: &Pubkey) -> AddressLookupTableAccount {
    fetch_address_lookup_table_account(rpc, key).await.expect("fetch address lookup table")
}

pub fn build_sim_tx(
    funder: Pubkey,
    wallet: &Keypair,
    business_instructions: Vec<Instruction>,
    recent_blockhash: Hash,
    lookup_tables: &[AddressLookupTableAccount],
) -> VersionedTransaction {
    // Only CU limit (no price) to leave room for fat multi-hop txs under the
    // simulateTransaction base64 size cap (~1644 encoded bytes).
    let mut instructions = Vec::with_capacity(business_instructions.len() + 2);
    instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(1_400_000));
    instructions.push(system_instruction::transfer(
        &funder,
        &wallet.pubkey(),
        SIM_FUND_LAMPORTS,
    ));
    instructions.extend(dedupe_create_ata_ixs(business_instructions));

    let message =
        v0::Message::try_compile(&funder, &instructions, lookup_tables, recent_blockhash)
            .expect("compile simulation message");
    println!(
        "simulation alts={} static_keys={}",
        lookup_tables.len(),
        message.account_keys.len()
    );
    VersionedTransaction {
        signatures: vec![Signature::default(); message.header.num_required_signatures as usize],
        message: VersionedMessage::V0(message),
    }
}

/// Drop duplicate idempotent `CreateIdempotent` ATA ixs (same accounts) that
/// appear when buy+sell legs are concatenated.
fn dedupe_create_ata_ixs(instructions: Vec<Instruction>) -> Vec<Instruction> {
    let ata_program = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
    let mut seen = std::collections::HashSet::new();
    instructions
        .into_iter()
        .filter(|ix| {
            if ix.program_id != ata_program {
                return true;
            }
            let key = ix.accounts.iter().map(|a| a.pubkey).collect::<Vec<_>>();
            seen.insert(key)
        })
        .collect()
}

pub async fn simulate(rpc: &SolanaRpcClient, tx: &VersionedTransaction) -> SimResult {
    let result = match rpc
        .simulate_transaction_with_config(
            tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig { commitment: CommitmentLevel::Processed }),
                encoding: Some(UiTransactionEncoding::Base64),
                accounts: None,
                min_context_slot: None,
                inner_instructions: true,
            },
        )
        .await
    {
        Ok(r) => r,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("too large") {
                panic!(
                    "simulateTransaction rejected oversized tx ({msg}). \
                     Shrink ixs, add an ALT, or split buy/sell across sims."
                );
            }
            panic!("simulateTransaction RPC call: {err}");
        }
    };

    SimResult {
        ok: result.value.err.is_none(),
        units_consumed: result.value.units_consumed,
        err: result.value.err.map(|e| format!("{e:?}")),
        logs: result.value.logs.unwrap_or_default(),
    }
}

pub fn print_tail_logs(logs: &[String], n: usize) {
    for log in logs.iter().rev().take(n).collect::<Vec<_>>().into_iter().rev() {
        println!("  log: {log}");
    }
}

pub fn assert_sim_ok(label: &str, result: &SimResult) {
    println!(
        "{label} simulate ok={} cu={:?} err={:?}",
        result.ok, result.units_consumed, result.err
    );
    print_tail_logs(&result.logs, 14);
    assert!(result.ok, "{label} mainnet simulation must succeed: {:?}", result.err);
}

/// Convenience: fund + build + simulate in one shot.
pub async fn run_business_sim(
    rpc: &SolanaRpcClient,
    wallet: &Keypair,
    business: Vec<Instruction>,
    lookup_tables: &[AddressLookupTableAccount],
    label: &str,
) {
    let funder = pick_funder(rpc).await;
    let blockhash = match rpc_retry("blockhash", || rpc.get_latest_blockhash()).await {
        Ok(v) => v,
        Err(err) => {
            if is_transient_rpc_error(&err) {
                println!("skip {label}: transient RPC failure fetching blockhash: {err}");
                return;
            }
            panic!("{label} blockhash: {err}");
        }
    };
    let tx = build_sim_tx(funder, wallet, business, blockhash, lookup_tables);
    let result = match rpc_retry("simulate", || async {
        // simulate() panics on oversized; call raw path with Result for retry of transport only.
        rpc.simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig { commitment: CommitmentLevel::Processed }),
                encoding: Some(UiTransactionEncoding::Base64),
                accounts: None,
                min_context_slot: None,
                inner_instructions: true,
            },
        )
        .await
    })
    .await
    {
        Ok(r) => SimResult {
            ok: r.value.err.is_none(),
            units_consumed: r.value.units_consumed,
            err: r.value.err.map(|e| format!("{e:?}")),
            logs: r.value.logs.unwrap_or_default(),
        },
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("too large") {
                panic!(
                    "simulateTransaction rejected oversized tx ({msg}). \
                     Shrink ixs, add an ALT, or split buy/sell across sims."
                );
            }
            if is_transient_rpc_error(&err) {
                println!("skip {label}: transient RPC failure during simulate: {err}");
                return;
            }
            panic!("{label} simulate RPC: {err}");
        }
    };
    assert_sim_ok(label, &result);
}

/// Like [`run_business_sim`], but soft-skips when the compiled message exceeds the
/// `simulateTransaction` size cap (common for multi-DEX hops without an ALT).
pub async fn run_business_sim_skip_oversized(
    rpc: &SolanaRpcClient,
    wallet: &Keypair,
    business: Vec<Instruction>,
    lookup_tables: &[AddressLookupTableAccount],
    label: &str,
) {
    let funder = pick_funder(rpc).await;
    let blockhash = match rpc_retry("blockhash", || rpc.get_latest_blockhash()).await {
        Ok(v) => v,
        Err(err) => {
            if is_transient_rpc_error(&err) {
                println!("skip {label}: transient RPC failure fetching blockhash: {err}");
                return;
            }
            panic!("{label} blockhash: {err}");
        }
    };
    let tx = build_sim_tx(funder, wallet, business, blockhash, lookup_tables);
    let result = match rpc_retry("simulate", || async {
        rpc.simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig { commitment: CommitmentLevel::Processed }),
                encoding: Some(UiTransactionEncoding::Base64),
                accounts: None,
                min_context_slot: None,
                inner_instructions: true,
            },
        )
        .await
    })
    .await
    {
        Ok(r) => SimResult {
            ok: r.value.err.is_none(),
            units_consumed: r.value.units_consumed,
            err: r.value.err.map(|e| format!("{e:?}")),
            logs: r.value.logs.unwrap_or_default(),
        },
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("too large") {
                println!("skip {label}: simulate tx oversized ({msg})");
                return;
            }
            if is_transient_rpc_error(&err) {
                println!("skip {label}: transient RPC failure during simulate: {err}");
                return;
            }
            panic!("{label} simulate RPC: {err}");
        }
    };
    assert_sim_ok(label, &result);
}

/// Load helpers with retry + soft-skip on public RPC rate limits.
pub async fn load_cpmm(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
) -> Option<crate::trading::core::params::RaydiumCpmmParams> {
    match rpc_retry("load_cpmm", || {
        crate::trading::core::params::RaydiumCpmmParams::from_pool_address_by_rpc(rpc, pool)
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_cpmm {pool}: {err}");
            None
        }
        Err(err) => panic!("load_cpmm {pool}: {err}"),
    }
}

pub async fn load_amm_v4(
    rpc: &SolanaRpcClient,
    amm: Pubkey,
) -> Option<crate::trading::core::params::RaydiumAmmV4Params> {
    match rpc_retry("load_amm_v4", || {
        crate::trading::core::params::RaydiumAmmV4Params::from_amm_address_by_rpc(rpc, amm)
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_amm_v4 {amm}: {err}");
            None
        }
        Err(err) => panic!("load_amm_v4 {amm}: {err}"),
    }
}

pub async fn load_pumpswap(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
) -> Option<crate::trading::core::params::PumpSwapParams> {
    match rpc_retry("load_pumpswap", || {
        crate::trading::core::params::PumpSwapParams::from_pool_address_by_rpc(rpc, pool)
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_pumpswap {pool}: {err}");
            None
        }
        Err(err) => panic!("load_pumpswap {pool}: {err}"),
    }
}

pub async fn load_pumpswap_by_mint(
    rpc: &SolanaRpcClient,
    mint: &Pubkey,
) -> Option<crate::trading::core::params::PumpSwapParams> {
    match rpc_retry("load_pumpswap_by_mint", || {
        crate::trading::core::params::PumpSwapParams::from_mint_by_rpc(rpc, mint)
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_pumpswap_by_mint {mint}: {err}");
            None
        }
        Err(err) => panic!("load_pumpswap_by_mint {mint}: {err}"),
    }
}

pub async fn load_stonkfun_curve(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
) -> Option<crate::trading::core::params::StonkFunParams> {
    match rpc_retry("load_stonkfun_curve", || {
        crate::trading::core::params::StonkFunParams::from_pool_by_rpc(rpc, pool)
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_stonkfun_curve {pool}: {err}");
            None
        }
        Err(err) => panic!("load_stonkfun_curve {pool}: {err}"),
    }
}

pub async fn load_meteora_damm_v2(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
) -> Option<crate::trading::core::params::MeteoraDammV2Params> {
    match rpc_retry("load_meteora_damm_v2", || {
        crate::trading::core::params::MeteoraDammV2Params::from_pool_address_by_rpc(rpc, pool)
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_meteora_damm_v2 {pool}: {err}");
            None
        }
        Err(err) => panic!("load_meteora_damm_v2 {pool}: {err}"),
    }
}

pub async fn load_raydium_clmm(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
) -> Option<crate::trading::core::params::RaydiumClmmParams> {
    match rpc_retry("load_raydium_clmm", || {
        crate::trading::core::params::RaydiumClmmParams::from_pool_address_by_rpc(
            rpc,
            pool,
            input_mint,
            output_mint,
        )
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_raydium_clmm {pool}: {err}");
            None
        }
        Err(err) => panic!("load_raydium_clmm {pool}: {err}"),
    }
}

pub async fn load_whirlpool(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
) -> Option<crate::trading::core::params::WhirlpoolParams> {
    match rpc_retry("load_whirlpool", || {
        crate::trading::core::params::WhirlpoolParams::from_pool_address_by_rpc(
            rpc,
            pool,
            input_mint,
            output_mint,
        )
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_whirlpool {pool}: {err}");
            None
        }
        Err(err) => panic!("load_whirlpool {pool}: {err}"),
    }
}

pub async fn load_meteora_dlmm(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
) -> Option<crate::trading::core::params::MeteoraDlmmParams> {
    match rpc_retry("load_meteora_dlmm", || {
        crate::trading::core::params::MeteoraDlmmParams::from_pool_address_by_rpc(
            rpc,
            pool,
            input_mint,
            output_mint,
        )
    })
    .await
    {
        Ok(v) => Some(v),
        Err(err) if is_transient_rpc_error(&err) => {
            println!("skip load_meteora_dlmm {pool}: {err}");
            None
        }
        Err(err) => panic!("load_meteora_dlmm {pool}: {err}"),
    }
}

/// Build a minimal `SwapParams` suitable for mainnet instruction builders.
pub fn swap_params(
    wallet: Arc<Keypair>,
    trade_type: TradeType,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
    slippage_bps: u64,
    protocol_params: DexParamEnum,
) -> SwapParams {
    SwapParams {
        rpc: None,
        payer: wallet,
        trade_type,
        input_mint,
        input_token_program: None,
        output_mint,
        output_token_program: None,
        input_amount: Some(amount),
        slippage_basis_points: Some(slippage_bps),
        address_lookup_table_accounts: Vec::new(),
        recent_blockhash: None,
        wait_tx_confirmed: false,
        protocol_params,
        open_seed_optimize: false,
        swqos_clients: Arc::new(Vec::new()),
        middleware_manager: None,
        durable_nonce: None,
        with_tip: false,
        create_input_mint_ata: true,
        close_input_mint_ata: false,
        create_output_mint_ata: true,
        close_output_mint_ata: false,
        fixed_output_amount: None,
        gas_fee_strategy: GasFeeStrategy::new(),
        simulate: true,
        log_enabled: true,
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

/// Pin PumpSwap fee recipients so buy+sell share fee ATAs (smaller combined txs).
pub fn pin_pumpswap_fees(
    pool: crate::trading::core::params::PumpSwapParams,
) -> crate::trading::core::params::PumpSwapParams {
    use crate::instruction::utils::pumpswap::{
        get_protocol_extra_fee_recipient_random, get_protocol_fee_recipient_random,
    };
    pool.with_fee_recipients(
        get_protocol_fee_recipient_random(),
        get_protocol_extra_fee_recipient_random(),
    )
}

/// Extract Raydium AMM v4 `swap_base_in_v2` minimum_amount_out from built ixs.
pub fn amm_v4_min_out(ixs: &[Instruction]) -> Option<u64> {
    use crate::instruction::utils::raydium_amm_v4::{
        accounts as amm_accounts, SWAP_BASE_IN_V2_DISCRIMINATOR,
    };
    ixs.iter()
        .find(|ix| {
            ix.program_id == amm_accounts::RAYDIUM_AMM_V4
                && ix.data.len() >= 17
                && &ix.data[..1] == SWAP_BASE_IN_V2_DISCRIMINATOR
        })
        .map(|ix| u64::from_le_bytes(ix.data[9..17].try_into().unwrap()))
}

/// Extract PumpSwap `buy_exact_quote_in` min_base_amount_out (bytes [16..24]).
pub fn pumpswap_buy_min_base_out(ixs: &[Instruction]) -> Option<u64> {
    use crate::instruction::utils::pumpswap::{accounts, BUY_EXACT_QUOTE_IN_DISCRIMINATOR};
    ixs.iter()
        .find(|ix| {
            ix.program_id == accounts::AMM_PROGRAM
                && ix.data.len() >= 24
                && ix.data[..8] == BUY_EXACT_QUOTE_IN_DISCRIMINATOR
        })
        .map(|ix| u64::from_le_bytes(ix.data[16..24].try_into().unwrap()))
        .or_else(|| {
            // Fallback: plain buy stores base_amount_out at [8..16].
            ixs.iter()
                .find(|ix| ix.program_id == accounts::AMM_PROGRAM && ix.data.len() >= 16)
                .map(|ix| u64::from_le_bytes(ix.data[8..16].try_into().unwrap()))
        })
}

/// Build SOL→USDC hop via Raydium AMM v4; returns (instructions, usdc_min_out).
pub async fn build_sol_to_usdc_hop(
    rpc: &SolanaRpcClient,
    wallet: Arc<Keypair>,
    sol_lamports: u64,
) -> Option<(Vec<Instruction>, u64)> {
    let pool = load_amm_v4(rpc, fixtures::AMM_V4_WSOL_USDC).await?;
    let hop = swap_params(
        wallet,
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        sol_lamports,
        500,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    let hop_ixs = crate::instruction::raydium_amm_v4::RaydiumAmmV4InstructionBuilder
        .build_buy_instructions(&hop)
        .await
        .ok()?;
    let usdc_min = amm_v4_min_out(&hop_ixs)?;
    if usdc_min == 0 {
        return None;
    }
    Some((hop_ixs, usdc_min))
}

/// Build SOL→USDT hop via Raydium AMM v4.
pub async fn build_sol_to_usdt_hop(
    rpc: &SolanaRpcClient,
    wallet: Arc<Keypair>,
    sol_lamports: u64,
) -> Option<(Vec<Instruction>, u64)> {
    let pool = load_amm_v4(rpc, fixtures::AMM_V4_WSOL_USDT).await?;
    let hop = swap_params(
        wallet,
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDT_MINT,
        sol_lamports,
        500,
        DexParamEnum::RaydiumAmmV4(pool),
    );
    let hop_ixs = crate::instruction::raydium_amm_v4::RaydiumAmmV4InstructionBuilder
        .build_buy_instructions(&hop)
        .await
        .ok()?;
    let usdt_min = amm_v4_min_out(&hop_ixs)?;
    if usdt_min == 0 {
        return None;
    }
    Some((hop_ixs, usdt_min))
}

/// Build SOL→USDC hop via Raydium CLMM (min_out floored to 1 for concentrated venues).
pub async fn build_sol_to_usdc_hop_clmm(
    rpc: &SolanaRpcClient,
    wallet: Arc<Keypair>,
    sol_lamports: u64,
) -> Option<(Vec<Instruction>, u64)> {
    let pool = load_raydium_clmm(
        rpc,
        &fixtures::RAYDIUM_CLMM_SOL_USDC,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &fixtures::USDC_MINT,
    )
    .await?;
    let mut hop = swap_params(
        wallet,
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        sol_lamports,
        800,
        DexParamEnum::RaydiumClmm(pool),
    );
    hop.fixed_output_amount = Some(1);
    let hop_ixs = crate::instruction::raydium_clmm::RaydiumClmmInstructionBuilder
        .build_buy_instructions(&hop)
        .await
        .ok()?;
    // CLMM sims use min_out=1; estimate a conservative spendable credit from input size.
    // Callers that need a precise fill should prefer [`build_sol_to_usdc_hop`].
    let estimated = (sol_lamports / 2_000).max(1); // ~rough µUSDC floor for tiny SOL sizes
    Some((hop_ixs, estimated))
}

/// Build SOL→USDC hop via Orca Whirlpool.
pub async fn build_sol_to_usdc_hop_whirlpool(
    rpc: &SolanaRpcClient,
    wallet: Arc<Keypair>,
    sol_lamports: u64,
) -> Option<(Vec<Instruction>, u64)> {
    let pool = load_whirlpool(
        rpc,
        &fixtures::ORCA_WHIRLPOOL_SOL_USDC,
        &crate::constants::WSOL_TOKEN_ACCOUNT,
        &fixtures::USDC_MINT,
    )
    .await?;
    let mut hop = swap_params(
        wallet,
        TradeType::Buy,
        crate::constants::WSOL_TOKEN_ACCOUNT,
        fixtures::USDC_MINT,
        sol_lamports,
        800,
        DexParamEnum::OrcaWhirlpool(pool),
    );
    hop.fixed_output_amount = Some(1);
    let hop_ixs = crate::instruction::whirlpool::WhirlpoolInstructionBuilder
        .build_buy_instructions(&hop)
        .await
        .ok()?;
    let estimated = (sol_lamports / 2_000).max(1);
    Some((hop_ixs, estimated))
}

/// Extract Raydium CPMM `swap_base_in` minimum_amount_out (bytes [16..24]).
pub fn cpmm_min_out(ixs: &[Instruction]) -> Option<u64> {
    use crate::instruction::utils::raydium_cpmm::{
        accounts as cpmm_accounts, SWAP_BASE_IN_DISCRIMINATOR,
    };
    ixs.iter()
        .find(|ix| {
            ix.program_id == cpmm_accounts::RAYDIUM_CPMM
                && ix.data.len() >= 24
                && &ix.data[..8] == SWAP_BASE_IN_DISCRIMINATOR
        })
        .map(|ix| u64::from_le_bytes(ix.data[16..24].try_into().unwrap()))
}

/// Like [`swap_params`] but enables seed-optimized ATA derivation.
pub fn swap_params_seeded(
    wallet: Arc<Keypair>,
    trade_type: TradeType,
    input_mint: Pubkey,
    output_mint: Pubkey,
    amount: u64,
    slippage_bps: u64,
    protocol_params: DexParamEnum,
) -> SwapParams {
    let mut p = swap_params(
        wallet,
        trade_type,
        input_mint,
        output_mint,
        amount,
        slippage_bps,
        protocol_params,
    );
    p.open_seed_optimize = true;
    p
}

/// Concatenate instruction lists for multi-leg simulate txs.
pub fn concat_ixs(legs: impl IntoIterator<Item = Vec<Instruction>>) -> Vec<Instruction> {
    let mut out = Vec::new();
    for leg in legs {
        out.extend(leg);
    }
    out
}

#[cfg(test)]
mod harness_unit_tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn concat_ixs_preserves_order() {
        let a = Instruction {
            program_id: Pubkey::new_from_array([1; 32]),
            accounts: vec![],
            data: vec![1],
        };
        let b = Instruction {
            program_id: Pubkey::new_from_array([2; 32]),
            accounts: vec![],
            data: vec![2],
        };
        let out = concat_ixs([vec![a.clone()], vec![b.clone()]]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].data, vec![1]);
        assert_eq!(out[1].data, vec![2]);
    }

    #[test]
    fn amm_v4_min_out_parses_swap_base_in_v2() {
        use crate::instruction::utils::raydium_amm_v4::{
            accounts as amm_accounts, SWAP_BASE_IN_V2_DISCRIMINATOR,
        };
        let mut data = Vec::with_capacity(17);
        data.extend_from_slice(SWAP_BASE_IN_V2_DISCRIMINATOR);
        data.extend_from_slice(&100_u64.to_le_bytes()); // amount_in
        data.extend_from_slice(&42_u64.to_le_bytes()); // min_out
        let ix = Instruction {
            program_id: amm_accounts::RAYDIUM_AMM_V4,
            accounts: vec![],
            data,
        };
        assert_eq!(amm_v4_min_out(&[ix]), Some(42));
    }

    #[test]
    fn pumpswap_buy_min_base_out_parses_exact_quote_in() {
        use crate::instruction::utils::pumpswap::{accounts, BUY_EXACT_QUOTE_IN_DISCRIMINATOR};
        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&BUY_EXACT_QUOTE_IN_DISCRIMINATOR);
        data.extend_from_slice(&1_000_u64.to_le_bytes()); // spendable_quote
        data.extend_from_slice(&777_u64.to_le_bytes()); // min_base
        let ix = Instruction {
            program_id: accounts::AMM_PROGRAM,
            accounts: vec![],
            data,
        };
        assert_eq!(pumpswap_buy_min_base_out(&[ix]), Some(777));
    }

    #[test]
    fn cpmm_min_out_parses_swap_base_in() {
        use crate::instruction::utils::raydium_cpmm::{
            accounts as cpmm_accounts, SWAP_BASE_IN_DISCRIMINATOR,
        };
        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(SWAP_BASE_IN_DISCRIMINATOR);
        data.extend_from_slice(&50_u64.to_le_bytes());
        data.extend_from_slice(&99_u64.to_le_bytes());
        let ix = Instruction {
            program_id: cpmm_accounts::RAYDIUM_CPMM,
            accounts: vec![],
            data,
        };
        assert_eq!(cpmm_min_out(&[ix]), Some(99));
    }

    #[test]
    fn enabled_gate_is_env_driven() {
        // Just ensure the helper is callable; env may or may not be set in unit tests.
        let _ = enabled();
    }
}
