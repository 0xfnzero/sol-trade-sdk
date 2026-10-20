//! Bounded route selection from caller-approved cached conversion pools.
use super::{
    pumpswap_preparation::canonical, CacheReadContext, CachedQuoteRequest, CachedRouteStep,
    PoolTradeHint, PreparedPumpSwapTrade, SubscriptionAccountCache,
};
use crate::instruction::utils::{pumpswap as ps, pumpswap_types as types};
use anyhow::{anyhow, ensure, Context, Result};
use smallvec::{smallvec, SmallVec};
use solana_sdk::pubkey::Pubkey;
use std::collections::BTreeMap;

/// One selected PumpSwap market plus an unordered allowlist of conversion pools.
/// Hints describe either mint orientation; both directions are considered and
/// verified against cached state. This does not discover pools over the network.
#[derive(Clone, Debug)]
pub struct PumpSwapRouteSearch {
    pub pool: Pubkey,
    /// The token being bought/sold; may be either side of the selected pool.
    pub token_mint: Pubkey,
    /// Payment/receipt asset. Native SOL and existing WSOL remain distinct here.
    pub asset_mint: Pubkey,
    pub conversion_pools: Vec<PoolTradeHint>,
    /// Excludes the selected PumpSwap leg. Supported range: 0..=3.
    pub maximum_conversion_hops: usize,
    /// Execution tolerance per conversion hop; 100 bps by default.
    pub bridge_slippage_basis_points: u16,
    /// Maximum deterioration vs a 1/100-size probe per conversion hop.
    pub maximum_bridge_price_impact_basis_points: u16,
    /// Bounds all explored partial paths, including dead ends. Range: 1..=4096.
    /// Exhaustion returns an error even if a valid route was already found.
    pub maximum_search_nodes: usize,
}

impl PumpSwapRouteSearch {
    pub fn new(
        pool: Pubkey,
        token_mint: Pubkey,
        asset_mint: Pubkey,
        conversion_pools: Vec<PoolTradeHint>,
    ) -> Self {
        Self {
            pool,
            token_mint,
            asset_mint,
            conversion_pools,
            maximum_conversion_hops: 2,
            bridge_slippage_basis_points: 100,
            maximum_bridge_price_impact_basis_points: 100,
            maximum_search_nodes: 512,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PumpSwapRouteRejection {
    pub pools: Vec<Pubkey>,
    pub reason: String,
}

/// Best protected output among fully evaluated simple paths within the supplied
/// hop limit. Does not split orders, estimate SOL gas/rent, or guarantee execution.
#[derive(Clone)]
pub struct PumpSwapRouteSelection {
    pub trade: PreparedPumpSwapTrade,
    pub routes_evaluated: usize,
    pub rejected_routes: Vec<PumpSwapRouteRejection>,
}

pub(crate) fn canonical_conversion_pools(
    pools: impl IntoIterator<Item = PoolTradeHint>,
) -> Result<Vec<PoolTradeHint>> {
    let mut seen = std::collections::HashMap::new();
    let mut unique = Vec::new();
    for hint in pools {
        let a = canonical(hint.input_mint);
        let b = canonical(hint.output_mint);
        ensure!(
            hint.pool != Pubkey::default()
                && a != b
                && a != Pubkey::default()
                && b != Pubkey::default(),
            "Invalid bridge pool identity"
        );
        let pair = (a.min(b), a.max(b));
        if let Some(previous) = seen.insert(hint.pool, pair) {
            ensure!(previous == pair, "Conflicting bridge pool identity");
        } else {
            unique.push(PoolTradeHint { pool: hint.pool, input_mint: a, output_mint: b });
            ensure!(unique.len() <= 128, "Too many bridge pools");
        }
    }
    Ok(unique)
}

/// Immutable adjacency, validated and sorted once for a routing instance.
pub(crate) struct ConversionGraph {
    adjacency: BTreeMap<Pubkey, Vec<PoolTradeHint>>,
}
impl ConversionGraph {
    pub(crate) fn new(pools: &[PoolTradeHint]) -> Result<Self> {
        ensure!(pools.len() <= 128, "PumpSwap search accepts at most 128 conversion pool hints");
        let mut candidates = canonical_conversion_pools(pools.iter().copied())?;
        // Preserve deterministic traversal and tie-breaking by pool identity.
        candidates.sort_unstable_by_key(|hint| hint.pool);
        let mut adjacency: BTreeMap<Pubkey, Vec<PoolTradeHint>> = BTreeMap::new();
        for hint in candidates {
            adjacency.entry(hint.input_mint).or_default().push(hint);
            adjacency.entry(hint.output_mint).or_default().push(hint.reversed());
        }
        Ok(Self { adjacency })
    }
}

struct PathSearch<'a> {
    graph: &'a ConversionGraph,
    excluded_pool: Pubkey,
    target: Pubkey,
    excluded_mint: Pubkey,
    maximum_hops: usize,
    nodes_remaining: usize,
    // At most three conversion hops plus the selected PumpSwap leg.
    paths: Vec<SmallVec<[PoolTradeHint; 4]>>,
}

impl PathSearch<'_> {
    fn walk(
        &mut self,
        mint: Pubkey,
        path: &mut SmallVec<[PoolTradeHint; 4]>,
        visited: &mut SmallVec<[Pubkey; 4]>,
    ) -> Result<()> {
        ensure!(self.nodes_remaining > 0,
            "PumpSwap route search budget exhausted; reduce candidates/hops or increase maximum_search_nodes");
        self.nodes_remaining -= 1;
        if mint == self.target {
            self.paths.push(path.clone());
            return Ok(());
        }
        if path.len() == self.maximum_hops {
            return Ok(());
        }
        for pool in self.graph.adjacency.get(&mint).into_iter().flatten() {
            if pool.pool == self.excluded_pool {
                continue;
            }
            let output = pool.output_mint;
            if output == self.excluded_mint || visited.contains(&output) {
                continue;
            }
            visited.push(output);
            path.push(PoolTradeHint { input_mint: mint, output_mint: output, ..*pool });
            self.walk(output, path, visited)?;
            path.pop();
            visited.pop();
        }
        Ok(())
    }
}

impl SubscriptionAccountCache {
    /// Find a route for one independent buy/sell using fresh quotes for every
    /// connected candidate path. Invalid/stale candidates are reported and skipped;
    /// invalid requests, conflicting identities and incomplete searches fail.
    /// Equal output prefers fewer hops, then lexicographic pool pubkey byte order.
    pub fn prepare_best_pumpswap_trade(
        &self,
        search: &PumpSwapRouteSearch,
        request: CachedQuoteRequest,
        ctx: CacheReadContext,
        is_buy: bool,
    ) -> Result<PumpSwapRouteSelection> {
        ensure!(
            search.conversion_pools.iter().all(|hint| hint.pool != search.pool),
            "Conversion candidates must not reuse the selected PumpSwap pool"
        );
        let graph = ConversionGraph::new(&search.conversion_pools)?;
        self.prepare_best_pumpswap_trade_with_graph(search, request, ctx, is_buy, &graph)
    }

    pub(crate) fn prepare_best_pumpswap_trade_with_graph(
        &self,
        search: &PumpSwapRouteSearch,
        request: CachedQuoteRequest,
        ctx: CacheReadContext,
        is_buy: bool,
        graph: &ConversionGraph,
    ) -> Result<PumpSwapRouteSelection> {
        ensure!(
            search.conversion_pools.len() <= 128,
            "PumpSwap search accepts at most 128 conversion pool hints"
        );
        ensure!(
            search.bridge_slippage_basis_points < 10_000
                && search.maximum_bridge_price_impact_basis_points < 10_000
                && search.maximum_conversion_hops <= 3
                && (1..=4096).contains(&search.maximum_search_nodes),
            "Invalid PumpSwap route search limits"
        );
        ensure!(
            request.amount_in > 0
                && request.slippage_basis_points < 10_000
                && (1..=32).contains(&request.maximum_arrays),
            "Invalid cached quote request"
        );
        let token = canonical(search.token_mint);
        let asset = canonical(search.asset_mint);
        ensure!(token != asset, "PumpSwap payment/receipt asset must differ from the traded token");
        let data = self.get(search.pool, ps::accounts::AMM_PROGRAM, ctx)?;
        ensure!(
            data.get(..8) == Some(types::POOL_DISCRIMINATOR.as_slice()),
            "Invalid cached PumpSwap pool discriminator"
        );
        let pool = types::pool_decode(&data[8..])
            .ok_or_else(|| anyhow!("Invalid cached PumpSwap pool"))?;
        let quote = if token == pool.base_mint {
            pool.quote_mint
        } else {
            ensure!(
                token == pool.quote_mint,
                "Traded token does not belong to selected PumpSwap pool"
            );
            pool.base_mint
        };
        let pump = PoolTradeHint {
            pool: search.pool,
            input_mint: if is_buy { quote } else { token },
            output_mint: if is_buy { token } else { quote },
        };
        self.pumpswap(pump, ctx).context("Selected PumpSwap pool is not ready")?;

        let (from, to) = if is_buy { (asset, quote) } else { (quote, asset) };
        let mut paths = PathSearch {
            graph,
            excluded_pool: search.pool,
            target: to,
            excluded_mint: token,
            maximum_hops: search.maximum_conversion_hops,
            nodes_remaining: search.maximum_search_nodes,
            paths: Vec::new(),
        };
        paths.walk(from, &mut SmallVec::new(), &mut smallvec![from])?;
        ensure!(
            !paths.paths.is_empty(),
            "No conversion path from {from} to {to} within the supplied candidates and hop limit"
        );
        let routes_evaluated = paths.paths.len();
        let mut best: Option<PreparedPumpSwapTrade> = None;
        let mut rejected_routes = Vec::new();
        for mut path in paths.paths {
            if is_buy {
                path.push(pump);
            } else {
                path.insert(0, pump);
            }
            let steps: SmallVec<[_; 4]> =
                path.iter().map(|h| CachedRouteStep { pool: *h, input_amount: None }).collect();
            let (input, output) = if is_buy {
                (search.asset_mint, search.token_mint)
            } else {
                (search.token_mint, search.asset_mint)
            };
            match self.prepare_pumpswap_trade_with_bridge_limits(
                &steps,
                input,
                output,
                request,
                ctx,
                is_buy,
                Some(super::cached_quote::BridgeQuoteLimits {
                    slippage_basis_points: search.bridge_slippage_basis_points,
                    maximum_price_impact_basis_points: search
                        .maximum_bridge_price_impact_basis_points,
                    target_pool: search.pool,
                }),
            ) {
                Ok(trade) => {
                    let better = best.as_ref().is_none_or(|previous| {
                        let output = trade.preview().minimum_output_credit;
                        let old_output = previous.preview().minimum_output_credit;
                        output > old_output
                            || (output == old_output
                                && trade.steps().len() < previous.steps().len())
                    });
                    if better {
                        best = Some(trade);
                    }
                }
                Err(error) => rejected_routes.push(PumpSwapRouteRejection {
                    pools: path.iter().map(|h| h.pool).collect(),
                    reason: format!("{error:#}"),
                }),
            }
        }
        let trade = best.ok_or_else(|| {
            anyhow!(
                "No executable PumpSwap route among {routes_evaluated} paths: {}",
                rejected_routes.first().map(|r| r.reason.as_str()).unwrap_or("no quote")
            )
        })?;
        Ok(PumpSwapRouteSelection { trade, routes_evaluated, rejected_routes })
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn usdc_input_starts_at_usdc_and_never_loops_back() {
        let usdc = crate::constants::USDC_TOKEN_ACCOUNT;
        let wsol = crate::constants::WSOL_TOKEN_ACCOUNT;
        let stock = Pubkey::new_unique();
        let pools = vec![
            PoolTradeHint { pool: Pubkey::new_unique(), input_mint: wsol, output_mint: usdc },
            PoolTradeHint { pool: Pubkey::new_unique(), input_mint: usdc, output_mint: stock },
            PoolTradeHint { pool: Pubkey::new_unique(), input_mint: wsol, output_mint: stock },
        ];
        let graph = ConversionGraph::new(&pools).unwrap();
        let mut search = PathSearch {
            graph: &graph,
            excluded_pool: Pubkey::default(),
            target: stock,
            excluded_mint: Pubkey::new_unique(),
            maximum_hops: 3,
            nodes_remaining: 512,
            paths: Vec::new(),
        };
        search.walk(usdc, &mut SmallVec::new(), &mut smallvec![usdc]).unwrap();
        assert_eq!(search.paths.len(), 2);
        assert!(search.paths.iter().any(|p| p.len() == 1 && p[0].pool == pools[1].pool));
        assert!(search.paths.iter().any(|p| p.len() == 2 && p[0].output_mint == wsol));
        for path in search.paths {
            assert_eq!(path[0].input_mint, usdc);
            assert!(path.iter().all(|hop| hop.output_mint != usdc));
            assert_eq!(path.last().unwrap().output_mint, stock);
        }
    }
}
