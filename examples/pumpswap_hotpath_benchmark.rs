//! Configurable native offline cache/route/sign benchmark; never sends or fetches transactions.
use anyhow::{ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sol_trade_sdk::{
    common::{GasFeeStrategyValue, TradeTransactionVersion},
    instruction::pumpswap::PumpSwapInstructionBuilder,
    instruction::utils::pumpswap as ps,
    trading::core::{
        params::{
            CacheReadContext, CachedAccount, CachedQuoteRequest, PoolTradeHint,
            PumpSwapRouteSearch, SubscriptionAccountCache,
        },
        traits::InstructionBuilder,
    },
};
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::{hint::black_box, sync::Arc, time::Instant};
#[path = "support/pumpswap_offline.rs"]
mod offline;
#[derive(Deserialize)]
struct Config {
    warmup: usize,
    samples: usize,
    batch: usize,
}
fn measure(config: &Config, name: &str, mut run: impl FnMut() -> Result<u64>) -> Result<Value> {
    let mut checksum = 0u64;
    for _ in 0..config.warmup {
        checksum = checksum.wrapping_add(black_box(run()?));
    }
    let mut samples = Vec::with_capacity(config.samples);
    for _ in 0..config.samples {
        let start = Instant::now();
        for _ in 0..config.batch {
            checksum = checksum.wrapping_add(black_box(run()?));
        }
        samples.push(start.elapsed().as_secs_f64() * 1_000_000.0 / config.batch as f64);
    }
    samples.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        samples[((config.samples as f64 * p).ceil() as usize)
            .saturating_sub(1)
            .min(config.samples - 1)]
    };
    Ok(
        json!({"name":name,"unit":"microseconds/transaction","p50":percentile(0.5),"p95":percentile(0.95),"p99":percentile(0.99),"checksum":checksum}),
    )
}
fn main() -> Result<()> {
    let path = std::env::args().nth(1);
    let config: Config = serde_json::from_str(
        &path
            .map(std::fs::read_to_string)
            .transpose()?
            .unwrap_or_else(|| include_str!("pumpswap_hotpath_benchmark.json").into()),
    )?;
    for count in [config.warmup, config.samples, config.batch] {
        ensure!((1..=100_000).contains(&count), "Invalid benchmark count");
    }
    let f: Value =
        serde_json::from_str(include_str!("../tests/fixtures/pumpswap_ttwo_route_accounts.json"))?;
    let key = |v: &Value| -> Result<Pubkey> {
        Ok(v.as_str().ok_or_else(|| anyhow::anyhow!("Missing pubkey"))?.parse()?)
    };
    let mut cache = SubscriptionAccountCache::default();
    for a in f["accounts"].as_array().unwrap() {
        let address = key(&a["address"])?;
        let mut data = STANDARD.decode(a["data_base64"].as_str().unwrap())?;
        if address == ps::accounts::GLOBAL_ACCOUNT {
            // Keep one captured permitted recipient per array. Production cache
            // preparation randomly selects a recipient; benchmarks fix that input
            // so message bytes and PDA derivation workloads remain repeatable.
            let global = ps::decode_global_config(&data)
                .ok_or_else(|| anyhow::anyhow!("Invalid fixture GlobalConfig"))?;
            for recipients in [global.protocol_fee_recipients, global.buyback_fee_recipients] {
                let bytes: Vec<_> = recipients.iter().flat_map(|key| key.to_bytes()).collect();
                let positions: Vec<_> = data
                    .windows(bytes.len())
                    .enumerate()
                    .filter_map(|(i, value)| (value == bytes).then_some(i))
                    .collect();
                ensure!(positions.len() == 1, "Ambiguous fixture recipient layout");
                let chosen = recipients
                    .iter()
                    .find(|key| **key != Pubkey::default())
                    .ok_or_else(|| anyhow::anyhow!("No permitted fixture fee recipient"))?;
                let start = positions[0];
                data[start..start + bytes.len()].fill(0);
                data[start..start + 32].copy_from_slice(chosen.as_ref());
            }
        }
        cache.update(
            address,
            CachedAccount {
                owner: key(&a["owner"])?,
                data,
                slot: a["slot"].as_u64().unwrap(),
                write_version: 0,
            },
        )?;
    }
    let context = CacheReadContext {
        slot: f["slot"].as_u64().unwrap(),
        epoch: f["epoch"].as_u64().unwrap(),
        maximum_slot_age: f["maximum_slot_age"].as_u64().unwrap(),
    };
    let s = &f["route_search"];
    let hints = s["conversion_pools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            Ok(PoolTradeHint {
                pool: key(&p["pool"])?,
                input_mint: key(&p["input_mint"])?,
                output_mint: key(&p["output_mint"])?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let search = PumpSwapRouteSearch::new(
        key(&s["pool"])?,
        key(&s["token_mint"])?,
        key(&s["asset_mint"])?,
        hints,
    );
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let mut results = Vec::new();
    let mut signed_oracles = Vec::new();
    for is_buy in [true, false] {
        let request = CachedQuoteRequest {
            amount_in: if is_buy { 1_000_000 } else { 1_000_000_000 },
            slippage_basis_points: 100,
            unix_timestamp: f["unix_timestamp"].as_u64().unwrap(),
            maximum_arrays: 6,
        };
        let prepared = cache.prepare_best_pumpswap_trade(&search, request, context, is_buy)?.trade;
        let mut settings = offline::offline_demo_params(TradeTransactionVersion::V1, !is_buy);
        settings.payer = Arc::new(Keypair::new_from_array([42; 32]));
        settings.recent_blockhash = Some(Hash::default());
        let applied = prepared.clone().apply_to(settings.clone())?;
        let name = if is_buy { "buy" } else { "sell" };
        results.push(measure(&config, &format!("search_prepare_three_hop_{name}"), || {
            Ok(cache
                .prepare_best_pumpswap_trade(&search, request, context, is_buy)?
                .trade
                .preview()
                .minimum_output_credit)
        })?);
        results.push(measure(&config, &format!("build_three_hop_instructions_{name}"), || {
            let ixs = runtime.block_on(async {
                if is_buy {
                    PumpSwapInstructionBuilder.build_buy_instructions(&applied).await
                } else {
                    PumpSwapInstructionBuilder.build_sell_instructions(&applied).await
                }
            })?;
            Ok(ixs.len() as u64)
        })?);
        let fee = GasFeeStrategyValue { cu_limit: 450_000, cu_price: 10_000, tip: 0.0 };
        let tx = runtime.block_on(prepared.clone().build_transaction(
            settings.clone(),
            context,
            fee,
            Pubkey::default(),
        ))?;
        ensure!(
            tx.signatures[0].verify(settings.payer.pubkey().as_ref(), &tx.message.serialize()),
            "Invalid pre-benchmark signature"
        );
        signed_oracles.push(json!({"direction":name,"message_sha256":format!("{:x}",Sha256::digest(tx.message.serialize())),"signature":tx.signatures[0].to_string()}));
        results.push(measure(&config, &format!("build_sign_prepared_v1_{name}"), || {
            let tx = runtime.block_on(prepared.clone().build_transaction(
                settings.clone(),
                context,
                fee,
                Pubkey::default(),
            ))?;
            Ok(wincode::serialized_size(&tx)? as u64)
        })?);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"profile":if cfg!(debug_assertions) {"debug"} else {"release"},"config":{"warmup":config.warmup,"samples":config.samples,"batch":config.batch},"results":results,"signed_oracles":signed_oracles,"note":"Warm local CPU distributions; setup, fixture reads and network excluded. No RPC or broadcast. Batch-normalized samples; not production latency."})
        )?
    );
    Ok(())
}
