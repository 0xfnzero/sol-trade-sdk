//! Instance-scoped bridge markets with atomically published subscription snapshots.
use crate::trading::core::params::{
    canonical_conversion_pools, CacheReadContext, CachedQuoteRequest, ConversionGraph,
    PoolTradeHint, PreparedPumpSwapTrade, PumpSwapRouteSearch, SubscriptionAccountCache,
};
use anyhow::{ensure, Result};
use arc_swap::ArcSwapOption;
use solana_sdk::pubkey::Pubkey;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Register markets once; refresh their account state outside the trading path.
/// Only protocols already supported by the cached quote engine can be added.
#[derive(Clone, Debug)]
pub struct PumpSwapRoutingConfig {
    pub conversion_pools: Vec<PoolTradeHint>,
    /// Per conversion hop execution tolerance in bps (100 = 1%).
    pub bridge_slippage_basis_points: u16,
    /// Per conversion hop price deterioration vs a 1/100-size cached probe.
    pub maximum_bridge_price_impact_basis_points: u16,
    pub maximum_conversion_hops: usize,
    pub maximum_search_nodes: usize,
    pub maximum_arrays: usize,
    pub maximum_slot_age: u64,
    /// Local timeout also rejects a stopped subscription whose slot never advances.
    pub maximum_snapshot_age: Duration,
}
impl Default for PumpSwapRoutingConfig {
    /// Mainnet bridge identities screened on 2026-10-20. No network access.
    /// State must still be subscribed and published; defaults are not a depth guarantee.
    /// See docs/PUMPSWAP_BRIDGE_DEFAULTS.md for sampling and admission criteria.
    fn default() -> Self {
        use solana_sdk::pubkey;
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../stock_token_catalog.json"))
                .expect("tested shared stock catalog");
        let mut pools = vec![PoolTradeHint {
            pool: pubkey!("58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"),
            input_mint: crate::constants::WSOL_TOKEN_ACCOUNT,
            output_mint: crate::constants::USDC_TOKEN_ACCOUNT,
        }];
        for row in catalog["pools"].as_array().expect("catalog pools") {
            let key = |field: &str| row[field].as_str().unwrap().parse().expect("catalog pubkey");
            pools.push(PoolTradeHint {
                pool: key("pool"),
                input_mint: key("quote_mint"),
                output_mint: key("mint"),
            });
        }
        Self::new(pools)
    }
}
impl PumpSwapRoutingConfig {
    /// Use only the supplied markets (no built-in defaults).
    pub fn new(conversion_pools: Vec<PoolTradeHint>) -> Self {
        Self {
            conversion_pools,
            bridge_slippage_basis_points: 100,
            maximum_bridge_price_impact_basis_points: 100,
            maximum_conversion_hops: 2,
            maximum_search_nodes: 512,
            maximum_arrays: 6,
            maximum_slot_age: 32,
            maximum_snapshot_age: Duration::from_secs(2),
        }
    }

    /// Append custom markets, deduplicating by pool address. Reversed pairs and
    /// SOL/WSOL aliases are equivalent; conflicting identities fail atomically.
    /// The 128-market budget applies after deduplication.
    pub fn extend_pools(&mut self, pools: impl IntoIterator<Item = PoolTradeHint>) -> Result<()> {
        let combined = self.conversion_pools.iter().copied().chain(pools);
        self.conversion_pools = canonical_conversion_pools(combined)?;
        Ok(())
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
    graph: ConversionGraph,
    snapshot: ArcSwapOption<Snapshot>,
    invalidation_generation: AtomicU64,
}
impl std::fmt::Debug for PumpSwapRouting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PumpSwapRouting").field("config", &self.config).finish_non_exhaustive()
    }
}
impl PumpSwapRouting {
    pub fn new(mut config: PumpSwapRoutingConfig) -> Result<Self> {
        config.conversion_pools = canonical_conversion_pools(config.conversion_pools)?;
        ensure!(
            config.bridge_slippage_basis_points < 10_000
                && config.maximum_bridge_price_impact_basis_points < 10_000,
            "Invalid bridge slippage or price impact limit"
        );
        ensure!(config.maximum_conversion_hops <= 3, "Too many bridge hops");
        ensure!((1..=4096).contains(&config.maximum_search_nodes), "Invalid search budget");
        ensure!((1..=32).contains(&config.maximum_arrays), "Invalid array budget");
        ensure!(!config.maximum_snapshot_age.is_zero(), "Snapshot timeout must be positive");
        let graph = ConversionGraph::new(&config.conversion_pools)?;
        Ok(Self {
            config,
            graph,
            snapshot: ArcSwapOption::empty(),
            invalidation_generation: AtomicU64::new(0),
        })
    }

    /// Normalized, deduplicated identities for background subscription setup.
    pub fn conversion_pools(&self) -> &[PoolTradeHint] {
        &self.config.conversion_pools
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
        self.invalidation_generation.fetch_add(1, Ordering::SeqCst);
    }

    fn ensure_generation(&self, generation: u64) -> Result<()> {
        ensure!(
            self.snapshot.load().is_some()
                && self.invalidation_generation.load(Ordering::SeqCst) == generation,
            "PumpSwap routing snapshot invalidated during search"
        );
        Ok(())
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
        let generation = self.invalidation_generation.load(Ordering::SeqCst);
        let snapshot_guard = self.snapshot.load();
        let snapshot = snapshot_guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("PumpSwap routing snapshot is not ready"))?;
        ensure!(
            snapshot.received_at.elapsed() <= self.config.maximum_snapshot_age,
            "PumpSwap routing snapshot expired"
        );
        let mut search = PumpSwapRouteSearch::new(pool, token, asset, Vec::new());
        search.bridge_slippage_basis_points = self.config.bridge_slippage_basis_points;
        search.maximum_bridge_price_impact_basis_points =
            self.config.maximum_bridge_price_impact_basis_points;
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
        let selected = snapshot.cache.prepare_best_pumpswap_trade_with_graph(
            &search,
            request,
            context,
            is_buy,
            &self.graph,
        )?;
        ensure!(
            snapshot.received_at.elapsed() <= self.config.maximum_snapshot_age,
            "PumpSwap routing snapshot expired during search"
        );
        self.ensure_generation(generation)?;
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

    #[test]
    fn defaults_include_every_shared_stock_pool_and_the_sol_usdc_bridge() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../stock_token_catalog.json")).unwrap();
        let config = PumpSwapRoutingConfig::default();
        let rows = catalog["pools"].as_array().unwrap();
        assert_eq!(config.conversion_pools.len(), rows.len() + 1);
        assert!(rows.iter().any(|row| row["preferred"] == false));
        for row in rows {
            let key = |field: &str| row[field].as_str().unwrap().parse::<Pubkey>().unwrap();
            let matches: Vec<_> =
                config.conversion_pools.iter().filter(|hint| hint.pool == key("pool")).collect();
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].input_mint, key("quote_mint"));
            assert_eq!(matches[0].output_mint, key("mint"));
            assert!(row["liquid_side_reserve_usd"].as_f64().unwrap() >= 500.0);
        }
        assert_eq!(
            config
                .conversion_pools
                .iter()
                .filter(|hint| hint.pool == config_pool_wsol_usdc())
                .count(),
            1
        );
        PumpSwapRouting::new(config).unwrap();
    }

    #[test]
    fn preferred_pool_mints_match_mainnet_accounts_and_official_issuer_records() {
        let evidence: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/research/bridge-defaults-20261020-expanded.json"
        ))
        .unwrap();
        let mut official = Vec::<Pubkey>::new();
        for entry in evidence["issuer_records"].as_array().unwrap() {
            let record = &entry["record"];
            match entry["source"].as_str().unwrap() {
                "https://api.backpack.exchange/api/v1/assets" => {
                    for token in record["tokens"].as_array().unwrap() {
                        if token["blockchain"] == "Solana" {
                            official
                                .push(token["contractAddress"].as_str().unwrap().parse().unwrap());
                        }
                    }
                }
                "https://xstocks.com/products" => {
                    official.push(record["addresses"]["solana"].as_str().unwrap().parse().unwrap());
                }
                other => panic!("Unrecognized issuer source: {other}"),
            }
        }
        let config = PumpSwapRoutingConfig::default();
        let pools = evidence["pools"].as_array().unwrap();
        // This capture covers the historical preferred subset; all alternatives
        // are checked against the shared screened catalog in the parity test.
        assert_eq!(
            config
                .conversion_pools
                .iter()
                .filter(|hint| pools
                    .iter()
                    .any(|p| p["pool"].as_str() == Some(&hint.pool.to_string())))
                .count(),
            pools.len()
        );
        for hint in &config.conversion_pools {
            for mint in [hint.input_mint, hint.output_mint] {
                assert!(
                    [WSOL_TOKEN_ACCOUNT, crate::constants::USDC_TOKEN_ACCOUNT].contains(&mint)
                        || official.contains(&mint),
                    "Unverified default mint: {mint}"
                );
            }
        }
        assert!(config.conversion_pools.len() <= 128);
        for hint in config
            .conversion_pools
            .into_iter()
            .filter(|hint| pools.iter().any(|p| p["pool"].as_str() == Some(&hint.pool.to_string())))
        {
            let captured =
                pools.iter().find(|p| p["pool"].as_str() == Some(&hint.pool.to_string())).unwrap();
            let account = &captured["pool_account"];
            let data = STANDARD.decode(account["data"][0].as_str().unwrap()).unwrap();
            let owner: Pubkey = account["owner"].as_str().unwrap().parse().unwrap();
            use crate::instruction::utils::{
                meteora_dlmm, raydium_amm_v4_types, raydium_clmm, raydium_cpmm, raydium_cpmm_types,
                whirlpool,
            };
            let (a, b) = if owner == meteora_dlmm::PROGRAM_ID {
                let p = meteora_dlmm::decode_lb_pair(&data).unwrap();
                (p.token_x_mint, p.token_y_mint)
            } else if owner == raydium_clmm::PROGRAM_ID {
                let p = raydium_clmm::decode_pool_state(&data).unwrap();
                (p.token_mint_0, p.token_mint_1)
            } else if owner == whirlpool::PROGRAM_ID {
                let p = whirlpool::decode_whirlpool(&data).unwrap();
                (p.token_mint_a, p.token_mint_b)
            } else if owner == raydium_cpmm::accounts::RAYDIUM_CPMM {
                assert_eq!(data[..8], raydium_cpmm_types::POOL_STATE_DISCRIMINATOR);
                let p = raydium_cpmm_types::pool_state_decode(&data[8..]).unwrap();
                (p.token0_mint, p.token1_mint)
            } else {
                assert_eq!(owner.to_string(), "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");
                let p = raydium_amm_v4_types::amm_info_decode(&data).unwrap();
                (p.coin_mint, p.pc_mint)
            };
            assert_eq!(
                (a.min(b), a.max(b)),
                (hint.input_mint.min(hint.output_mint), hint.input_mint.max(hint.output_mint))
            );
            for mint in [a, b] {
                assert!(
                    [WSOL_TOKEN_ACCOUNT, crate::constants::USDC_TOKEN_ACCOUNT].contains(&mint)
                        || official.contains(&mint),
                    "Unverified default stock mint: {mint}"
                );
            }
            if hint.pool != config_pool_wsol_usdc() {
                let audit = &captured["depth"]["reports"][0]["audit"];
                assert_eq!(audit["eligible"], true);
                assert!(audit["liquid_side_reserve_usd"].as_f64().unwrap() >= 500.0);
                // Catalog membership must respect fee preference after depth admission.
                let scan: serde_json::Value = serde_json::from_str(include_str!(
                    "../../docs/research/bridge-defaults-20261020-expanded-scan.json"
                ))
                .unwrap();
                let fee = captured["fees"]["snapshot_start_fee_bps"].as_f64().unwrap();
                assert_eq!(captured["fees"]["stock_transfer_fee"]["basis_points"], 0);
                for candidate in scan["candidate_results"].as_array().unwrap() {
                    if candidate["eligible"] == true
                        && candidate["mint"].as_str() == Some(&hint.output_mint.to_string())
                    {
                        let other_fee =
                            candidate["fees"]["snapshot_start_fee_bps"].as_f64().unwrap();
                        assert!(fee <= other_fee, "A cheaper qualifying pool was skipped");
                        if fee == other_fee {
                            assert!(
                                audit["liquid_side_reserve_usd"].as_f64().unwrap()
                                    >= candidate["depth"]["reports"][0]["audit"]
                                        ["liquid_side_reserve_usd"]
                                        .as_f64()
                                        .unwrap()
                            );
                        }
                    }
                }
                assert!(audit["buy_output"].as_u64().unwrap() > 0);
                assert!(audit["sell_output"].as_u64().unwrap() > 0);
            }
        }
    }

    fn config_pool_wsol_usdc() -> Pubkey {
        solana_sdk::pubkey!("58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2")
    }

    #[test]
    fn registry_extensions_are_atomic_normalized_and_deduplicated_before_limits() {
        let mut config = PumpSwapRoutingConfig::default();
        let original = config.conversion_pools.len();
        let hint = config.conversion_pools[0];
        let reverse =
            PoolTradeHint { input_mint: hint.output_mint, output_mint: SOL_TOKEN_ACCOUNT, ..hint };
        config.extend_pools(std::iter::repeat(reverse).take(64)).unwrap();
        assert_eq!(config.conversion_pools.len(), original);
        let custom = PoolTradeHint { pool: Pubkey::new_unique(), ..hint };
        config.extend_pools([custom, custom]).unwrap();
        assert_eq!(config.conversion_pools.len(), original + 1);
        let conflict = PoolTradeHint { output_mint: Pubkey::new_unique(), ..hint };
        let extra = PoolTradeHint { pool: Pubkey::new_unique(), ..hint };
        assert!(config.extend_pools([extra, conflict]).is_err());
        assert_eq!(config.conversion_pools.len(), original + 1);
        assert!(!config.conversion_pools.iter().any(|p| p.pool == extra.pool));
        assert!(config
            .extend_pools((0..129).map(|_| PoolTradeHint { pool: Pubkey::new_unique(), ..hint }))
            .is_err());
        assert_eq!(config.conversion_pools.len(), original + 1);
        // Direct public-field construction also goes through the same validation.
        let routing = PumpSwapRouting::new(PumpSwapRoutingConfig::new(vec![reverse; 64])).unwrap();
        assert_eq!(routing.conversion_pools().len(), 1);
        assert_eq!(routing.conversion_pools()[0].output_mint, WSOL_TOKEN_ACCOUNT);
        assert!(PumpSwapRouting::new(PumpSwapRoutingConfig::new(vec![]))
            .unwrap()
            .conversion_pools()
            .is_empty());
    }

    #[test]
    fn defaults_allow_cached_routes_without_unrelated_default_accounts() {
        let (existing, cache, ctx, time, pool, token) = fixture();
        let mut config = PumpSwapRoutingConfig::default();
        let original = config.conversion_pools.len();
        config.extend_pools(existing.config.conversion_pools.clone()).unwrap();
        assert_eq!(config.conversion_pools.len(), original);
        let routing = PumpSwapRouting::new(config).unwrap();
        routing.publish(cache, ctx, time);
        assert!(routing.prepare(pool, token, SOL_TOKEN_ACCOUNT, 1_000_000, 100, true).is_ok());
    }

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
    #[test]
    fn compiled_graph_matches_public_search_for_both_directions() {
        let (routing, cache, context, time, pool, token) = fixture();
        routing.publish(cache.clone(), context, time);
        for is_buy in [true, false] {
            for amount in [100_000, 1_000_000] {
                let search = PumpSwapRouteSearch::new(
                    pool,
                    token,
                    SOL_TOKEN_ACCOUNT,
                    routing.conversion_pools().to_vec(),
                );
                let expected = cache.prepare_best_pumpswap_trade(
                    &search,
                    CachedQuoteRequest::new(amount, time),
                    context,
                    is_buy,
                );
                let actual = routing.prepare(pool, token, SOL_TOKEN_ACCOUNT, amount, 100, is_buy);
                match (expected, actual) {
                    (Ok(expected), Ok(actual)) => {
                        assert_eq!(
                            expected.trade.preview().minimum_output_credit,
                            actual.preview().minimum_output_credit,
                        );
                        let pools = |trade: &PreparedPumpSwapTrade| {
                            trade.steps().iter().map(|s| s.pool.pool).collect::<Vec<_>>()
                        };
                        assert_eq!(pools(&expected.trade), pools(&actual));
                    }
                    (Err(expected), Err(actual)) => {
                        assert_eq!(expected.to_string(), actual.to_string())
                    }
                    _ => panic!("compiled routing and public search disagree"),
                }
            }
        }
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
    fn clearing_rejects_retained_search_even_after_republication() {
        let (routing, cache, context, time, _, _) = fixture();
        routing.publish(cache.clone(), context, time);
        let generation = routing.invalidation_generation.load(Ordering::SeqCst);
        let retained = routing.snapshot.load_full().unwrap();
        // A normal update does not invalidate an otherwise fresh coherent search.
        routing.publish(cache.clone(), context, time);
        routing.ensure_generation(generation).unwrap();
        routing.clear();
        assert!(routing.ensure_generation(generation).is_err());
        routing.publish(cache, context, time);
        assert!(routing.ensure_generation(generation).is_err());
        assert!(retained.received_at.elapsed() < Duration::from_secs(10));
        routing.ensure_generation(routing.invalidation_generation.load(Ordering::SeqCst)).unwrap();
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
