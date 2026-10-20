//! Shared mechanics; strategy and transport configuration stay in examples.
#![allow(dead_code)]
#[path = "cpmm_creator_fee_stream_guard.rs"]
mod guard;
#[path = "pumpswap_routing_demo.rs"]
mod historical;
use anyhow::{Context, Result};
use sol_parser_sdk::{
    grpc::{
        AccountFilter, ClientConfig, EventType, EventTypeFilter, TransactionFilter, YellowstoneGrpc,
    },
    shredstream::ShredStreamClient,
    DexEvent,
};
use sol_trade_sdk::{PumpSwapRouting, PumpSwapRoutingConfig, PumpSwapRoutingFeed};
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

#[path = "streaming_options.rs"]
mod streaming_options;
use streaming_options::Source;
struct Signal {
    pool: Pubkey,
    buy: bool,
    received: Instant,
    signature: Signature,
}
fn received_at(timestamp: i64) -> Option<Instant> {
    let age = sol_parser_sdk::core::now_micros().checked_sub(timestamp)?;
    if timestamp <= 0 || !(0..=500_000).contains(&age) {
        return None;
    }
    Instant::now().checked_sub(Duration::from_micros(age as u64))
}
fn parser_signal(event: DexEvent) -> Option<Signal> {
    // This example filters the TARGET trade as a trigger only. Conversion pools
    // are independently subscribed raw account state across all configured DEXes.
    // For conversion candidates use stock_conversion_replay; this signal
    // filter must not be reused as a whole-route or stock-price event filter.
    let signature = event.metadata().signature;
    let received = received_at(event.metadata().grpc_recv_us)?;
    let (pool, buy) = match event {
        DexEvent::PumpSwapBuy(e) => (e.pool, true),
        DexEvent::PumpSwapSell(e) => (e.pool, false),
        _ => return None,
    };
    Some(Signal { pool, buy, received, signature })
}
fn streamer_signal(
    event: solana_streamer_sdk::streaming::event_parser::DexEvent,
) -> Option<Signal> {
    use solana_streamer_sdk::streaming::event_parser::DexEvent as E;
    let signature = event.metadata().signature;
    let received = received_at(event.metadata().recv_us)?;
    let (pool, buy) = match event {
        E::PumpSwapBuyEvent(e) => (e.pool, true),
        E::PumpSwapSellEvent(e) => (e.pool, false),
        _ => return None,
    };
    Some(Signal { pool, buy, received, signature })
}
fn endpoint() -> Result<String> {
    let endpoint = std::env::var("GRPC_ENDPOINT")
        .or_else(|_| std::env::var("GRPC_URL"))
        .context("GRPC_ENDPOINT (or GRPC_URL) required")?;
    anyhow::ensure!(!endpoint.trim().is_empty(), "GRPC_ENDPOINT is empty");
    Ok(endpoint)
}
fn token() -> Option<String> {
    std::env::var("GRPC_AUTH_TOKEN")
        .or_else(|_| std::env::var("GRPC_TOKEN"))
        .ok()
        .filter(|token| !token.trim().is_empty())
}
fn number(name: &str, default: u16) -> Result<u16> {
    Ok(std::env::var(name).ok().map(|v| v.parse()).transpose()?.unwrap_or(default))
}
fn config() -> Result<PumpSwapRoutingConfig> {
    let mut config = PumpSwapRoutingConfig::default();
    config.bridge_slippage_basis_points = number("BRIDGE_SLIPPAGE_BPS", 100)?;
    config.maximum_bridge_price_impact_basis_points = number("BRIDGE_IMPACT_BPS", 100)?;
    // Optional shared catalog exported by sol-parser-sdk. Merge identity data only;
    // historical fee/price/TVL fields never become fresh trading account state.
    if let Ok(path) = std::env::var("STOCK_CATALOG_JSON") {
        let catalog: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let pools = catalog["pools"].as_array().context("catalog pools missing")?;
        let mut hints = Vec::new();
        for pool in pools {
            let key = |name: &str| -> Result<Pubkey> {
                Ok(pool[name].as_str().context("catalog pubkey missing")?.parse()?)
            };
            hints.push(sol_trade_sdk::trading::core::params::PoolTradeHint {
                pool: key("pool")?,
                input_mint: key("quote_mint")?,
                output_mint: key("mint")?,
            });
        }
        config.extend_pools(hints)?;
        config.maximum_search_nodes = 512;
    }
    Ok(config)
}
async fn replay(source: Source) -> Result<()> {
    let snapshot = historical::historical_snapshot()?;
    let mut cfg = config()?;
    cfg.extend_pools(snapshot.candidates.clone())?;
    let routing = PumpSwapRouting::new(cfg)?;
    routing.publish(snapshot.cache, snapshot.context, snapshot.unix_timestamp);
    for (buy, amount) in [(true, 1_000_000u64), (false, 1_000_000_000u64)] {
        let trade = routing.prepare(
            snapshot.pool,
            snapshot.token,
            sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT,
            amount,
            100,
            buy,
        )?;
        println!("{source:?} historical quote buy={buy} hops={} min_output={} slot={}; no transport/signing/broadcast",trade.steps().len(),trade.preview().minimum_output_credit,trade.context().slot);
    }
    Ok(())
}

/// A separate parser raw-account connection serves every signal source. Streamer
/// exposes this same parser via parser_sdk; its decoded trades aren't raw state.
async fn account_feed(
    routing: Arc<PumpSwapRouting>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let mut keys: Vec<String> = std::env::var("ACCOUNT_KEYS")
        .context(
            "ACCOUNT_KEYS: complete pool/mint/vault/config/tick/bin dependency addresses required",
        )?
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<Pubkey>().map(|p| p.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    anyhow::ensure!(!keys.is_empty(), "ACCOUNT_KEYS is empty");
    keys.push(solana_sdk::sysvar::clock::ID.to_string());
    keys.sort();
    keys.dedup();
    let grpc = YellowstoneGrpc::new_with_config(endpoint()?, token(), ClientConfig::low_latency())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let q = grpc
        .subscribe_dex_events(
            vec![],
            vec![AccountFilter { account: keys, owner: vec![], filters: vec![] }],
            Some(EventTypeFilter::include_only(vec![
                EventType::AccountRawSnapshot,
                EventType::BlockMeta,
            ])),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let result: Result<()> = async {
        let mut feed = PumpSwapRoutingFeed::default();
        eprintln!("Live account collection only: executable snapshots require a provider-verified bootstrap and complete batch boundary (see docs/STREAMING_QUOTES.md).");
        let mut health = guard::CollectionStreamGuard::new(Duration::from_secs(2));
        loop {
            if *stop.borrow() {
                break;
            }
            if health.observe_status(grpc.subscription_status()) {
                feed.reset(&routing);
                // Bound the discard even if a producer keeps filling the queue.
                for _ in 0..q.len() { q.pop(); }
            }
            for _ in 0..256 {
                let Some(event) = q.pop() else { break };
                match event {
                    DexEvent::RawAccountSnapshot(event) => {
                        feed.update_from_parser_snapshot(*event)?;
                    }
                    DexEvent::BlockMeta(event) => health.observe_received_block(
                        event.metadata.slot,
                        event.metadata.grpc_recv_us,
                        sol_parser_sdk::core::now_micros(),
                        Instant::now(),
                    ),
                    _ => {}
                }
            }
            if health.observe_status(grpc.subscription_status()) {
                feed.reset(&routing);
                for _ in 0..q.len() { q.pop(); }
            }
            // A drained queue is not a transaction/batch completion signal.
            // Integrations publish here only after their provider confirms a
            // coherent bootstrap + catch-up and a complete ordered batch.
            if !health.is_live(Instant::now()) {
                routing.clear();
            }
            tokio::select! {
                changed = stop.changed() => { if changed.is_err() { break; } },
                _ = tokio::time::sleep(Duration::from_millis(1)) => {},
            }
        }
        Ok(())
    }
    .await;
    routing.clear();
    grpc.stop().await;
    result
}

pub async fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let Some((source, live)) = streaming_options::parse(&args)? else {
        println!("Usage: pumpswap_streaming_quotes [--source parser-grpc|parser-shred|streamer-grpc|streamer-shred] [--replay|--live]");
        println!("Default/--replay: offline captured-account quote. --live: collect signals; live quotes require provider-verified bootstrap and complete batch publication. Never signs or sends.\nRequired live: GRPC_ENDPOINT, ACCOUNT_KEYS, TARGET_POOL, TARGET_MINT, BUY_AMOUNT_RAW, SELL_AMOUNT_RAW. Optional: GRPC_AUTH_TOKEN, SHRED_ENDPOINT (shred modes), ASSET_MINT (WSOL), BRIDGE_SLIPPAGE_BPS=100, BRIDGE_IMPACT_BPS=100, TARGET_SLIPPAGE_BPS=100, STOCK_CATALOG_JSON. See docs/STREAMING_QUOTES.md.");
        return Ok(());
    };
    if !live {
        return replay(source).await;
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pool: Pubkey = std::env::var("TARGET_POOL")?.parse()?;
    let mint: Pubkey = std::env::var("TARGET_MINT")?.parse()?;
    let asset = std::env::var("ASSET_MINT")
        .ok()
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT);
    let buy: u64 = std::env::var("BUY_AMOUNT_RAW")?.parse()?;
    let sell: u64 = std::env::var("SELL_AMOUNT_RAW")?.parse()?;
    let target_slippage = u64::from(number("TARGET_SLIPPAGE_BPS", 100)?);
    let routing = Arc::new(PumpSwapRouting::new(config()?)?);
    let (tx, mut rx) = mpsc::channel::<Signal>(256);
    let (stop_tx, stop) = tokio::sync::watch::channel(false);
    // All error paths below stop subscriptions and clear the published state.
    let state_routing = routing.clone();
    let mut accounts = tokio::spawn(account_feed(state_routing, stop.clone()));
    let mut signals = tokio::spawn(signal_source(source, pool, tx, stop));
    let result: Result<()> = async {
        let mut seen = std::collections::VecDeque::new();
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                outcome = &mut accounts => {
                    outcome??;
                    anyhow::bail!("Account subscription stopped");
                },
                outcome = &mut signals => {
                    outcome??;
                    anyhow::bail!("Signal subscription stopped");
                },
                signal = rx.recv() => {
                    let Some(signal) = signal else { anyhow::bail!("Signal channel closed") };
                    if signal.pool != pool || signal.received.elapsed() > Duration::from_millis(500) {
                        continue;
                    }
                    let identity = (signal.signature, signal.pool, signal.buy);
                    if seen.contains(&identity) {
                        continue;
                    }
                    if seen.len() == 256 {
                        seen.pop_front();
                    }
                    seen.push_back(identity);
                    // Use our own amount and fresh cached state, never copied trade
                    // amounts/reserves. Shred does not establish execution success.
                    let amount = if signal.buy { buy } else { sell };
                    match routing.prepare(pool, mint, asset, amount, target_slippage, signal.buy) {
                        Ok(trade) => println!(
                            "{source:?} quote buy={} hops={} min_output={} state_slot={}",
                            signal.buy, trade.steps().len(),
                            trade.preview().minimum_output_credit, trade.context().slot,
                        ),
                        Err(error) => eprintln!("quote skipped: {error:#}"),
                    }
                },
            }
        }
        Ok(())
    }
    .await;
    let _ = stop_tx.send(true);
    routing.clear();
    if !accounts.is_finished() {
        let _ = accounts.await;
    }
    if !signals.is_finished() {
        let _ = signals.await;
    }
    result
}

async fn signal_source(
    source: Source,
    pool: Pubkey,
    tx: mpsc::Sender<Signal>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    match source {
        Source::ParserGrpc => {
            let grpc =
                YellowstoneGrpc::new_with_config(endpoint()?, token(), ClientConfig::low_latency())
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            let q = grpc
                .subscribe_dex_events(
                    vec![TransactionFilter::new().include_account(pool.to_string())],
                    vec![],
                    Some(EventTypeFilter::include_only(vec![
                        EventType::PumpSwapBuy,
                        EventType::PumpSwapSell,
                    ])),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut revision = grpc.subscription_status().continuity_revision;
            while !*stop.borrow() {
                let status = grpc.subscription_status();
                if status.continuity_revision != revision {
                    for _ in 0..q.len() {
                        q.pop();
                    }
                    revision = status.continuity_revision;
                }
                if status.connected {
                    for _ in 0..256 {
                        let Some(e) = q.pop() else { break };
                        if let Some(s) = parser_signal(e) {
                            let _ = tx.try_send(s);
                        }
                    }
                }
                tokio::select! {
                    changed = stop.changed() => { if changed.is_err() { break; } },
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {},
                }
            }
            grpc.stop().await;
        }
        Source::ParserShred => {
            let client = ShredStreamClient::new(std::env::var("SHRED_ENDPOINT")?)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let q = client
                .subscribe_with_filter(Some(EventTypeFilter::include_only(vec![
                    EventType::PumpSwapBuy,
                    EventType::PumpSwapSell,
                ])))
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            while !*stop.borrow() {
                for _ in 0..256 {
                    let Some(e) = q.pop() else { break };
                    if let Some(s) = parser_signal(e) {
                        let _ = tx.try_send(s);
                    }
                }
                tokio::select! {
                    changed = stop.changed() => { if changed.is_err() { break; } },
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {},
                }
            }
            client.stop().await;
        }
        Source::StreamerGrpc | Source::StreamerShred => {
            use solana_streamer_sdk::streaming::{
                event_parser::{
                    common::{filter::EventTypeFilter as F, EventType as E},
                    Protocol,
                },
                ShredStreamGrpc as S, YellowstoneGrpc as G,
            };
            let filter = Some(F::include_only(vec![E::PumpSwapBuy, E::PumpSwapSell]));
            let callback = move |event| {
                if let Some(s) = streamer_signal(event) {
                    if s.pool == pool {
                        let _ = tx.try_send(s);
                    }
                }
            };
            if matches!(source, Source::StreamerGrpc) {
                let grpc = G::new(endpoint()?, token()).map_err(|e| anyhow::anyhow!("{e}"))?;
                let f = solana_streamer_sdk::streaming::yellowstone_grpc::TransactionFilter {
                    account_include: vec![pool.to_string()],
                    account_exclude: vec![],
                    account_required: vec![],
                };
                grpc.subscribe_events_immediate(
                    vec![Protocol::PumpSwap],
                    None,
                    vec![f],
                    vec![],
                    filter,
                    None,
                    callback,
                )
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                while !*stop.borrow() {
                    if stop.changed().await.is_err() {
                        break;
                    }
                }
                grpc.stop().await;
            } else {
                let client =
                    S::new_with_config(std::env::var("SHRED_ENDPOINT")?, Default::default())
                        .await
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                client
                    .shredstream_subscribe(vec![Protocol::PumpSwap], None, filter, callback)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                while !*stop.borrow() {
                    if stop.changed().await.is_err() {
                        break;
                    }
                }
                client.stop().await;
            }
        }
    }
    Ok(())
}
