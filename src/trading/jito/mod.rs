//! Jito Bundle Execution Loop — strategy-aware tip calculation, bundle metrics.
//!
//! This module provides:
//!
//! 1. **TipCalculator** — Maps `composite_score` + `confidence` → optimal Jito tip (SOL).
//! 2. **TipConfig** — Configuration for tip calculation strategy.
//! 3. **BundleMetrics** — Execution metrics (bundle success rate, tip cost, landing latency).
//!
//! The trade execution loop itself lives in [`bundle_executor`].
//!
//! # Architecture
//!
//! ```text
//! TradeSignal { composite_score, confidence, strength }
//!     │
//!     ▼
//! TipCalculator::compute(score, confidence, config)
//!     │
//!     ▼
//! tip_sol: f64
//!     │
//!     ▼
//! New GasFeeStrategy with Jito tip = tip_sol
//!     │
//!     ▼
//! executor.swap(SwapParams { with_tip: true, gas_fee_strategy, ... })
//! ```

pub mod bundle_executor;

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// TipStrategy
// ---------------------------------------------------------------------------

/// Strategy for computing Jito bundle tips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TipStrategy {
    /// Fixed tip — always pay `fixed_tip_sol` regardless of signal strength.
    Fixed,
    /// Strategy-aware — tip scales with `|composite_score| * confidence`.
    StrategyAware,
}

impl Default for TipStrategy {
    fn default() -> Self {
        Self::Fixed
    }
}

impl fmt::Display for TipStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed => write!(f, "fixed"),
            Self::StrategyAware => write!(f, "strategy-aware"),
        }
    }
}

// ---------------------------------------------------------------------------
// TipConfig
// ---------------------------------------------------------------------------

/// Configuration for Jito bundle tip calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipConfig {
    /// Tip strategy: "fixed" or "strategy-aware".
    pub strategy: TipStrategy,
    /// Fixed tip in SOL (used when strategy == Fixed).
    pub fixed_tip_sol: f64,
    /// Minimum tip in SOL (used when strategy == StrategyAware).
    pub min_tip_sol: f64,
    /// Maximum tip in SOL (used when strategy == StrategyAware).
    pub max_tip_sol: f64,
    /// Global multiplier applied after calculation.
    pub multiplier: f64,
    /// Maximum bundle retry attempts.
    pub max_retries: u32,
    /// Tip escalation factor per retry (e.g. 2.0 = double tip each retry).
    pub tip_escalation_factor: f64,
    /// Enable simulation gating: skip if simulation fails.
    pub enable_simulation_gate: bool,
    /// Minimum profit-to-tip ratio for strategy-aware mode.
    /// Skip if expected_profit / tip < this value.
    pub min_profit_to_tip_ratio: f64,
}

impl Default for TipConfig {
    fn default() -> Self {
        Self {
            strategy: TipStrategy::StrategyAware,
            fixed_tip_sol: 0.001,              // 0.001 SOL
            min_tip_sol: 0.000_1,              // 0.0001 SOL (~100K lamports)
            max_tip_sol: 0.01,                 // 0.01 SOL (~10M lamports)
            multiplier: 1.0,
            max_retries: 3,
            tip_escalation_factor: 2.0,
            enable_simulation_gate: true,
            min_profit_to_tip_ratio: 2.0,
        }
    }
}

impl TipConfig {
    /// Create a TipConfig from the app-level Jito submission config.
    pub fn from_jito_config(jito: &crate::common::config::JitoSubmissionConfig) -> Self {
        let strategy = match jito.bundle_tip_strategy.as_str() {
            "strategy-aware" | "strategy_aware" | "strategyaware" => TipStrategy::StrategyAware,
            _ => TipStrategy::Fixed,
        };
        Self {
            strategy,
            fixed_tip_sol: (jito.tip_min_lamports as f64) / 1e9,
            min_tip_sol: (jito.tip_min_lamports as f64) / 1e9,
            max_tip_sol: (jito.tip_max_lamports as f64) / 1e9,
            multiplier: 1.0,
            max_retries: 3,
            tip_escalation_factor: 2.0,
            enable_simulation_gate: true,
            min_profit_to_tip_ratio: 2.0,
        }
    }
}

// ---------------------------------------------------------------------------
// TipCalculator
// ---------------------------------------------------------------------------

/// Maps strategy signal strength to Jito tip amount.
///
/// # Tip calculation
///
/// **Strategy-aware mode:**
/// ```text
/// raw = |composite_score| * confidence          // [0, 1]
/// tip = min_tip + raw * (max_tip - min_tip)     // linear interpolation
/// tip *= multiplier
/// ```
///
/// **Fixed mode:**
/// ```text
/// tip = fixed_tip_sol * multiplier
/// ```
pub struct TipCalculator;

impl TipCalculator {
    /// Compute the Jito tip in SOL for a given signal.
    ///
    /// * `composite_score` — The strategy's composite score, typically in [-1, +1].
    /// * `confidence` — Signal confidence in [0.0, 1.0].
    /// * `config` — Tip configuration.
    pub fn compute(
        composite_score: f64,
        confidence: f64,
        config: &TipConfig,
    ) -> f64 {
        match config.strategy {
            TipStrategy::Fixed => {
                config.fixed_tip_sol * config.multiplier
            }
            TipStrategy::StrategyAware => {
                // Linear interpolation over [min, max]
                let raw = composite_score.abs() * confidence;
                let raw = raw.clamp(0.0, 1.0);
                let tip = config.min_tip_sol + raw * (config.max_tip_sol - config.min_tip_sol);
                tip * config.multiplier
            }
        }
    }

    /// Compute tip for a retry attempt. Escalates by `factor^attempt`.
    pub fn compute_retry_tip(
        base_tip: f64,
        attempt: u32,
        factor: f64,
    ) -> f64 {
        base_tip * factor.powi(attempt as i32)
    }
}

// ---------------------------------------------------------------------------
// BundleOutcome
// ---------------------------------------------------------------------------

/// Outcome of a single bundle execution attempt.
#[derive(Debug, Clone)]
pub enum BundleOutcome {
    /// Bundle was submitted and confirmed on chain.
    Landed {
        tip_sol: f64,
        landing_ms: u64,
        attempts: u32,
    },
    /// Bundle was rejected by Jito (nack).
    Nacked {
        tip_sol: f64,
        attempts: u32,
        reason: String,
    },
    /// Bundle simulation failed.
    SimulationFailed {
        reason: String,
    },
    /// All retries exhausted.
    Exhausted {
        tip_sol: f64,
        attempts: u32,
    },
    /// Trade plan rejected (kill switch, risk, or strategy gate).
    Rejected {
        reason: String,
    },
}

impl fmt::Display for BundleOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Landed { tip_sol, landing_ms, attempts } => {
                write!(f, "LANDED tip={:.6} SOL landed_in={}ms attempts={}", tip_sol, landing_ms, attempts)
            }
            Self::Nacked { tip_sol, attempts, reason } => {
                write!(f, "NACKED tip={:.6} SOL attempts={} reason={}", tip_sol, attempts, reason)
            }
            Self::SimulationFailed { reason } => {
                write!(f, "SIM_FAIL reason={}", reason)
            }
            Self::Exhausted { tip_sol, attempts } => {
                write!(f, "EXHAUSTED tip={:.6} SOL attempts={}", tip_sol, attempts)
            }
            Self::Rejected { reason } => {
                write!(f, "REJECTED reason={}", reason)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BundleMetrics (zero-cost when perf-trace feature is disabled)
// ---------------------------------------------------------------------------

#[cfg(feature = "perf-trace")]
mod perf_bundle_metrics {
    use crate::perf::{LatencyHistogram, PerfCounter, PerfRegistry};
    use once_cell::sync::Lazy;
    use std::sync::Arc;

    /// Total bundle submissions.
    pub static BUNDLE_SUBMITTED: PerfCounter = PerfCounter::new("jito.bundle_submitted");
    /// Successfully landed bundles.
    pub static BUNDLE_LANDED: PerfCounter = PerfCounter::new("jito.bundle_landed");
    /// Bundles nacked by Jito.
    pub static BUNDLE_NACK: PerfCounter = PerfCounter::new("jito.bundle_nack");
    /// Bundles that failed simulation.
    pub static BUNDLE_SIM_FAIL: PerfCounter = PerfCounter::new("jito.bundle_sim_fail");
    /// Tip amounts paid (SOL * 1e9 for integer histogram).
    pub static BUNDLE_TIP_LAMPORTS: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
        let h = Arc::new(LatencyHistogram::new());
        PerfRegistry::global().register_histogram("jito.bundle_tip_lamports", h.clone());
        h
    });
    /// Landing latency in ms.
    pub static BUNDLE_LANDING_MS: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
        let h = Arc::new(LatencyHistogram::new());
        PerfRegistry::global().register_histogram("jito.bundle_landing_ms", h.clone());
        h
    });
    /// Retry count per bundle.
    pub static BUNDLE_RETRY_COUNT: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
        let h = Arc::new(LatencyHistogram::new());
        PerfRegistry::global().register_histogram("jito.bundle_retry_count", h.clone());
        h
    });
}

/// Record bundle metrics (no-op when perf-trace feature is disabled).
pub struct BundleMetrics;

impl BundleMetrics {
    /// Record a bundle submission.
    #[inline]
    pub fn submitted() {
        #[cfg(feature = "perf-trace")]
        perf_bundle_metrics::BUNDLE_SUBMITTED.increment(1);
    }

    /// Record a successfully landed bundle.
    #[inline]
    pub fn landed(tip_sol: f64, landing_ms: u64, attempts: u32) {
        #[cfg(feature = "perf-trace")]
        {
            perf_bundle_metrics::BUNDLE_LANDED.increment(1);
            let tip_lamports = (tip_sol * 1e9) as u64;
            perf_bundle_metrics::BUNDLE_TIP_LAMPORTS.observe(tip_lamports as i64);
            perf_bundle_metrics::BUNDLE_LANDING_MS.observe(landing_ms as i64);
            perf_bundle_metrics::BUNDLE_RETRY_COUNT.observe(attempts as i64);
        }
    }

    /// Record a nack.
    #[inline]
    pub fn nacked() {
        #[cfg(feature = "perf-trace")]
        perf_bundle_metrics::BUNDLE_NACK.increment(1);
    }

    /// Record a simulation failure.
    #[inline]
    pub fn sim_failed() {
        #[cfg(feature = "perf-trace")]
        perf_bundle_metrics::BUNDLE_SIM_FAIL.increment(1);
    }
}