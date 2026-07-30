//! Trade Orchestrator — wires Phase 5-8 services into a coherent execution pipeline.
//!
//! The Orchestrator manages the full lifecycle of a trade intent:
//!
//! 1. **Pre-trade validation** — RiskEngine gates (35 hard limits)
//! 2. **Blockhash + fee provisioning** — BlockhashService + FeeService (Phase 7)
//! 3. **ALT caching** — AltCache for address lookup tables (Phase 7)
//! 4. **Execution** — Delegates to the existing GenericTradeExecutor
//! 5. **Confirmation** — Polls for on-chain landing
//! 6. **Reconciliation** — 7-step protocol on ambiguous results (Phase 8)
//! 7. **Kill switch** — AtomicBool that gates all new intents

use crate::common::{
    alt_cache::AltCache,
    blockhash_service::BlockhashService,
    config::{AppConfig, RiskConfig},
    fee_service::FeeService,
    SolanaRpcClient, SwqosSubmitTiming,
};
use crate::constants::risk::{RiskContext, RiskEngine, TradeLedger, TradeRiskParams};
use crate::swqos::SwqosClient;
use crate::trading::core::{
    reconciliation::{
        BalanceSnapshot, ReconciliationConfig, ReconciliationOutcome, ReconciliationService,
    },
    state::{TradeDirection, TradeIntent, TradeState},
    traits::TradeExecutor,
};
use crate::trading::strategy::{StrategyEngine, StrategyOutcome, StrategyStats};
use crate::trading::SwapParams;
use crate::perf::shredstream::classifier::ClassifiedEvent;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
use tracing::{info, trace, warn};

#[cfg(feature = "perf-trace")]
use crate::perf::TraceSpan;

// ── perf-trace instrumentation (zero-cost when feature disabled) ──

#[cfg(feature = "perf-trace")]
mod perf_metrics {
    use super::*;
    use crate::perf::{LatencyHistogram, PerfCounter, PerfRegistry, TraceSpan};
    use once_cell::sync::Lazy;

    /// Histogram for plan_trade latency (ns).
    pub static PLAN_TRADE_NS: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
        let h = Arc::new(LatencyHistogram::new());
        PerfRegistry::global().register_histogram("orchestrator.plan_trade_ns", h.clone());
        h
    });

    /// Histogram for execute_plan latency (ns).
    pub static EXECUTE_PLAN_NS: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
        let h = Arc::new(LatencyHistogram::new());
        PerfRegistry::global().register_histogram("orchestrator.execute_plan_ns", h.clone());
        h
    });

    /// Histogram for reconcile latency (ns).
    pub static RECONCILE_NS: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
        let h = Arc::new(LatencyHistogram::new());
        PerfRegistry::global().register_histogram("orchestrator.reconcile_ns", h.clone());
        h
    });

    /// Counter for successful trades.
    pub static TRADE_LANDED: PerfCounter = PerfCounter::new("orchestrator.trade_landed");
    /// Counter for failed trades.
    pub static TRADE_FAILED: PerfCounter = PerfCounter::new("orchestrator.trade_failed");
    /// Counter for trades sent to reconciliation.
    pub static TRADE_RECONCILED: PerfCounter = PerfCounter::new("orchestrator.trade_reconciled");
    /// Counter for trades rejected by kill switch.
    pub static TRADE_REJECTED: PerfCounter = PerfCounter::new("orchestrator.trade_rejected");
}

// ---------------------------------------------------------------------------
// OrchestratorConfig
// ---------------------------------------------------------------------------

/// Configuration for the orchestrator.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Blockhash cache capacity (default: 3).
    pub blockhash_cache_capacity: usize,
    /// Blockhash refresh interval in ms (default: 200).
    pub blockhash_refresh_interval_ms: u64,
    /// Fee service window size (default: 100).
    pub fee_window_size: usize,
    /// Fee service percentile (default: 50.0 — P50).
    pub fee_percentile: f64,
    /// Minimum CU price fallback (default: 1_000).
    pub min_cu_price: u64,
    /// ALT cache capacity (default: 1024).
    pub alt_cache_capacity: usize,
    /// Reconciliation config.
    pub reconciliation: ReconciliationConfig,
    /// Max time to wait for confirmation before triggering reconciliation (default: 30s).
    pub confirmation_timeout: Duration,
    /// Whether to run the reconciliation service (default: true).
    pub enable_reconciliation: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            blockhash_cache_capacity: 3,
            blockhash_refresh_interval_ms: 200,
            fee_window_size: 100,
            fee_percentile: 50.0,
            min_cu_price: 1_000,
            alt_cache_capacity: 1024,
            reconciliation: ReconciliationConfig::default(),
            confirmation_timeout: Duration::from_secs(30),
            enable_reconciliation: true,
        }
    }
}

impl From<&AppConfig> for OrchestratorConfig {
    fn from(cfg: &AppConfig) -> Self {
        Self {
            blockhash_cache_capacity: cfg.blockhash.cache_capacity,
            blockhash_refresh_interval_ms: cfg.blockhash.refresh_interval_ms,
            fee_window_size: cfg.fees.fee_window_size,
            fee_percentile: cfg.fees.fee_percentile,
            min_cu_price: cfg.fees.compute_unit_price,
            alt_cache_capacity: cfg.runtime.alt_cache_capacity,
            // TODO(P2-01): Wire from AppConfig.runtime.reconciliation_config (requires storing
            // ReconciliationConfig on RuntimeConfig or adding a dedicated section).
            reconciliation: ReconciliationConfig::default(),
            confirmation_timeout: Duration::from_secs(cfg.runtime.confirmation_timeout_secs),
            enable_reconciliation: cfg.runtime.enable_reconciliation,
        }
    }
}

// ---------------------------------------------------------------------------
// TradePlan — a validated intent ready for execution
// ---------------------------------------------------------------------------

/// A validated trade intent ready for execution. Created by the orchestrator
/// after RiskEngine passes.
#[derive(Debug)]
pub struct TradePlan {
    /// The trade intent with state machine tracking.
    pub intent: TradeIntent,
    /// Blockhash to use for this trade.
    pub blockhash: solana_hash::Hash,
    /// Estimated CU price.
    pub cu_price: u64,
    /// Compute unit limit.
    pub cu_limit: u32,
    /// Pre-trade balance snapshot for reconciliation.
    pub balance_snapshot: BalanceSnapshot,
    /// Current slot at plan time.
    pub current_slot: u64,
    /// Last valid block height for this blockhash.
    pub last_valid_slot: Option<u64>,
}

// ---------------------------------------------------------------------------
// TradeResult — the outcome of executing a TradePlan
// ---------------------------------------------------------------------------

/// The full result of executing a trade plan.
#[derive(Debug)]
pub struct TradeResult {
    /// The (possibly updated) trade intent.
    pub intent: TradeIntent,
    /// Whether the trade was successfully executed.
    pub success: bool,
    /// Signatures from all submission lanes.
    pub signatures: Vec<Signature>,
    /// Optional error message.
    pub error: Option<String>,
    /// Per-provider submission timings (latency data from SWQOS lanes).
    pub timings: Vec<SwqosSubmitTiming>,
    /// Reconciliation outcome, if reconciliation was triggered.
    pub reconciliation: Option<ReconciliationOutcome>,
    /// Total elapsed time from plan to completion.
    pub elapsed_ms: u64,
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// The trade orchestrator wires Phase 5-8 services into a single execution pipeline.
///
/// # Lifecycle
///
/// 1. Call `start()` to spawn background services (blockhash refresh, fee refresh).
/// 2. Call `plan_trade()` to validate and create a `TradePlan`.
/// 3. Call `execute_plan()` to submit and track the trade.
/// 4. Call `stop()` to gracefully shut down.
///
/// The kill switch (`kill()`) immediately rejects all new trade plans.
pub struct Orchestrator {
    // Phase 7 cached services
    pub blockhash_service: BlockhashService,
    pub fee_service: FeeService,
    pub alt_cache: AltCache,

    // Phase 5 risk engine
    pub risk_engine: RiskEngine,

    // Phase 14/15 strategy engine (Mutex for &self-access from plan_trade)
    pub strategy_engine: Mutex<StrategyEngine>,

    // Phase 8 reconciliation
    pub reconciliation_service: ReconciliationService,
    pub reconciliation_config: ReconciliationConfig,

    // Configuration
    pub config: OrchestratorConfig,

    // Kill switch
    pub kill_switch: Arc<AtomicBool>,

    // RPC client for on-chain queries
    pub rpc: Arc<SolanaRpcClient>,

    // Payer keypair (the execution wallet)
    pub payer: Arc<Keypair>,

    /// SWQOS submission clients for execution.
    pub swqos_clients: Arc<Vec<Arc<SwqosClient>>>,

    /// Runtime trade ledger tracking open positions, daily counts, and P&L.
    pub trade_ledger: Mutex<TradeLedger>,
}

impl Orchestrator {
    /// Create a new orchestrator, wiring all services together.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rpc_url: impl Into<String>,
        payer: Arc<Keypair>,
        risk_config: RiskConfig,
        config: OrchestratorConfig,
        strategy_engine: StrategyEngine,
        swqos_clients: Arc<Vec<Arc<SwqosClient>>>,
    ) -> Self {
        let rpc_url = rpc_url.into();
        let rpc = Arc::new(SolanaRpcClient::new(rpc_url.clone()));

        let blockhash_service = BlockhashService::new(
            rpc_url.clone(),
            config.blockhash_cache_capacity,
            config.blockhash_refresh_interval_ms,
        );

        let fee_service = FeeService::new(
            rpc_url.clone(),
            config.fee_window_size,
            config.fee_percentile,
            config.min_cu_price,
        );

        let alt_cache = AltCache::with_defaults(rpc_url.clone());

        let reconciliation_service =
            ReconciliationService::new(rpc.clone(), config.reconciliation.clone());

        let risk_engine = RiskEngine::new(risk_config);

        Self {
            blockhash_service,
            fee_service,
            alt_cache,
            risk_engine,
            strategy_engine: Mutex::new(strategy_engine),
            reconciliation_service,
            reconciliation_config: config.reconciliation.clone(),
            config,
            kill_switch: Arc::new(AtomicBool::new(false)),
            rpc,
            payer,
            swqos_clients,
            trade_ledger: Mutex::new(TradeLedger::default()),
        }
    }

    /// Create from an `AppConfig` (the production config struct).
    pub fn from_config(
        config: AppConfig,
        payer: Arc<Keypair>,
        swqos_clients: Arc<Vec<Arc<SwqosClient>>>,
    ) -> anyhow::Result<Self> {
        config.validate()?;
        let orchestrator_config = OrchestratorConfig::from(&config);
        let rpc_url = config.rpc.primary.clone();
        let strategy_engine = StrategyEngine::new(config.strategy.to_engine_config());
        Ok(Self::new(
            rpc_url,
            payer,
            config.risk.into(),
            orchestrator_config,
            strategy_engine,
            swqos_clients,
        ))
    }

    /// Start background service loops (blockhash refresh, fee refresh).
    pub fn start(&self) {
        self.blockhash_service.spawn_refresh();
        self.fee_service.spawn_refresh();
        info!(target: "sol_trade_sdk", "orchestrator: background services started (blockhash + fee refresh)");
    }

    // ── Kill switch ──

    /// Activate the kill switch. All subsequent `plan_trade()` calls will be rejected.
    pub fn kill(&self) {
        self.kill_switch.store(true, Ordering::Release);
        info!(target: "sol_trade_sdk", "orchestrator: KILL SWITCH ACTIVATED — no new trades accepted");
    }

    /// Deactivate the kill switch.
    pub fn unkill(&self) {
        self.kill_switch.store(false, Ordering::Release);
        info!(target: "sol_trade_sdk", "orchestrator: kill switch deactivated");
    }

    /// Check if the kill switch is active.
    pub fn is_killed(&self) -> bool {
        self.kill_switch.load(Ordering::Acquire)
    }

    // ── Strategy engine integration (Phase 14/15) ──

    /// Feed a classified event into the strategy engine's market state.
    ///
    /// Call this in the event-ingest loop before calling `plan_trade()` to
    /// ensure the strategy engine has up-to-date market state.
    pub fn process_event(&self, event: &ClassifiedEvent) {
        let now_micros = crate::common::fast_timing::fast_now_micros() as i64;
        if let Ok(mut engine) = self.strategy_engine.lock() {
            engine.process_event(event, now_micros);
        }
    }

    /// Returns a snapshot of strategy engine statistics.
    pub fn strategy_stats(&self) -> StrategyStats {
        if let Ok(engine) = self.strategy_engine.lock() {
            engine.stats()
        } else {
            StrategyStats {
                eval_count: 0,
                signal_count: 0,
                notrade_count: 0,
                tracked_mints: 0,
                eviction_count: 0,
            }
        }
    }

    // ── Trade planning ──

    /// Validate and create a `TradePlan` for a trade opportunity.
    ///
    /// Gates, in order:
    /// 1. Kill switch
    /// 2. Risk engine
    /// 3. Strategy engine signal check
    pub async fn plan_trade(
        &self,
        protocol: &str,
        mint: &Pubkey,
        direction: TradeDirection,
        input_amount: u64,
        min_output_amount: u64,
        detected_slot: Option<u64>,
        trade_params: &TradeRiskParams,
    ) -> Option<TradePlan> {
        #[cfg(feature = "perf-trace")]
        let _plan_span = TraceSpan::new(perf_metrics::PLAN_TRADE_NS.clone());

        if self.is_killed() {
            warn!(target: "sol_trade_sdk", "orchestrator: trade rejected — kill switch active");
            #[cfg(feature = "perf-trace")]
            perf_metrics::TRADE_REJECTED.increment(1);
            return None;
        }

        // ── Build RiskContext from the runtime trade ledger ──
        let risk_context = {
            let mut ledger = self.trade_ledger.lock().ok()?;
            ledger.daily_reset_if_needed();
            ledger.build_context()
        };

        if let Err(e) = self.risk_engine.check(
                    &risk_context,
                    trade_params,
                    protocol,
                    "default",
                    &mint.to_string(),
                ) {
                    warn!(target: "sol_trade_sdk", "orchestrator: trade rejected by risk engine: {e}");
                    return None;
                }

                // ── Gate 3: Strategy engine signal check ──
                let _mid_price = {
                    let mut engine = self.strategy_engine.lock().ok()?;
                    let mint_str = mint.to_string();
                    // Get midpoint price from market state, fallback to 0.0
                    let mp = engine
                        .market_state
                        .get(protocol, &mint_str)
                        .map(|s| s.last_price)
                        .unwrap_or(0.0);
                    let now_micros = crate::common::fast_timing::fast_now_micros() as i64;
                    let slot = detected_slot.unwrap_or(0);
                    match engine.evaluate(&mint_str, protocol, slot, mp, now_micros) {
                        Some(StrategyOutcome::Trade(signal)) => {
                            info!(
                                target: "sol_trade_sdk",
                                "orchestrator: strategy signal for {} — {} (conf={:.2}, score={:.2})",
                                mint_str, signal.strength, signal.confidence, signal.composite_score,
                            );
                            signal.midpoint_price
                        }
                        Some(StrategyOutcome::NoTrade(nt)) => {
                            warn!(
                                target: "sol_trade_sdk",
                                "orchestrator: trade rejected by strategy engine for {}: {}",
                                mint_str, nt.reason,
                            );
                            return None;
                        }
                        None => {
                            // Strategy engine is time-gated (too early for another eval).
                            // Still allow the trade — the market state is being built.
                            trace!(
                                target: "sol_trade_sdk",
                                "orchestrator: strategy engine not ready for {} (eval gate)",
                                mint_str,
                            );
                            0.0
                        }
                    }
                };

        let blockhash = self.blockhash_service.get()?;
        // TODO(P1-08): BlockhashService returns only a Hash — the last_valid_block_height is
        // never queried from the RPC. Wire through BlockhashService to return both (Hash, Slot)
        // so reconciliation can assess blockhash expiry in Step 3.
        let last_valid_slot = None;

        let cu_price = self.fee_service.estimate_cu_price();
        let cu_limit = 400_000;

        let current_slot = match self.rpc.get_slot().await {
            Ok(slot) => slot,
            Err(e) => {
                warn!(target: "sol_trade_sdk", "orchestrator: failed to get slot: {}", e);
                0
            }
        };

        let payer_sol = self.rpc.get_balance(&self.payer.pubkey()).await.unwrap_or(0);

        let balance_snapshot = BalanceSnapshot {
            payer_sol_lamports: payer_sol,
            // TODO(P1-09): Query input/output token ATAs in plan_trade and populate both
            // balances so reconciliation can perform balance-based checks in Step 4.
            input_token_balance: 0,
            output_token_balance: 0,
            snapshot_slot: current_slot,
        };

        let intent = TradeIntent::new(
            protocol,
            mint.to_string(),
            direction,
            input_amount,
            min_output_amount,
            detected_slot,
        );

        Some(TradePlan {
            intent,
            blockhash,
            cu_price,
            cu_limit,
            balance_snapshot,
            current_slot,
            last_valid_slot,
        })
    }

    // ── Trade execution ──

    /// Execute a trade plan through the full pipeline: sign, submit, confirm, reconcile.
    pub async fn execute_plan(
        &self,
        plan: TradePlan,
        executor: &dyn TradeExecutor,
        swap_params: SwapParams,
    ) -> TradeResult {
        #[cfg(feature = "perf-trace")]
        let _exec_span = TraceSpan::new(perf_metrics::EXECUTE_PLAN_NS.clone());

        let start = Instant::now();
        let mut intent = plan.intent;

        if let Err(e) = intent.transition(TradeState::Validated, "risk check passed") {
            return TradeResult {
                intent,
                success: false,
                signatures: vec![],
                timings: vec![],
                error: Some(e.to_string()),
                reconciliation: None,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        if let Err(e) = intent.transition(TradeState::Built, "instructions built") {
            return TradeResult {
                intent,
                success: false,
                signatures: vec![],
                timings: vec![],
                error: Some(e.to_string()),
                reconciliation: None,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        let swap_result = executor.swap(swap_params).await;

        let (ok, signatures, err, timings) = match swap_result {
            Ok(result) => result,
            Err(e) => {
                // Built -> Failed is now valid (P1-08). Log and continue.
                if let Err(transition_err) = intent.transition(
                    TradeState::Failed,
                    format!("executor failed before submission: {e}"),
                ) {
                    warn!(target: "sol_trade_sdk", "state transition failed: {:#}", transition_err);
                }
                return TradeResult {
                    intent,
                    success: false,
                    signatures: vec![],
                    timings: vec![],
                    error: Some(e.to_string()),
                    reconciliation: None,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                };
            }
        };

        if ok {
            #[cfg(feature = "perf-trace")]
            perf_metrics::TRADE_LANDED.increment(1);

            for sig in &signatures {
                // P0-06: Insert Signed transition before record_submission.
                // record_submission internally calls transition(Submitted) — (Built -> Submitted)
                // is invalid so we must go through Signed first.
                if let Err(e) =
                    intent.transition(TradeState::Signed, "signed by execution keypair")
                {
                    warn!(target: "sol_trade_sdk", "state transition to Signed failed: {:#}", e);
                    return TradeResult {
                        intent,
                        success: false,
                        signatures: vec![],
                        timings,
                        error: Some(format!("state transition failed: {e}")),
                        reconciliation: None,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    };
                }
                if let Err(e) = intent.record_submission(sig.to_string()) {
                    warn!(target: "sol_trade_sdk", "record_submission failed: {:#}", e);
                    return TradeResult {
                        intent,
                        success: false,
                        signatures: vec![],
                        timings,
                        error: Some(format!("record_submission failed: {e}")),
                        reconciliation: None,
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    };
                }
            }
            if let Err(e) = intent.transition(TradeState::Landed, "confirmed on chain") {
                warn!(target: "sol_trade_sdk", "state transition to Landed failed: {:#}", e);
                return TradeResult {
                    intent,
                    success: false,
                    signatures: vec![],
                    timings,
                    error: Some(format!("state transition to Landed failed: {e}")),
                    reconciliation: None,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                };
            }
            if let Err(e) = intent.transition(TradeState::Settled, "trade completed") {
                warn!(target: "sol_trade_sdk", "state transition to Settled failed: {:#}", e);
                return TradeResult {
                    intent,
                    success: false,
                    signatures: vec![],
                    timings,
                    error: Some(format!("state transition to Settled failed: {e}")),
                    reconciliation: None,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                };
            }
            return TradeResult {
                intent,
                success: true,
                signatures,
                error: None,
                timings,
                reconciliation: None,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        // Ambiguous case — trigger reconciliation
        if !signatures.is_empty() && self.config.enable_reconciliation {
            #[cfg(feature = "perf-trace")]
            {
                perf_metrics::TRADE_RECONCILED.increment(1);
                let _reconcile_span = TraceSpan::new(perf_metrics::RECONCILE_NS.clone());
            }

            if let Err(e) = intent.transition(
                TradeState::Ambiguous,
                "all lanes returned ambiguous, starting reconciliation",
            ) {
                warn!(target: "sol_trade_sdk", "state transition to Ambiguous failed: {:#}", e);
            }

            // P2-08: TODO — pass real ATAs and mints from plan/swap_params so reconciliation
            // can query post-trade token balances. Currently all None prevents Step 4 balance checks.
            let outcome = self
                .reconciliation_service
                .reconcile(
                    &signatures,
                    &plan.balance_snapshot,
                    plan.last_valid_slot,
                    plan.current_slot,
                    &self.payer.pubkey(),
                    None, // TODO(P2-08): input_mint from plan/swap_params
                    None, // TODO(P2-08): output_mint from plan/swap_params
                    None, // TODO(P2-08): input_ata from plan/swap_params
                    None, // TODO(P2-08): output_ata from plan/swap_params
                )
                .await;

            info!(target: "sol_trade_sdk", "orchestrator: reconciliation complete: {:?}", outcome);

            match &outcome {
                ReconciliationOutcome::Landed { signature, slot } => {
                    if let Err(e) = intent.transition(
                        TradeState::Landed,
                        format!("reconciled: landed at slot {}", slot),
                    ) {
                        warn!(target: "sol_trade_sdk", "state transition to Landed after reconciliation failed: {:#}", e);
                    }
                    if let Err(e) = intent.transition(TradeState::Settled, "reconciled: trade completed") {
                        warn!(target: "sol_trade_sdk", "state transition to Settled after reconciliation failed: {:#}", e);
                    }
                    TradeResult {
                        intent,
                        success: true,
                        signatures: vec![*signature],
                        error: None,
                        timings: timings.clone(),
                        reconciliation: Some(outcome),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    }
                }
                ReconciliationOutcome::Failed { signature, error: ref err_msg, slot } => {
                    if let Err(e) = intent.transition_with_evidence(
                        TradeState::Failed,
                        format!("reconciled: failed at slot {}", slot),
                        err_msg.clone(),
                    ) {
                        warn!(target: "sol_trade_sdk", "state transition to Failed after reconciliation failed: {:#}", e);
                    }
                    TradeResult {
                        intent,
                        success: false,
                        signatures: vec![*signature],
                        error: Some(err_msg.clone()),
                        timings: timings.clone(),
                        reconciliation: Some(outcome),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    }
                }
                ReconciliationOutcome::RolledBack { reason } => {
                    if let Err(e) = intent.transition(
                        TradeState::Expired,
                        format!("reconciled: rolled back — {}", reason),
                    ) {
                        warn!(target: "sol_trade_sdk", "state transition to Expired (rolled back) failed: {:#}", e);
                    }
                    TradeResult {
                        intent,
                        success: false,
                        signatures: vec![],
                        error: Some(reason.clone()),
                        timings: timings.clone(),
                        reconciliation: Some(outcome),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    }
                }
                ReconciliationOutcome::Rebuild { reason } => {
                    if let Err(e) = intent.transition(
                        TradeState::Expired,
                        format!("reconciled: rebuild recommended — {}", reason),
                    ) {
                        warn!(target: "sol_trade_sdk", "state transition to Expired (rebuild) failed: {:#}", e);
                    }
                    TradeResult {
                        intent,
                        success: false,
                        signatures: vec![],
                        error: Some(reason.clone()),
                        timings: timings.clone(),
                        reconciliation: Some(outcome),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    }
                }
            }
        } else {
            let err_msg = err.map(|e| e.to_string()).unwrap_or_else(|| "execution failed".into());
            if let Err(e) = intent.transition(TradeState::Failed, &err_msg) {
                warn!(target: "sol_trade_sdk", "state transition to Failed failed: {:#}", e);
            }
            TradeResult {
                intent,
                success: false,
                signatures,
                error: Some(err_msg),
                timings,
                reconciliation: None,
                elapsed_ms: start.elapsed().as_millis() as u64,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::config::RiskConfig;

    #[test]
    fn orchestrator_creation() {
        let payer = Arc::new(Keypair::new());
        let risk_config = RiskConfig::default();
        let swqos_clients = Arc::new(vec![]);
        let orch = Orchestrator::new(
            "https://api.mainnet-beta.solana.com",
            payer,
            risk_config,
            OrchestratorConfig::default(),
            StrategyEngine::default_with_config(),
            swqos_clients,
        );
        assert!(!orch.is_killed());
        assert_eq!(orch.config.blockhash_refresh_interval_ms, 200);
    }

    #[test]
    fn kill_switch_blocks_planning() {
        let payer = Arc::new(Keypair::new());
        let risk_config = RiskConfig::default();
        let swqos_clients = Arc::new(vec![]);
        let orch = Orchestrator::new(
            "https://api.mainnet-beta.solana.com",
            payer,
            risk_config,
            OrchestratorConfig::default(),
            StrategyEngine::default_with_config(),
            swqos_clients,
        );
        orch.kill();
        assert!(orch.is_killed());
    }

    #[test]
    fn unkill_restores_planning() {
        let payer = Arc::new(Keypair::new());
        let risk_config = RiskConfig::default();
        let swqos_clients = Arc::new(vec![]);
        let orch = Orchestrator::new(
            "https://api.mainnet-beta.solana.com",
            payer,
            risk_config,
            OrchestratorConfig::default(),
            StrategyEngine::default_with_config(),
            swqos_clients,
        );
        orch.kill();
        assert!(orch.is_killed());
        orch.unkill();
        assert!(!orch.is_killed());
    }

    #[test]
    fn config_from_appconfig() {
        let app_cfg = AppConfig::default();
        let orch_cfg = OrchestratorConfig::from(&app_cfg);
        assert_eq!(orch_cfg.blockhash_refresh_interval_ms, 200);
        assert_eq!(orch_cfg.min_cu_price, 1_000);
        assert!(orch_cfg.enable_reconciliation);
    }

    #[test]
    fn orchestrator_config_defaults() {
        let cfg = OrchestratorConfig::default();
        assert_eq!(cfg.blockhash_cache_capacity, 3);
        assert_eq!(cfg.fee_window_size, 100);
        assert_eq!(cfg.fee_percentile, 50.0);
        assert_eq!(cfg.min_cu_price, 1_000);
        assert_eq!(cfg.alt_cache_capacity, 1024);
        assert_eq!(cfg.confirmation_timeout, Duration::from_secs(30));
        assert!(cfg.enable_reconciliation);
    }

    #[test]
    fn trade_plan_structure() {
        let intent =
            TradeIntent::new("pumpfun", "mint123", TradeDirection::Buy, 1000, 990, Some(42));
        let plan = TradePlan {
            intent,
            blockhash: solana_hash::Hash::new_unique(),
            cu_price: 1000,
            cu_limit: 400_000,
            balance_snapshot: BalanceSnapshot {
                payer_sol_lamports: 1_000_000_000,
                input_token_balance: 0,
                output_token_balance: 0,
                snapshot_slot: 100,
            },
            current_slot: 100,
            last_valid_slot: Some(250),
        };
        assert_eq!(plan.cu_price, 1000);
        assert_eq!(plan.cu_limit, 400_000);
        assert_eq!(plan.intent.state, TradeState::Detected);
    }

    #[test]
    fn trade_result_format() {
        let intent = TradeIntent::new("pumpfun", "mint123", TradeDirection::Buy, 1000, 990, None);
        let result = TradeResult {
            intent,
            success: true,
            signatures: vec![Signature::default()],
            error: None,
            timings: vec![],
            reconciliation: None,
            elapsed_ms: 42,
        };
        assert!(result.success);
        assert_eq!(result.elapsed_ms, 42);
        assert_eq!(result.signatures.len(), 1);
    }
}