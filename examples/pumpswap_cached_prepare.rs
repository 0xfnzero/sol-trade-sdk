//! Offline cache → quote → high-level request, with no RPC or submission.
//! cargo run --example pumpswap_cached_prepare -- buy 10000
//! cargo run --example pumpswap_cached_prepare -- sell 1000000000 snapshot.json
//! Append v0 or v1 to also build/sign with an ephemeral demo wallet (no submission).
//! The default uses a historical TTWO snapshot; refresh all accounts for trading.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use sol_trade_sdk::{
    common::{GasFeeStrategy, GasFeeStrategyValue, TradeTransactionVersion},
    trading::core::params::{
        CacheReadContext, CachedAccount, CachedQuoteRequest, CachedRouteStep,
        PoolTradeHint, PumpSwapRouteSearch, SubscriptionAccountCache,
    },
};
use solana_sdk::{hash::Hash, pubkey::Pubkey, signer::Signer};
#[path = "support/pumpswap_offline.rs"]
mod offline;
use offline::offline_demo_params;

fn pubkey<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Pubkey, D::Error> {
    String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
}

#[derive(Deserialize)]
struct Account {
    #[serde(deserialize_with = "pubkey")]
    address: Pubkey,
    #[serde(deserialize_with = "pubkey")]
    owner: Pubkey,
    /// Uncompressed account bytes in base64 (decompress base64+zstd RPC first).
    data_base64: String,
    slot: u64,
    #[serde(default)]
    write_version: u64,
}
#[derive(Deserialize)]
struct Step {
    #[serde(deserialize_with = "pubkey")]
    pool: Pubkey,
    #[serde(deserialize_with = "pubkey")]
    input_mint: Pubkey,
    #[serde(deserialize_with = "pubkey")]
    output_mint: Pubkey,
    input_amount: Option<u64>,
}

#[derive(Deserialize)]
struct Search {
    #[serde(deserialize_with = "pubkey")]
    pool: Pubkey,
    #[serde(deserialize_with = "pubkey")]
    token_mint: Pubkey,
    #[serde(deserialize_with = "pubkey")]
    asset_mint: Pubkey,
    conversion_pools: Vec<Step>,
}

#[derive(Deserialize)]
struct Snapshot {
    accounts: Vec<Account>,
    slot: u64,
    epoch: u64,
    maximum_slot_age: u64,
    unix_timestamp: u64,
    route_search: Option<Search>,
    // Explicit execution order; sell paths are quoted separately.
    #[serde(default)]
    buy_steps: Vec<Step>,
    #[serde(default)]
    sell_steps: Vec<Step>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let direction = args.first().map(String::as_str).unwrap_or("buy");
    ensure!(matches!(direction, "buy" | "sell"), "Direction must be buy or sell");
    let is_buy = direction == "buy";
    let amount = args.get(1).map(|v| v.parse()).transpose()?.unwrap_or(if is_buy {
        10_000
    } else {
        1_000_000_000
    });
    let json = if let Some(path) = args.get(2) {
        std::fs::read_to_string(path)?
    } else {
        include_str!("../tests/fixtures/pumpswap_ttwo_accounts.json").to_owned()
    };
    let snapshot: Snapshot = serde_json::from_str(&json)?;
    let mut cache = SubscriptionAccountCache::default();
    for account in snapshot.accounts {
        cache.update(
            account.address,
            CachedAccount {
                owner: account.owner,
                data: STANDARD.decode(account.data_base64)?,
                slot: account.slot,
                write_version: account.write_version,
            },
        )?;
    }
    let request = CachedQuoteRequest {
        amount_in: amount,
        slippage_basis_points: 100,
        unix_timestamp: snapshot.unix_timestamp,
        maximum_arrays: 6,
    };
    let context = CacheReadContext {
        slot: snapshot.slot,
        epoch: snapshot.epoch,
        maximum_slot_age: snapshot.maximum_slot_age,
    };
    let (prepared, input, output) = if let Some(search) = snapshot.route_search {
        let mut candidates = Vec::new();
        for step in search.conversion_pools {
            ensure!(
                step.input_amount.is_none(),
                "Search candidates must not contain historical input amounts"
            );
            candidates.push(PoolTradeHint {
                pool: step.pool,
                input_mint: step.input_mint,
                output_mint: step.output_mint,
            });
        }
        let plan =
            PumpSwapRouteSearch::new(search.pool, search.token_mint, search.asset_mint, candidates);
        let selected = cache.prepare_best_pumpswap_trade(&plan, request, context, is_buy)?;
        println!(
            "Evaluated {} paths, rejected {}",
            selected.routes_evaluated,
            selected.rejected_routes.len()
        );
        for rejection in &selected.rejected_routes {
            println!("Rejected {:?}: {}", rejection.pools, rejection.reason);
        }
        let (input, output) = if is_buy {
            (search.asset_mint, search.token_mint)
        } else {
            (search.token_mint, search.asset_mint)
        };
        (selected.trade, input, output)
    } else {
        let steps: Vec<_> = if is_buy { snapshot.buy_steps } else { snapshot.sell_steps }
            .into_iter()
            .map(|step| CachedRouteStep {
                pool: PoolTradeHint {
                    pool: step.pool,
                    input_mint: step.input_mint,
                    output_mint: step.output_mint,
                },
                input_amount: step.input_amount,
            })
            .collect();
        let input = steps.first().context("Missing route steps")?.pool.input_mint;
        let output = steps.last().unwrap().pool.output_mint;
        (
            cache.prepare_pumpswap_trade(&steps, input, output, request, context, is_buy)?,
            input,
            output,
        )
    };
    println!(
        "Offline snapshot slot={}, {} {} → {}: input={}, minimum output={}",
        prepared.context().slot,
        direction,
        input,
        output,
        amount,
        prepared.preview().minimum_output_credit
    );
    for step in prepared.steps() {
        println!(
            "  pool {}: {} → {}, input {}",
            step.pool.pool,
            step.pool.input_mint,
            step.pool.output_mint,
            step.input_amount.unwrap()
        );
    }
    if let Some(version) = args.get(3) {
        let version = match version.as_str() {
            "v0" => TradeTransactionVersion::V0,
            "v1" => TradeTransactionVersion::V1,
            _ => anyhow::bail!("Transaction version must be v0 or v1"),
        };
        // Ephemeral wallet and synthetic blockhash: this artifact demonstrates
        // construction against historical data and is not a live trade.
        let params = offline_demo_params(version, !is_buy);
        let payer = params.payer.pubkey();
        let tx = prepared
            .clone()
            .build_transaction(
                params,
                context,
                GasFeeStrategyValue { cu_limit: 450_000, cu_price: 10_000, tip: 0.0 },
                Pubkey::default(),
            )
            .await?;
        ensure!(
            tx.signatures[0].verify(payer.as_ref(), &tx.message.serialize()),
            "Invalid local signature"
        );
        println!(
            "Offline demo transaction: {:?}, {} bytes, signature verified; not submitted",
            version,
            wincode::serialized_size(&tx)?
        );
    }
    // Demonstrate high-level request construction only. Execution needs a current
    // blockhash, fresh state, the caller's wallet and suitable ALTs/tx version.
    if is_buy {
        let request = prepared.into_buy_params(Hash::default(), GasFeeStrategy::new())?;
        println!("Prepared buy: {:?}, pay with {:?}", request.amount, request.pay_with);
    } else {
        let request = prepared.into_sell_params(Hash::default(), GasFeeStrategy::new())?;
        println!("Prepared sell: {:?}, receive as {:?}", request.amount, request.receive_as);
    }
    Ok(())
}
