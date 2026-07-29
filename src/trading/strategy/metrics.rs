//! StrategyMetrics — dedicated observability for the strategy evaluation pipeline.
//!
//! All counters and histograms are gated behind the `perf-trace` Cargo feature
//! (zero-cost when disabled). Registers everything with the global `PerfRegistry`
//! so that `GET /metrics` serves them alongside orchestator and executor metrics.
//!
//! # Usage
//!
//! ```ignore
//! use crate::trading::strategy::metrics;
//! metrics::EVAL_CALLED.increment(1);
//! ```

use crate::perf::{LatencyHistogram, PerfCounter, PerfRegistry};
use once_cell::sync::Lazy;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Counters
// ---------------------------------------------------------------------------

/// Total number of times `StrategyEngine::evaluate()` was called.
pub static EVAL_CALLED: PerfCounter = PerfCounter::new("strategy.eval_called");

/// Number of times `evaluate()` returned a `TradeSignal` (passed all gates).
pub static SIGNAL_GENERATED: PerfCounter = PerfCounter::new("strategy.signal_generated");

/// Number of times `evaluate()` returned `NoTrade` (rejected by a gate).
pub static SIGNAL_REJECTED: PerfCounter = PerfCounter::new("strategy.signal_rejected");

/// Number of times `evaluate()` returned `None` (time gate — too early).
pub static EVAL_SKIPPED: PerfCounter = PerfCounter::new("strategy.eval_skipped");

/// Counter for each rejection gate — incremented at the specific gate that failed.
pub static GATE_COMPOSITE: PerfCounter = PerfCounter::new("strategy.gate_composite");
pub static GATE_SPREAD: PerfCounter = PerfCounter::new("strategy.gate_spread");
pub static GATE_STALE: PerfCounter = PerfCounter::new("strategy.gate_stale");
pub static GATE_EVENT_RATE: PerfCounter = PerfCounter::new("strategy.gate_event_rate");
pub static GATE_CONFIDENCE: PerfCounter = PerfCounter::new("strategy.gate_confidence");

/// Number of classified events fed into the engine via `process_event()`.
pub static EVENTS_INGESTED: PerfCounter = PerfCounter::new("strategy.events_ingested");

/// Number of mints evicted from the market state due to staleness / cap.
pub static MINTS_EVICTED: PerfCounter = PerfCounter::new("strategy.mints_evicted");

// ---------------------------------------------------------------------------
// Histograms
// ---------------------------------------------------------------------------

/// Latency histogram for `StrategyEngine::evaluate()` (nanoseconds).
pub static EVAL_NS: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
    let h = Arc::new(LatencyHistogram::new());
    PerfRegistry::global().register_histogram("strategy.eval_ns", h.clone());
    h
});

/// Latency histogram for `process_event()` — updating market state (nanoseconds).
pub static PROCESS_EVENT_NS: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
    let h = Arc::new(LatencyHistogram::new());
    PerfRegistry::global().register_histogram("strategy.process_event_ns", h.clone());
    h
});

/// Signal strength distribution — recorded as 0 (Hold/Neutral), 1 (Buy/Sell), 2 (StrongBuy/StrongSell).
pub static SIGNAL_STRENGTH: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
    let h = Arc::new(LatencyHistogram::new());
    PerfRegistry::global().register_histogram("strategy.signal_strength", h.clone());
    h
});