//! gRPC raw-account cache → creator-fee estimate/instruction, with optional shred intent.
//! No RPC, simulation, signer or broadcast. Requires --features parser-adapter.
#[path = "support/cpmm_creator_fee_stream_guard.rs"]
mod stream_guard;
use anyhow::{Context, Result};
use sol_parser_sdk::{
    accounts::liquidity_snapshot::RawAccountSnapshotEvent,
    grpc::{
        AccountFilter, ClientConfig, EventType, EventTypeFilter, TransactionFilter, YellowstoneGrpc,
    },
    shredstream::ShredStreamClient,
    DexEvent,
};
use sol_trade_sdk::{
    instruction::utils::raydium_cpmm::get_creator_fee_share_pda,
    trading::core::params::{
        CacheReadContext, PreparedCpmmCreatorFeeCollection, SubscriptionAccountCache,
    },
};
use solana_sdk::pubkey::Pubkey;
use std::{
    io::{BufRead, BufReader, BufWriter, Write},
    time::{Duration, Instant},
};
use stream_guard::CollectionStreamGuard;

fn prepare(
    cache: &SubscriptionAccountCache,
    pool: Pubkey,
    payer: Option<Pubkey>,
    slot: u64,
    age: u64,
) -> Result<PreparedCpmmCreatorFeeCollection> {
    cache.prepare_cpmm_creator_fee_collection(
        pool,
        payer,
        CacheReadContext { slot, epoch: 0, maximum_slot_age: age },
    )
}
fn display(p: &PreparedCpmmCreatorFeeCollection, slot: u64) {
    println!("pool={} slot={} share_rate={} creator_before_transfer_fee={}/{} protocol_share={}/{} instruction_accounts={}",
        p.pool,slot,p.share_rate,p.creator_payout_token0,p.creator_payout_token1,p.protocol_share_token0,p.protocol_share_token1,p.instruction.accounts.len());
}
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("CPMM_POOL=<pool> CPMM_CREATOR=<creator> CPMM_AMM_CONFIG=<config> \
GRPC_URL=<endpoint> GRPC_TOKEN=<token> cargo run --features parser-adapter --example cpmm_creator_fee_stream
Optional: CPMM_PAYER (permissionless), SHRED_ENDPOINT, CPMM_SNAPSHOT_JSONL, MAXIMUM_SLOT_AGE (default 256), DURATION_SECONDS (default 30), STREAM_STALL_SECONDS (default 10), STREAM_POLL_US (default 100; 0 cooperative polling), CPMM_STREAM_REPORT_PATH (record raw JSONL).
--replay reads CPMM_SNAPSHOT_JSONL and exits without network. RawAccountSnapshotEvent JSON lines can be recorded with the parser SDK's grpc_raw_cache example.
No RPC bootstrap/fallback. A missing share update is unknown, not proof that the PDA does not exist. Supply validated startup snapshots or wait for complete account updates. Shred shows instruction intent only; it never changes cached fees.");
        return Ok(());
    }
    let pool: Pubkey = std::env::var("CPMM_POOL").context("CPMM_POOL required")?.parse()?;
    let payer = std::env::var("CPMM_PAYER").ok().map(|s| s.parse()).transpose()?;
    let age = std::env::var("MAXIMUM_SLOT_AGE").ok().map(|s| s.parse()).transpose()?.unwrap_or(256);
    let mut cache = SubscriptionAccountCache::default();
    let mut slot = 0;
    if let Ok(path) = std::env::var("CPMM_SNAPSHOT_JSONL") {
        for line in BufReader::new(std::fs::File::open(path)?).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event: RawAccountSnapshotEvent = serde_json::from_str(&line)?;
            slot = slot.max(event.metadata.slot);
            cache.update_from_parser_snapshot(&event)?;
        }
    }
    if std::env::args().any(|a| a == "--replay") {
        display(&prepare(&cache, pool, payer, slot, age)?, slot);
        return Ok(());
    }
    let creator: Pubkey = std::env::var("CPMM_CREATOR")
        .context("CPMM_CREATOR required for stream address filter")?
        .parse()?;
    let config: Pubkey = std::env::var("CPMM_AMM_CONFIG")
        .context("CPMM_AMM_CONFIG required for stream address filter")?
        .parse()?;
    let share = get_creator_fee_share_pda(&creator, &config);
    let _ = rustls::crypto::ring::default_provider().install_default();
    let grpc = YellowstoneGrpc::new_with_config(
        std::env::var("GRPC_URL")
            .or_else(|_| std::env::var("GRPC_ENDPOINT"))
            .context("GRPC_URL required")?,
        std::env::var("GRPC_TOKEN").or_else(|_| std::env::var("GRPC_AUTH_TOKEN")).ok(),
        ClientConfig::low_latency(),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let queue = grpc
        .subscribe_dex_events(
            vec![TransactionFilter::new().include_account(pool.to_string())],
            vec![AccountFilter {
                account: vec![pool.to_string(), config.to_string(), share.to_string()],
                owner: vec![],
                filters: vec![],
            }],
            Some(EventTypeFilter::include_only(vec![
                EventType::AccountRawSnapshot,
                EventType::BlockMeta,
                EventType::RaydiumCpmmCollectCreatorFee,
            ])),
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut shred = None;
    let result: Result<()> = async {
    shred = if let Ok(url) = std::env::var("SHRED_ENDPOINT") {
        let client = ShredStreamClient::new(url).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        let q = client
            .subscribe_with_filter(Some(EventTypeFilter::include_only(vec![
                EventType::RaydiumCpmmCollectCreatorFee,
            ])))
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Some((client, q))
    } else {
        None
    };
    let duration =
        std::env::var("DURATION_SECONDS").ok().map(|s| s.parse()).transpose()?.unwrap_or(30);
    let stall_seconds: u64 =
        std::env::var("STREAM_STALL_SECONDS").ok().map(|s| s.parse()).transpose()?.unwrap_or(10);
    anyhow::ensure!(stall_seconds > 0, "STREAM_STALL_SECONDS must be positive");
    let poll_us: u64 = std::env::var("STREAM_POLL_US").ok().map(|s| s.parse()).transpose()?.unwrap_or(100);
    let deadline = Instant::now() + Duration::from_secs(duration);
    let mut guard = CollectionStreamGuard::new(Duration::from_secs(stall_seconds));
    let mut last_prepared = None;
    let mut recorder = std::env::var("CPMM_STREAM_REPORT_PATH")
        .ok()
        .map(std::fs::File::create)
        .transpose()?
        .map(BufWriter::new);
    let mut seen = std::collections::HashSet::new();
    let mut raw_updates = 0u64;
    let mut last_error = String::new();
    let mut ready = false;
    let mut changed = true;
    while Instant::now() < deadline {
        if guard.observe_status(grpc.subscription_status()) {
            cache = SubscriptionAccountCache::default();
            for _ in 0..queue.len() { let _ = queue.pop(); }
            last_prepared = None;
            ready = false;
            changed = true;
            eprintln!("gRPC continuity lost: discarded cache and queued updates; waiting for a new snapshot");
        }
        let mut processed = 0;
        for _ in 0..256 {
            let Some(event) = queue.pop() else { break };
            processed += 1;
            match event {
                DexEvent::RawAccountSnapshot(e) => {
                    raw_updates += 1;
                    if let Some(file) = &mut recorder {
                        writeln!(file, "{}", serde_json::to_string(&*e)?)?;
                    }
                    if seen.insert(e.account.pubkey) {
                        println!(
                            "gRPC account={} slot={} write_version={} closed={}",
                            e.account.pubkey,
                            e.metadata.slot,
                            e.write_version,
                            e.account.lamports == 0
                        );
                    }
                    changed |= cache.update_from_owned_parser_snapshot(*e)?;
                }
                DexEvent::BlockMeta(e) => {
                    guard.observe_received_block(
                        e.metadata.slot,
                        e.metadata.grpc_recv_us,
                        sol_parser_sdk::core::now_micros(),
                        Instant::now(),
                    );
                    changed = true;
                }
                DexEvent::RaydiumCpmmCollectCreatorFee(e) if e.pool_state == pool => {
                    println!("gRPC collection instruction pool={} permissionless={}; await raw account updates for executed balances",pool,e.permissionless);
                }
                _ => {}
            }
        }
        if let Some((_, q)) = &shred {
            for _ in 0..256 {
                let Some(event) = q.pop() else { break };
                processed += 1;
                if let DexEvent::RaydiumCpmmCollectCreatorFee(e) = event {
                    if e.pool_state == pool {
                        println!("shred collection intent pool={} permissionless={}; execution unconfirmed, cache unchanged",pool,e.permissionless);
                    }
                }
            }
        }
        // Check again after draining: a gap/overflow during the batch invalidates it.
        if guard.observe_status(grpc.subscription_status()) {
            cache = SubscriptionAccountCache::default();
            for _ in 0..queue.len() { let _ = queue.pop(); }
            last_prepared = None;
            ready = false;
            changed = true;
        }
        if !guard.is_live(Instant::now()) {
            ready = false;
            last_prepared = None;
            let error = "gRPC disconnected or block progress missing/stalled; estimates unavailable";
            if error != last_error {
                eprintln!("Not ready: {error}");
                last_error = error.to_owned();
            }
        } else if !queue.is_empty() {
            // Never expose an estimate from an unfinished account batch.
            ready = false;
            // Even a batch containing only intents can defer readiness. Recheck
            // when draining finishes, without waiting for the next block event.
            changed = true;
        } else if changed {
            changed = false;
            let current_slot = guard.slot().expect("live guard has block progress");
            match prepare(&cache, pool, payer, current_slot, age) {
                Ok(p) => {
                    // Preparation itself takes time: reject a gap or new queued
                    // observations before publishing the derived instruction.
                    let gap = guard.observe_status(grpc.subscription_status());
                    if gap { cache = SubscriptionAccountCache::default(); last_prepared = None; }
                    if gap || !guard.is_live(Instant::now()) || !queue.is_empty() {
                        ready = false;
                        changed = true;
                        tokio::task::yield_now().await;
                        continue;
                    }
                    let same_estimate = last_prepared.as_ref().is_some_and(
                        |old: &PreparedCpmmCreatorFeeCollection| {
                            old.instruction == p.instruction
                                && [
                                    old.share_rate,
                                    old.creator_payout_token0,
                                    old.creator_payout_token1,
                                    old.protocol_share_token0,
                                    old.protocol_share_token1,
                                ] == [
                                    p.share_rate,
                                    p.creator_payout_token0,
                                    p.creator_payout_token1,
                                    p.protocol_share_token0,
                                    p.protocol_share_token1,
                                ]
                        },
                    );
                    if !same_estimate {
                        display(&p, current_slot);
                    }
                    last_prepared = Some(p);
                    ready = true;
                    last_error.clear();
                }
                Err(e) => {
                    ready = false;
                    last_prepared = None;
                    let error = e.to_string();
                    if error != last_error {
                        eprintln!("Not ready: {error}");
                        last_error = error;
                    }
                }
            }
        }
        if processed != 0 || poll_us == 0 {
            tokio::task::yield_now().await;
        } else {
            // Tokio timer resolution/scheduling can exceed the configured interval.
            tokio::time::sleep(Duration::from_micros(poll_us)).await;
        }
    }
    if let Some(file) = &mut recorder {
        file.flush()?;
    }
    // Disk flushing can stall long enough to disconnect or expire block progress.
    let final_status = grpc.subscription_status();
    if guard.observe_status(final_status) || !guard.is_live(Instant::now()) || !queue.is_empty() {
        ready = false;
    }
    println!(
        "gRPC raw_updates={raw_updates} distinct_accounts={} ready={ready} generation={} disconnects={} dropped_events={} rpc_calls=0",
        seen.len(), final_status.generation, final_status.disconnects, final_status.dropped_events
    );
    anyhow::ensure!(
        ready,
        "No complete, fresh collection snapshot available; no RPC fallback was attempted"
    );
    Ok(())
    }.await;
    // Cleanup also runs on recorder, cache-conflict and configuration errors.
    grpc.stop().await;
    if let Some((client, _)) = shred {
        client.stop().await;
    }
    result
}
