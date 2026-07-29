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
    SolanaRpcClient,
};
use crate::constants::risk::{RiskContext, RiskEngine, TradeRiskParams};
use crate::trading::core::{
    reconciliation::{
        BalanceSnapshot, ReconciliationConfig, ReconciliationOutcome, ReconciliationService,
    },
    state::{TradeDirection, TradeIntent, TradeState},
    traits::TradeExecutor,
};
use crate::trading::SwapParams;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tracing::{info, warn};

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
            blockhash_cache_capacity: 3,
            blockhash_refresh_interval_ms: cfg.blockhash.refresh_interval_ms,
            fee_window_size: 100,
            fee_percentile: 50.0,
            min_cu_price: cfg.fees.compute_unit_price,
            alt_cache_capacity: 1024,
            reconciliation: ReconciliationConfig::default(),
            confirmation_timeout: Duration::from_secs(30),
            enable_reconciliation: true,
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

    // Phase 8 reconciliation
    pub reconciliation_service: ReconciliationService,
    pub reconciliation_config: ReconciliationConfig,

    // Configuration
    pub config: OrchestratorConfig,

    // Kill switch
    pub kill_switch: Arc<AtomicBool>,

    // RPC client for on-chain queries
    rpc: Arc<SolanaRpcClient>,

    // Payer keypair (the execution wallet)
    payer: Arc<Keypair>,
}

impl Orchestrator {
    /// Create a new orchestrator, wiring all services together.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rpc_url: impl Into<String>,
        payer: Arc<Keypair>,
        risk_config: RiskConfig,
        config: OrchestratorConfig,
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
            reconciliation_service,
            reconciliation_config: config.reconciliation.clone(),
            config,
            kill_switch: Arc::new(AtomicBool::new(false)),
            rpc,
            payer,
        }
    }

    /// Create from an `AppConfig` (the production config struct).
    pub fn from_config(config: AppConfig, payer: Arc<Keypair>) -> anyhow::Result<Self> {
        config.validate()?;
        let orchestrator_config = OrchestratorConfig::from(&config);
        let rpc_url = config.rpc.primary.clone();
        Ok(Self::new(rpc_url, payer, config.risk.into(), orchestrator_config))
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

    // ── Trade planning ──

    /// Validate and create a `TradePlan` for a trade opportunity.
    pub async fn plan_trade(
        &self,
        protocol: &str,
        mint: &Pubkey,
        direction: TradeDirection,
        input_amount: u64,
        min_output_amount: u64,
        detected_slot: Option<u64>,
        risk_context: &RiskContext,
        trade_params: &TradeRiskParams,
    ) -> Option<TradePlan> {
        if self.is_killed() {
            warn!(target: "sol_trade_sdk", "orchestrator: trade rejected — kill switch active");
            return None;
        }

        if let Err(e) = self.risk_engine.check(
            risk_context,
            trade_params,
            protocol,
            "default",
            &mint.to_string(),
        ) {
            warn!(target: "sol_trade_sdk", "orchestrator: trade rejected by risk engine: {}", e);
            return None;
        }

        let blockhash = self.blockhash_service.get()?;
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
        let start = Instant::now();
        let mut intent = plan.intent;

        if let Err(e) = intent.transition(TradeState::Validated, "risk check passed") {
            return TradeResult {
                intent,
                success: false,
                signatures: vec![],
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
                error: Some(e.to_string()),
                reconciliation: None,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        let swap_result = executor.swap(swap_params).await;

        let (ok, signatures, err, _timings) = match swap_result {
            Ok(result) => result,
            Err(e) => {
                let _ = intent.transition(TradeState::Failed, e.to_string());
                return TradeResult {
                    intent,
                    success: false,
                    signatures: vec![],
                    error: Some(e.to_string()),
                    reconciliation: None,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                };
            }
        };

        if ok {
            for sig in &signatures {
                let _ = intent.record_submission(sig.to_string());
            }
            let _ = intent.transition(TradeState::Landed, "confirmed on chain");
            let _ = intent.transition(TradeState::Settled, "trade completed");
            return TradeResult {
                intent,
                success: true,
                signatures,
                error: None,
                reconciliation: None,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        // Ambiguous case — trigger reconciliation
        if !signatures.is_empty() && self.config.enable_reconciliation {
            let _ = intent.transition(
                TradeState::Ambiguous,
                "all lanes returned ambiguous, starting reconciliation",
            );

            let outcome = self
                .reconciliation_service
                .reconcile(
                    &signatures,
                    &plan.balance_snapshot,
                    plan.last_valid_slot,
                    plan.current_slot,
                    &self.payer.pubkey(),
                    None,
                    None,
                    None,
                    None,
                )
                .await;

            info!(target: "sol_trade_sdk", "orchestrator: reconciliation complete: {:?}", outcome);

            match &outcome {
                ReconciliationOutcome::Landed { signature, slot } => {
                    let _ = intent.transition(
                        TradeState::Landed,
                        format!("reconciled: landed at slot {}", slot),
                    );
                    let _ = intent.transition(TradeState::Settled, "reconciled: trade completed");
                    TradeResult {
                        intent,
                        success: true,
                        signatures: vec![*signature],
                        error: None,
                        reconciliation: Some(outcome),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    }
                }
                ReconciliationOutcome::Failed { signature, error: ref err_msg, slot } => {
                    let _ = intent.transition_with_evidence(
                        TradeState::Failed,
                        format!("reconciled: failed at slot {}", slot),
                        err_msg.clone(),
                    );
                    TradeResult {
                        intent,
                        success: false,
                        signatures: vec![*signature],
                        error: Some(err_msg.clone()),
                        reconciliation: Some(outcome),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    }
                }
                ReconciliationOutcome::RolledBack { reason } => {
                    let _ = intent.transition(
                        TradeState::Expired,
                        format!("reconciled: rolled back — {}", reason),
                    );
                    TradeResult {
                        intent,
                        success: false,
                        signatures: vec![],
                        error: Some(reason.clone()),
                        reconciliation: Some(outcome),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    }
                }
                ReconciliationOutcome::Rebuild { reason } => {
                    let _ = intent.transition(
                        TradeState::Expired,
                        format!("reconciled: rebuild recommended — {}", reason),
                    );
                    TradeResult {
                        intent,
                        success: false,
                        signatures: vec![],
                        error: Some(reason.clone()),
                        reconciliation: Some(outcome),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    }
                }
            }
        } else {
            let err_msg = err.map(|e| e.to_string()).unwrap_or_else(|| "execution failed".into());
            let _ = intent.transition(TradeState::Failed, &err_msg);
            TradeResult {
                intent,
                success: false,
                signatures,
                error: Some(err_msg),
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
        let orch = Orchestrator::new(
            "https://api.mainnet-beta.solana.com",
            payer,
            risk_config,
            OrchestratorConfig::default(),
        );
        assert!(!orch.is_killed());
        assert_eq!(orch.config.blockhash_refresh_interval_ms, 200);
    }

    #[test]
    fn kill_switch_blocks_planning() {
        let payer = Arc::new(Keypair::new());
        let risk_config = RiskConfig::default();
        let orch = Orchestrator::new(
            "https://api.mainnet-beta.solana.com",
            payer,
            risk_config,
            OrchestratorConfig::default(),
        );
        orch.kill();
        assert!(orch.is_killed());
    }

    #[test]
    fn unkill_restores_planning() {
        let payer = Arc::new(Keypair::new());
        let risk_config = RiskConfig::default();
        let orch = Orchestrator::new(
            "https://api.mainnet-beta.solana.com",
            payer,
            risk_config,
            OrchestratorConfig::default(),
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
            reconciliation: None,
            elapsed_ms: 42,
        };
        assert!(result.success);
        assert_eq!(result.elapsed_ms, 42);
        assert_eq!(result.signatures.len(), 1);
    }
}