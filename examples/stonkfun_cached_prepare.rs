//! Offline parser → subscription cache → one independent StonkFun trade.
//! cargo run --example stonkfun_cached_prepare --features parser-adapter -- plan.json
//! The file contains subscription snapshots plus a parsed TransactionRoute;
//! no RPC client, signer, submission or simulation is used by this example.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sol_parser_sdk::{
    accounts::liquidity_snapshot::RawAccountSnapshotEvent,
    grpc::{EventType, EventTypeFilter},
    transaction_route::TransactionRoute,
};
use sol_trade_sdk::trading::core::params::{
    CacheReadContext, CachedQuoteRequest, CachedRouteStep, PoolTradeHint, PreparedStonkFunTrade,
    SubscriptionAccountCache, SwapParams,
};
use solana_sdk::pubkey::Pubkey;

#[derive(Deserialize)]
struct Conversion {
    observed_leg: usize,
    reverse: bool,
    input_amount: Option<u64>,
}
#[derive(Deserialize)]
struct Plan {
    snapshots: Vec<RawAccountSnapshotEvent>,
    observed_route: TransactionRoute,
    meme_leg: usize,
    meme_mint: Pubkey,
    /// SOL sentinel, WSOL mint, USDC mint or the meme pool's actual quote mint.
    asset: Pubkey,
    conversion: Vec<Conversion>,
    amount_in: u64,
    slippage_basis_points: u16,
    slot: u64,
    epoch: u64,
    maximum_slot_age: u64,
    unix_timestamp: u64,
    maximum_arrays: usize,
    is_buy: bool,
}

/// Configure the background gRPC subscriber with this opt-in event filter.
/// Its account address filters must include all dependent pools/configs/mints,
/// vaults, direction/amount-relevant arrays and Clock. Add other trade events
/// if needed; TransactionRoute comes from the transaction analyzer separately.
pub fn subscription_filter() -> EventTypeFilter {
    EventTypeFilter::include_only(vec![
        EventType::AccountRawSnapshot,
        EventType::RaydiumLaunchlabTrade,
        EventType::RaydiumCpmmSwap,
    ])
}

/// Caller supplies their wallet, current blockhash/nonce, ALTs, lifecycle policy
/// and non-RPC submit providers. Simulation/confirmation must be disabled.
/// Apply immediately from the same stable snapshot; reprepare after updates.
pub fn use_own_wallet(prepared: PreparedStonkFunTrade, caller: SwapParams) -> Result<SwapParams> {
    prepared.apply_to(caller)
}

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("Usage: stonkfun_cached_prepare plan.json")?;
    let plan: Plan = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(plan.observed_route.succeeded, "Do not copy failed trade intent");
    let leg = plan.observed_route.legs.get(plan.meme_leg).context("Missing meme leg")?;
    ensure!(
        leg.stonkfun_mode.is_some() || leg.stonkfun_graduated,
        "Meme leg needs verified StonkFun attribution"
    );
    let hint = PoolTradeHint::from_route_leg(leg)?;
    // Apply a complete background batch before exposing the cache to readers.
    let mut cache = SubscriptionAccountCache::default();
    for event in &plan.snapshots {
        cache.update_from_parser_snapshot(event)?;
    }
    // Select and order conversion legs deliberately: a historical flat leg list
    // can include unrelated swaps. Never copy source wallets or amounts.
    let steps = plan
        .conversion
        .iter()
        .map(|step| {
            let leg = plan
                .observed_route
                .legs
                .get(step.observed_leg)
                .context("Missing conversion leg")?;
            let mut pool = PoolTradeHint::from_route_leg(leg)?;
            if step.reverse {
                std::mem::swap(&mut pool.input_mint, &mut pool.output_mint);
            }
            Ok(CachedRouteStep { pool, input_amount: step.input_amount })
        })
        .collect::<Result<Vec<_>>>()?;
    let prepared = cache.prepare_stonkfun_trade(
        hint,
        plan.meme_mint,
        plan.asset,
        &steps,
        CachedQuoteRequest {
            amount_in: plan.amount_in,
            slippage_basis_points: plan.slippage_basis_points,
            unix_timestamp: plan.unix_timestamp,
            maximum_arrays: plan.maximum_arrays,
        },
        CacheReadContext {
            slot: plan.slot,
            epoch: plan.epoch,
            maximum_slot_age: plan.maximum_slot_age,
        },
        plan.is_buy,
    )?;
    println!(
        "{} {} → {}: input {}, protected net output {}",
        if prepared.is_buy { "buy" } else { "sell" },
        prepared.input_mint,
        prepared.output_mint,
        prepared.amount_in,
        prepared.minimum_amount_out
    );
    Ok(())
}
