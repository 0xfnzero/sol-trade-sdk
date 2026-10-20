//! Offline exact-input quotes and price-impact checks for cached pool protocols.
//! cargo run --example pool_quote_tools -- [snapshot.json] [raw_amount] [reverse]
#[path = "support/pumpswap_routing_demo.rs"]
#[allow(dead_code)]
mod historical;
use anyhow::{Context, Result};
use sol_trade_sdk::trading::core::params::{CachedQuoteRequest, PoolTradeHint};
use solana_sdk::pubkey::Pubkey;

#[path = "support/account_snapshot.rs"]
mod account_snapshot;

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("pool_quote_tools [snapshot.json] [raw_amount=10000000] [reverse]\nNo arguments: historical USDC→TTWO quote. Snapshot format is bridge_pool_depth's {{slot,hints,accounts}}; hints determine direction. All amounts are raw units. BRIDGE_IMPACT_BPS defaults to 100. No RPC or transactions.");
        return Ok(());
    }
    anyhow::ensure!(
        args.len() <= 3
            && args.first().is_none_or(|arg| !arg.starts_with('-'))
            && args.get(2).is_none_or(|arg| arg == "reverse"),
        "expected [snapshot.json] [raw_amount] [reverse]; use --help"
    );
    let (cache, ctx, time, mut hint) = if let Some(path) = args.first() {
        let f: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let snapshot = account_snapshot::AccountSnapshot::from_document(&f)?;
        let h = f["hints"].as_array().and_then(|h| h.first()).context("one pool hint required")?;
        let key = |field: &str| -> Result<Pubkey> {
            Ok(h[field].as_str().context("hint key missing")?.parse()?)
        };
        (
            snapshot.cache,
            snapshot.context,
            snapshot.unix_timestamp,
            PoolTradeHint {
                pool: key("pool")?,
                input_mint: key("input_mint")?,
                output_mint: key("output_mint")?,
            },
        )
    } else {
        let d = historical::historical_snapshot()?;
        ((*d.cache).clone(), d.context, d.unix_timestamp, d.candidates[0].reversed())
    };
    if args.get(2).is_some_and(|v| v == "reverse") {
        hint = hint.reversed();
    }
    let amount = args.get(1).map(|v| v.parse()).transpose()?.unwrap_or(10_000_000);
    let request = CachedQuoteRequest::new(amount, time);
    let quote =
        cache.quote_pool_exact_in(hint.pool, hint.input_mint, hint.output_mint, request, ctx)?;
    println!(
        "pool={} in={} out={} amount_raw={} estimated_out_raw={} minimum_out_raw={} slot={}",
        hint.pool,
        hint.input_mint,
        hint.output_mint,
        amount,
        quote.estimated_net_amount_out,
        quote.minimum_net_amount_out,
        quote.state_slot
    );
    let limit =
        std::env::var("BRIDGE_IMPACT_BPS").ok().map(|v| v.parse()).transpose()?.unwrap_or(100);
    match cache.quote_exact_in_with_impact(hint, request, ctx, limit) {
        Ok(q) => println!(
            "price_impact_bps={:.4} probe_in_raw={} probe_out_raw={}",
            q.price_impact_basis_points, q.probe_amount_in, q.probe_amount_out
        ),
        Err(e) => println!("order rejected by impact policy: {e:#}"),
    }
    // This reverse quote starts from the SAME historical state. It is a separate
    // hypothetical sell, not a simulated sequential buy/sell against mutated state.
    let sell = cache.quote_exact_in(
        hint.reversed(),
        CachedQuoteRequest::new(quote.estimated_net_amount_out, time),
        ctx,
    )?;
    println!("independent_reverse_estimated_out_raw={}", sell.estimated_net_amount_out);
    Ok(())
}
