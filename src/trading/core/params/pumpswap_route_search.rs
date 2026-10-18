//! Bounded route selection from caller-approved cached conversion pools.
use super::{
    pumpswap_preparation::canonical, CacheReadContext, CachedQuoteRequest, CachedRouteStep,
    PoolTradeHint, PreparedPumpSwapTrade, SubscriptionAccountCache,
};
use crate::instruction::utils::{pumpswap as ps, pumpswap_types as types};
use anyhow::{anyhow, ensure, Context, Result};
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
            maximum_search_nodes: 128,
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

struct PathSearch<'a> {
    pools: &'a [PoolTradeHint],
    target: Pubkey,
    excluded_mint: Pubkey,
    maximum_hops: usize,
    nodes_remaining: usize,
    paths: Vec<Vec<PoolTradeHint>>,
}

impl PathSearch<'_> {
    fn walk(
        &mut self,
        mint: Pubkey,
        path: &mut Vec<PoolTradeHint>,
        visited: &mut Vec<Pubkey>,
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
        for pool in self.pools {
            let output = if mint == pool.input_mint {
                pool.output_mint
            } else if mint == pool.output_mint {
                pool.input_mint
            } else {
                continue;
            };
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
            search.conversion_pools.len() <= 32,
            "PumpSwap search accepts at most 32 conversion pool hints"
        );
        ensure!(
            search.maximum_conversion_hops <= 3
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

        // Canonical sorting makes ties deterministic regardless of hint order.
        // Duplicate/reversed hints collapse; conflicting identities are rejected.
        let mut candidates = BTreeMap::new();
        for hint in &search.conversion_pools {
            ensure!(
                hint.pool != search.pool,
                "Conversion candidates must not reuse the selected PumpSwap pool"
            );
            let a = canonical(hint.input_mint);
            let b = canonical(hint.output_mint);
            ensure!(
                hint.pool != Pubkey::default()
                    && a != b
                    && a != Pubkey::default()
                    && b != Pubkey::default(),
                "Invalid conversion pool identity"
            );
            let pair = (a.min(b), a.max(b));
            if let Some(previous) = candidates.insert(hint.pool, pair) {
                ensure!(
                    previous == pair,
                    "Conflicting conversion mint pairs for pool {}",
                    hint.pool
                );
            }
        }
        let candidates: Vec<_> = candidates
            .into_iter()
            .map(|(pool, (input_mint, output_mint))| PoolTradeHint {
                pool,
                input_mint,
                output_mint,
            })
            .collect();
        let (from, to) = if is_buy { (asset, quote) } else { (quote, asset) };
        let mut paths = PathSearch {
            pools: &candidates,
            target: to,
            excluded_mint: token,
            maximum_hops: search.maximum_conversion_hops,
            nodes_remaining: search.maximum_search_nodes,
            paths: Vec::new(),
        };
        paths.walk(from, &mut Vec::new(), &mut vec![from])?;
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
            let steps: Vec<_> =
                path.iter().map(|h| CachedRouteStep { pool: *h, input_amount: None }).collect();
            let (input, output) = if is_buy {
                (search.asset_mint, search.token_mint)
            } else {
                (search.token_mint, search.asset_mint)
            };
            match self.prepare_pumpswap_trade(&steps, input, output, request, ctx, is_buy) {
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
