//! Instance-scoped bridge markets with atomically published subscription snapshots.
use crate::trading::core::params::{
    CacheReadContext, CachedQuoteRequest, PoolTradeHint, PreparedPumpSwapTrade,
    PumpSwapRouteSearch, SubscriptionAccountCache,
};
use anyhow::{ensure, Result};
use arc_swap::ArcSwapOption;
use solana_sdk::pubkey::Pubkey;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Register markets once; refresh their account state outside the trading path.
/// Only protocols already supported by the cached quote engine can be added.
#[derive(Clone, Debug)]
pub struct PumpSwapRoutingConfig {
    pub conversion_pools: Vec<PoolTradeHint>,
    pub maximum_conversion_hops: usize,
    pub maximum_search_nodes: usize,
    pub maximum_arrays: usize,
    pub maximum_slot_age: u64,
    /// Local timeout also rejects a stopped subscription whose slot never advances.
    pub maximum_snapshot_age: Duration,
}
impl PumpSwapRoutingConfig {
    pub fn new(conversion_pools: Vec<PoolTradeHint>) -> Self {
        Self {
            conversion_pools,
            maximum_conversion_hops: 2,
            maximum_search_nodes: 128,
            maximum_arrays: 6,
            maximum_slot_age: 32,
            maximum_snapshot_age: Duration::from_secs(2),
        }
    }
}
struct Snapshot {
    cache: Arc<SubscriptionAccountCache>,
    context: CacheReadContext,
    unix_timestamp: u64,
    received_at: Instant,
}

/// Shared across cloned clients/wallets without holding a lock during quoting.
/// Publishing replaces cache + Clock/slot together. Readers retain one immutable
/// snapshot for the complete search. No RPC client or network fallback exists here.
pub struct PumpSwapRouting {
    config: PumpSwapRoutingConfig,
    snapshot: ArcSwapOption<Snapshot>,
}
impl std::fmt::Debug for PumpSwapRouting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PumpSwapRouting").field("config", &self.config).finish_non_exhaustive()
    }
}
impl PumpSwapRouting {
    pub fn new(config: PumpSwapRoutingConfig) -> Result<Self> {
        ensure!(config.conversion_pools.len() <= 32, "Too many bridge pools");
        ensure!(config.maximum_conversion_hops <= 3, "Too many bridge hops");
        ensure!((1..=4096).contains(&config.maximum_search_nodes), "Invalid search budget");
        ensure!((1..=32).contains(&config.maximum_arrays), "Invalid array budget");
        ensure!(!config.maximum_snapshot_age.is_zero(), "Snapshot timeout must be positive");
        let mut pools = std::collections::HashMap::new();
        for hint in &config.conversion_pools {
            let normalize = |mint| {
                if mint == crate::constants::SOL_TOKEN_ACCOUNT {
                    crate::constants::WSOL_TOKEN_ACCOUNT
                } else {
                    mint
                }
            };
            let a = normalize(hint.input_mint);
            let b = normalize(hint.output_mint);
            ensure!(
                hint.pool != Pubkey::default()
                    && a != b
                    && a != Pubkey::default()
                    && b != Pubkey::default(),
                "Invalid bridge pool identity"
            );
            if let Some(previous) = pools.insert(hint.pool, (a.min(b), a.max(b))) {
                ensure!(previous == (a.min(b), a.max(b)), "Conflicting bridge pool identity");
            }
        }
        Ok(Self { config, snapshot: ArcSwapOption::empty() })
    }

    /// Background ingestion only. Supply a coherent cache and current chain Clock
    /// from the same chosen fork. Preserve each account's actual update slot; do
    /// not relabel old bytes as fresh or republish a frozen Clock as a heartbeat.
    pub fn publish(
        &self,
        cache: Arc<SubscriptionAccountCache>,
        context: CacheReadContext,
        unix_timestamp: u64,
    ) {
        self.snapshot.store(Some(Arc::new(Snapshot {
            cache,
            context,
            unix_timestamp,
            received_at: Instant::now(),
        })));
    }

    /// Invalidate immediately on disconnect/reorg until a coherent snapshot is ready.
    pub fn clear(&self) {
        self.snapshot.store(None);
    }

    /// Local exact-input route preparation. Pool addresses are supplied per trade;
    /// bridge candidates and current state are taken from this instance.
    pub fn prepare(
        &self,
        pool: Pubkey,
        token: Pubkey,
        asset: Pubkey,
        amount: u64,
        slippage: u64,
        is_buy: bool,
    ) -> Result<PreparedPumpSwapTrade> {
        let snapshot = self
            .snapshot
            .load_full()
            .ok_or_else(|| anyhow::anyhow!("PumpSwap routing snapshot is not ready"))?;
        ensure!(
            snapshot.received_at.elapsed() <= self.config.maximum_snapshot_age,
            "PumpSwap routing snapshot expired"
        );
        let candidates =
            self.config.conversion_pools.iter().copied().filter(|hint| hint.pool != pool).collect();
        let mut search = PumpSwapRouteSearch::new(pool, token, asset, candidates);
        search.maximum_conversion_hops = self.config.maximum_conversion_hops;
        search.maximum_search_nodes = self.config.maximum_search_nodes;
        let request = CachedQuoteRequest {
            amount_in: amount,
            slippage_basis_points: slippage
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid slippage"))?,
            unix_timestamp: snapshot.unix_timestamp,
            maximum_arrays: self.config.maximum_arrays,
        };
        let mut context = snapshot.context;
        context.maximum_slot_age = context.maximum_slot_age.min(self.config.maximum_slot_age);
        let selected =
            snapshot.cache.prepare_best_pumpswap_trade(&search, request, context, is_buy)?;
        ensure!(
            snapshot.received_at.elapsed() <= self.config.maximum_snapshot_age,
            "PumpSwap routing snapshot expired during search"
        );
        Ok(selected.trade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::{
            AccountPolicy, BuyAmount, SellAmount, SimpleBuyParams, SimpleSellParams,
            TradeTokenType, TradingClient, TradingInfrastructure,
        },
        common::{GasFeeStrategy, InfrastructureConfig, SolanaRpcClient, TradeConfig},
        constants::{
            ASSOCIATED_TOKEN_PROGRAM_ID, SOL_TOKEN_ACCOUNT, TOKEN_PROGRAM, WSOL_TOKEN_ACCOUNT,
        },
        instruction::pumpswap::PumpSwapInstructionBuilder,
        trading::{
            core::{
                params::{CachedAccount, DexParamEnum},
                traits::InstructionBuilder,
            },
            factory::DexType,
        },
    };
    use base64::{engine::general_purpose::STANDARD, Engine};
    use solana_commitment_config::CommitmentConfig;
    use solana_sdk::{hash::Hash, signature::Keypair};

    fn fixture(
    ) -> (Arc<PumpSwapRouting>, Arc<SubscriptionAccountCache>, CacheReadContext, u64, Pubkey, Pubkey)
    {
        let f: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/pumpswap_ttwo_route_accounts.json"
        ))
        .unwrap();
        let key = |v: &serde_json::Value| v.as_str().unwrap().parse::<Pubkey>().unwrap();
        let mut cache = SubscriptionAccountCache::default();
        for a in f["accounts"].as_array().unwrap() {
            cache
                .update(
                    key(&a["address"]),
                    CachedAccount {
                        owner: key(&a["owner"]),
                        data: STANDARD.decode(a["data_base64"].as_str().unwrap()).unwrap(),
                        slot: a["slot"].as_u64().unwrap(),
                        write_version: 0,
                    },
                )
                .unwrap();
        }
        let s = &f["route_search"];
        let hints = s["conversion_pools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| PoolTradeHint {
                pool: key(&p["pool"]),
                input_mint: key(&p["input_mint"]),
                output_mint: key(&p["output_mint"]),
            })
            .collect();
        let context = CacheReadContext {
            slot: f["slot"].as_u64().unwrap(),
            epoch: f["epoch"].as_u64().unwrap(),
            maximum_slot_age: 32,
        };
        let time = f["unix_timestamp"].as_u64().unwrap();
        (
            Arc::new(PumpSwapRouting::new(PumpSwapRoutingConfig::new(hints)).unwrap()),
            Arc::new(cache),
            context,
            time,
            key(&s["pool"]),
            key(&s["token_mint"]),
        )
    }
    fn client(routing: Arc<PumpSwapRouting>, url: &str) -> TradingClient {
        let config = TradeConfig::builder(url.into(), vec![], CommitmentConfig::processed())
            .pumpswap_routing(routing.clone())
            .create_wsol_ata_on_startup(false)
            .build();
        assert!(Arc::ptr_eq(config.pumpswap_routing.as_ref().unwrap(), &routing));
        let infrastructure = Arc::new(TradingInfrastructure {
            rpc: Arc::new(SolanaRpcClient::new(url.into())),
            swqos_clients: Arc::new(vec![]),
            config: InfrastructureConfig::from_trade_config(&config),
            max_sender_concurrency: 1,
            effective_core_ids: Arc::new(vec![]),
        });
        TradingClient::from_infrastructure(Arc::new(Keypair::new()), infrastructure, true)
            .with_pumpswap_routing(routing)
    }
    fn buy(pool: Pubkey, token: Pubkey, asset: TradeTokenType) -> SimpleBuyParams {
        // Only the pool identity is used: executable parameters must come from the cache.
        SimpleBuyParams::new(
            DexType::PumpSwap,
            asset,
            token,
            BuyAmount::ExactInput(1_000_000),
            DexParamEnum::PumpSwapAutoRoute(pool),
            Hash::new_unique(),
            GasFeeStrategy::new(),
        )
    }
    #[tokio::test]
    async fn pumpswap_configured_routing_is_rpc_free_and_repeatable() {
        let (routing, cache, context, time, pool, token) = fixture();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let c = client(routing.clone(), &format!("http://{}", listener.local_addr().unwrap()));
        routing.publish(cache, context, time);
        for asset in [TradeTokenType::SOL, TradeTokenType::WSOL] {
            for _ in 0..2 {
                let params = c.prepare_simple_buy(buy(pool, token, asset)).unwrap();
                assert!(params.rpc.is_none());
                assert!(params.create_input_mint_ata && params.create_output_mint_ata);
                let ixs = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
                let creates: Vec<_> =
                    ixs.iter().filter(|ix| ix.program_id == ASSOCIATED_TOKEN_PROGRAM_ID).collect();
                assert_eq!(creates.len(), 4);
                assert!(creates.iter().all(|ix| ix.data == [1]));
                let transfers = ixs.iter().filter(|ix| ix.program_id == Pubkey::default()).count();
                assert_eq!(transfers, usize::from(asset == TradeTokenType::SOL));
                assert!(!ixs.iter().any(|ix| ix.program_id == TOKEN_PROGRAM && ix.data == [9]));
            }
        }
        let mut request = buy(pool, token, TradeTokenType::SOL);
        request.account_policy = AccountPolicy::AssumePrepared;
        let params = c.prepare_simple_buy(request).unwrap();
        let ixs = PumpSwapInstructionBuilder.build_buy_instructions(&params).await.unwrap();
        assert!(!ixs.iter().any(|ix| ix.program_id == ASSOCIATED_TOKEN_PROGRAM_ID));
        // Exercise the actual high-level executor too. No sender is configured,
        // so it stops at submission, after the production instruction builder.
        let result = c.buy_simple(buy(pool, token, TradeTokenType::SOL)).await.unwrap();
        assert!(!result.0 && result.2.is_some());
        // No connect attempt is allowed even though the infrastructure has an RPC client.
        assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }
    #[test]
    fn pumpswap_configured_routing_rejects_missing_expired_and_stale_state() {
        let (routing, cache, context, time, pool, token) = fixture();
        let c = client(routing.clone(), "http://127.0.0.1:1");
        assert!(c
            .prepare_simple_buy(buy(pool, token, TradeTokenType::SOL))
            .unwrap_err()
            .to_string()
            .contains("not ready"));
        routing.snapshot.store(Some(Arc::new(Snapshot {
            cache: cache.clone(),
            context,
            unix_timestamp: time,
            received_at: Instant::now() - Duration::from_secs(10),
        })));
        assert!(c
            .prepare_simple_buy(buy(pool, token, TradeTokenType::SOL))
            .unwrap_err()
            .to_string()
            .contains("expired"));
        let mut stale = context;
        stale.slot += 100;
        routing.publish(cache.clone(), stale, time);
        assert!(c.prepare_simple_buy(buy(pool, token, TradeTokenType::SOL)).is_err());
        routing.publish(cache, context, time);
        assert!(c.prepare_simple_buy(buy(pool, token, TradeTokenType::SOL)).is_ok());
        routing.clear();
        assert!(c.prepare_simple_buy(buy(pool, token, TradeTokenType::SOL)).is_err());
    }
    #[test]
    fn pumpswap_configured_routing_validates_registry_and_requires_configuration() {
        let (routing, cache, context, time, pool, token) = fixture();
        let mut c = client(routing.clone(), "http://127.0.0.1:1");
        c.pumpswap_routing = None;
        assert!(c
            .prepare_simple_buy(buy(pool, token, TradeTokenType::SOL))
            .unwrap_err()
            .to_string()
            .contains("requires configured"));
        let mut invalid = PumpSwapRoutingConfig::new(vec![]);
        invalid.maximum_search_nodes = 0;
        assert!(PumpSwapRouting::new(invalid).is_err());
        let mut conflicting = routing.config.clone();
        let mut hint = conflicting.conversion_pools[0];
        hint.input_mint = Pubkey::new_unique();
        conflicting.conversion_pools.push(hint);
        assert!(PumpSwapRouting::new(conflicting).is_err());
        routing.publish(cache, context, time);
        let direct_quote = routing
            .prepare(pool, token, SOL_TOKEN_ACCOUNT, 1_000_000, 100, true)
            .unwrap()
            .steps()
            .last()
            .unwrap()
            .pool
            .input_mint;
        let direct = routing.prepare(pool, token, direct_quote, 10_000, 100, true).unwrap();
        assert_eq!(direct.steps().len(), 1);
        // A global registry may include the selected market itself; exclude it
        // from bridge search instead of rejecting an otherwise valid direct trade.
        let mut config = routing.config.clone();
        config.conversion_pools.push(PoolTradeHint {
            pool,
            input_mint: token,
            output_mint: direct_quote,
        });
        let same_market = PumpSwapRouting::new(config).unwrap();
        let snapshot = routing.snapshot.load_full().unwrap();
        same_market.publish(snapshot.cache.clone(), snapshot.context, snapshot.unix_timestamp);
        assert!(same_market.prepare(pool, token, SOL_TOKEN_ACCOUNT, 1_000_000, 100, true).is_ok());
        let clone = c.with_pumpswap_routing(routing.clone()).clone();
        assert!(Arc::ptr_eq(clone.pumpswap_routing.as_ref().unwrap(), &routing));
    }

    #[tokio::test]
    async fn pumpswap_configured_routing_sell_and_explicit_override_preserve_policy() {
        let (routing, cache, context, time, pool, token) = fixture();
        let c = client(routing.clone(), "http://127.0.0.1:1");
        routing.publish(cache, context, time);
        let prepared_buy = c.prepare_simple_buy(buy(pool, token, TradeTokenType::SOL)).unwrap();
        let mut override_request = buy(pool, token, TradeTokenType::SOL);
        override_request.extension_params = prepared_buy.protocol_params;
        routing.clear();
        // Explicit per-trade routes override the registry. Their fresh quotes are caller-owned.
        assert!(c.prepare_simple_buy(override_request.clone()).unwrap().rpc.is_none());
        override_request.simulate = true;
        assert!(c.prepare_simple_buy(override_request).is_err());
        let (routing, cache, context, time, _, _) = fixture();
        let c = c.with_pumpswap_routing(routing.clone());
        routing.publish(cache, context, time);
        let sell = SimpleSellParams::new(
            DexType::PumpSwap,
            TradeTokenType::SOL,
            token,
            SellAmount::ExactInput(1_000_000_000),
            DexParamEnum::PumpSwapAutoRoute(pool),
            Hash::new_unique(),
            GasFeeStrategy::new(),
        );
        let params = c.prepare_simple_sell(sell).unwrap();
        assert!(params.rpc.is_none() && params.close_output_mint_ata);
        let ixs = PumpSwapInstructionBuilder.build_sell_instructions(&params).await.unwrap();
        assert_eq!(
            ixs.iter().filter(|ix| ix.program_id == TOKEN_PROGRAM && ix.data == [9]).count(),
            1
        );
        for flag in [0, 1, 2] {
            let mut b = buy(pool, token, TradeTokenType::SOL);
            match flag {
                0 => b.simulate = true,
                1 => b.wait_tx_confirmed = true,
                _ => {
                    b.amount =
                        BuyAmount::ExactOutput { output_amount: 100, max_input_amount: 1_000_000 }
                }
            }
            assert!(c.prepare_simple_buy(b).is_err());
        }
        assert!(routing.prepare(pool, token, Pubkey::new_unique(), 1_000_000, 100, true).is_err());
        // SOL and WSOL normalize to the same route, but wrapping remains endpoint-specific.
        assert_eq!(
            routing
                .prepare(pool, token, SOL_TOKEN_ACCOUNT, 1_000_000, 100, true)
                .unwrap()
                .steps()
                .len(),
            routing
                .prepare(pool, token, WSOL_TOKEN_ACCOUNT, 1_000_000, 100, true)
                .unwrap()
                .steps()
                .len()
        );
    }
}
