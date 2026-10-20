//! Shared offline setup for the global routing examples.
use anyhow::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_trade_sdk::{
    common::{
        GasFeeStrategy, InfrastructureConfig, SolanaRpcClient, TradeConfig, TradeTransactionVersion,
    },
    trading::{
        core::params::{
            CacheReadContext, CachedAccount, DexParamEnum, PoolTradeHint, SubscriptionAccountCache,
        },
        factory::DexType,
    },
    BuyAmount, PumpSwapRouting, PumpSwapRoutingConfig, SimpleBuyParams, TradeTokenType,
    TradingClient, TradingInfrastructure,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Keypair};
use std::sync::Arc;

pub struct DemoSnapshot {
    pub cache: Arc<SubscriptionAccountCache>,
    pub context: CacheReadContext,
    pub unix_timestamp: u64,
    pub pool: Pubkey,
    pub token: Pubkey,
    pub candidates: Vec<PoolTradeHint>,
}
pub fn historical_snapshot() -> Result<DemoSnapshot> {
    let f: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/pumpswap_ttwo_route_accounts.json"
    ))?;
    let key = |v: &serde_json::Value| -> Result<Pubkey> {
        Ok(v.as_str().ok_or_else(|| anyhow::anyhow!("Missing pubkey"))?.parse()?)
    };
    let mut cache = SubscriptionAccountCache::default();
    for a in f["accounts"].as_array().unwrap() {
        cache.update(
            key(&a["address"])?,
            CachedAccount {
                owner: key(&a["owner"])?,
                data: STANDARD.decode(a["data_base64"].as_str().unwrap())?,
                slot: a["slot"].as_u64().unwrap(),
                write_version: 0,
            },
        )?;
    }
    let s = &f["route_search"];
    let candidates = s["conversion_pools"]
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
    Ok(DemoSnapshot {
        cache: Arc::new(cache),
        context: CacheReadContext {
            slot: f["slot"].as_u64().unwrap(),
            epoch: f["epoch"].as_u64().unwrap(),
            maximum_slot_age: 32,
        },
        unix_timestamp: f["unix_timestamp"].as_u64().unwrap(),
        pool: key(&s["pool"])?,
        token: key(&s["token_mint"])?,
        candidates,
    })
}

pub fn configured_client(
    snapshot: &DemoSnapshot,
    wallet: Arc<Keypair>,
) -> Result<(TradingClient, Arc<PumpSwapRouting>)> {
    let mut markets = PumpSwapRoutingConfig::default();
    markets.extend_pools(snapshot.candidates.clone())?; // duplicates are harmless
    let routing = Arc::new(PumpSwapRouting::new(markets)?);
    let config =
        TradeConfig::builder("http://127.0.0.1:1".into(), vec![], CommitmentConfig::processed())
            .pumpswap_routing(routing.clone())
            .create_wsol_ata_on_startup(false)
            .transaction_version(TradeTransactionVersion::V1)
            .build();
    // Offline shared infrastructure: no senders, unusable RPC endpoint. Production
    // applications can instead call TradingClient::new(wallet, config).await once.
    let infrastructure = Arc::new(TradingInfrastructure {
        rpc: Arc::new(SolanaRpcClient::new(config.rpc_url.clone())),
        swqos_clients: Arc::new(vec![]),
        config: InfrastructureConfig::from_trade_config(&config),
        max_sender_concurrency: 1,
        effective_core_ids: Arc::new(vec![]),
    });
    let mut client = TradingClient::from_infrastructure(wallet, infrastructure, true)
        .with_pumpswap_routing(config.pumpswap_routing.unwrap());
    client.transaction_version = config.transaction_version;
    Ok((client, routing))
}

pub fn buy_request(snapshot: &DemoSnapshot, asset: TradeTokenType, hash: Hash) -> SimpleBuyParams {
    // Only the market identity is required; all executable pool data is rebuilt
    // from the globally published cache, never from historical trade fields.
    SimpleBuyParams::new(
        DexType::PumpSwap,
        asset,
        snapshot.token,
        BuyAmount::ExactInput(1_000_000),
        DexParamEnum::PumpSwapAutoRoute(snapshot.pool),
        hash,
        GasFeeStrategy::new(),
    )
    .slippage_basis_points(500)
}
