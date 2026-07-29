//! ShadowEngine — the core shadow-mode loop.
//!
//! Connects to the ShredStream pipeline (via ShredstreamAdapter), reads
//! classified events, maintains market state via the shared StrategyEngine,
//! evaluates the multi-factor strategy, records every hypothetical decision
//! in the DecisionBuffer, and registers directional decisions with the
//! AccuracyChecker.
//!
//! # Design
//!
//! The engine runs on a background tokio task. On each classified event:
//!
//! 1. Feeds event to `StrategyEngine::process_event()` (updates per-mint state)
//! 2. Calls `StrategyEngine::evaluate()` (time-gated, 200ms interval)
//! 3. Maps `TradeSignal` → `ShadowDecision`, records in buffer + accuracy checker
//!
//! No wallet keypair is loaded. No transactions are constructed or submitted.
//! This is a pure observation + simulation pipeline.

use crate::common::fast_timing;
use crate::perf::shredstream::classifier::ClassifiedEvent;
use crate::perf::shredstream::ShredstreamAdapter;
use crate::trading::shadow::accuracy::AccuracyChecker;
use crate::trading::shadow::decision::{DecisionBuffer, ShadowDecision, ShadowSignal};
use crate::trading::strategy::{
    SignalStrength, StrategyConfig, StrategyEngine, StrategyOutcome,
};

use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info, trace, warn};

// ---------------------------------------------------------------------------
// Perf-trace counters (gated behind `perf-trace` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "perf-trace")]
mod perf_metrics {
    use crate::perf::{LatencyHistogram, PerfCounter, PerfRegistry};
    use once_cell::sync::Lazy;
    use std::sync::Arc;

    pub static EVAL_NS: Lazy<Arc<LatencyHistogram>> = Lazy::new(|| {
        let h = Arc::new(LatencyHistogram::new());
        PerfRegistry::global().register_histogram("shadow.evaluate_ns", h.clone());
        h
    });

    pub static BUY_SIGNALS: PerfCounter = PerfCounter::new("shadow.buy_signals");
    pub static SELL_SIGNALS: PerfCounter = PerfCounter::new("shadow.sell_signals");
    pub static HOLD_SIGNALS: PerfCounter = PerfCounter::new("shadow.hold_signals");
    pub static EVENTS_PROCESSED: PerfCounter = PerfCounter::new("shadow.events_processed");
}

#[cfg(not(feature = "perf-trace"))]
mod perf_metrics {
    // No-op stubs — everything compiles away to zero overhead.
}

// ---------------------------------------------------------------------------
// ShadowEngine
// ---------------------------------------------------------------------------

/// The shadow mode engine. Runs the full strategy evaluation pipeline without
/// submitting any transactions.
///
/// Uses the shared [`StrategyEngine`] for multi-factor signal evaluation.
pub struct ShadowEngine {
    adapter: ShredstreamAdapter,
    decisions: Arc<DecisionBuffer>,
    accuracy: Arc<AccuracyChecker>,
    /// The shared strategy engine (per-mint market state + multi-factor signals).
    strategy_engine: StrategyEngine,
    /// Last evaluation time (micros) — separate from strategy engine's own gate.
    last_eval_micros: i64,
    /// Counter for total evaluations.
    eval_count: u64,
}

impl ShadowEngine {
    /// Create a new shadow engine from a ShredStream adapter.
    pub fn new(adapter: ShredstreamAdapter) -> Self {
        Self::with_strategy(adapter, StrategyConfig::default())
    }

    /// Create a new shadow engine with a custom strategy configuration.
    pub fn with_strategy(adapter: ShredstreamAdapter, strategy_config: StrategyConfig) -> Self {
        Self {
            adapter,
            decisions: Arc::new(DecisionBuffer::default()),
            accuracy: Arc::new(AccuracyChecker::default()),
            strategy_engine: StrategyEngine::new(strategy_config),
            last_eval_micros: 0,
            eval_count: 0,
        }
    }

    /// Returns a reference to the decision buffer.
    pub fn decisions(&self) -> &Arc<DecisionBuffer> {
        &self.decisions
    }

    /// Returns a reference to the accuracy checker.
    pub fn accuracy(&self) -> &Arc<AccuracyChecker> {
        &self.accuracy
    }

    /// Returns a reference to the ShredStream adapter (for metrics access).
    pub fn adapter(&self) -> &ShredstreamAdapter {
        &self.adapter
    }

    /// Returns a reference to the strategy engine (for stats access).
    pub fn strategy_engine(&self) -> &StrategyEngine {
        &self.strategy_engine
    }

    // ------------------------------------------------------------------
    // Evaluate from a classified event
    // ------------------------------------------------------------------

    /// Process a single classified event through the strategy engine and
    /// optionally generate a shadow decision.
    fn evaluate_event(&mut self, event: &ClassifiedEvent) -> Option<ShadowDecision> {
        let now = fast_timing::fast_now_micros() as i64;
        let eval_start = now;

        // Feed the event into the strategy engine (updates per-mint state)
        self.strategy_engine.process_event(event, now);

        #[cfg(feature = "perf-trace")]
        perf_metrics::EVENTS_PROCESSED.increment(1);

        // Time-based evaluation gate: only evaluate every `eval_interval_micros`
        if now - self.last_eval_micros < self.strategy_engine.config.eval_interval_micros {
            return None;
        }
        self.last_eval_micros = now;

        // Get the mint (program_id) and evaluate
        let mint = event.program_id.to_string();
        let protocol = infer_protocol(&event);

        // Call the strategy engine
        let outcome = self.strategy_engine.evaluate(
            &mint,
            protocol,
            event.slot,
            self.last_known_price(event),
            now,
        );

        let outcome = match outcome {
            Some(o) => o,
            None => return None,
        };

        let latency = fast_timing::fast_now_micros().saturating_sub(eval_start as u64);

        match outcome {
            StrategyOutcome::Trade(signal) => {
                let shadow_signal = match signal.strength {
                    SignalStrength::StrongBuy | SignalStrength::Buy => ShadowSignal::Buy,
                    SignalStrength::StrongSell | SignalStrength::Sell => ShadowSignal::Sell,
                    SignalStrength::Neutral => ShadowSignal::Hold,
                };

                #[cfg(feature = "perf-trace")]
                {
                    match shadow_signal {
                        ShadowSignal::Buy => perf_metrics::BUY_SIGNALS.increment(1),
                        ShadowSignal::Sell => perf_metrics::SELL_SIGNALS.increment(1),
                        ShadowSignal::Hold => perf_metrics::HOLD_SIGNALS.increment(1),
                    }
                    if let Ok(h) = perf_metrics::EVAL_NS.clone().lock() {
                        h.record(latency * 1000);
                    }
                }

                let decision = ShadowDecision::new(
                    event.slot,
                    shadow_signal,
                    signal.confidence,
                    signal.spread_bps,
                    signal.midpoint_price,
                    latency,
                    signal.reason,
                );

                Some(decision)
            }
            StrategyOutcome::NoTrade(_) => {
                #[cfg(feature = "perf-trace")]
                perf_metrics::HOLD_SIGNALS.increment(1);

                None // NoTrade doesn't create a shadow decision
            }
        }
    }

    /// Get a synthetic last-known price from the strategy engine.
    fn last_known_price(&self, event: &ClassifiedEvent) -> f64 {
        let mint = event.program_id.to_string();
        self.strategy_engine
            .market_state
            .get(&mint)
            .map(|s| s.last_price)
            .unwrap_or(100.0)
    }

    // ------------------------------------------------------------------
    // Main run loop (called from the binary)
    // ------------------------------------------------------------------

    /// Run the shadow engine main loop.
    ///
    /// Reads classified events from the ShredStream adapter, evaluates them
    /// through the StrategyEngine, records decisions, and periodically runs
    /// accuracy checks.
    pub async fn run(&mut self) {
        info!("ShadowEngine starting — live feed, no submission");

        // Start the ShredStream pipeline
        if let Err(e) = self.adapter.start() {
            error!("Failed to start ShredStream adapter: {e}");
            return;
        }

        info!("ShredStream pipeline started");

        let accuracy_check_interval = Duration::from_secs(5);
        let mut accuracy_check_at = Instant::now() + accuracy_check_interval;

        // The classifier event channel — clone to avoid borrow conflicts
        let rx = self.adapter.classified_event_rx().clone();

        loop {
            // Check for accuracy cycle
            if Instant::now() >= accuracy_check_at {
                // Use a fallback price for accuracy checking
                self.accuracy.check_cycle(100.0);
                accuracy_check_at = Instant::now() + accuracy_check_interval;
            }

            // Try to receive an event with a short timeout so accuracy checks aren't starved
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    if let Some(decision) = self.evaluate_event(&event) {
                        let id = self.decisions.push(decision);
                        self.accuracy.register_decision(
                            &self.decisions.recent(1)[0],
                        );
                        trace!("Shadow decision #{}: signal", id);
                        self.eval_count += 1;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // Normal — just means nothing arrived this cycle
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    warn!("ShredStream channel disconnected — all events consumed");
                    break;
                }
            }
        }

        info!(
            "ShadowEngine shutting down — {} evaluations, {} total decisions",
            self.eval_count,
            self.decisions.total_decisions()
        );
    }
}

/// Best-effort protocol inference from instruction data.
fn infer_protocol(event: &ClassifiedEvent) -> &'static str {
    // Check first byte for Raydium AMM V4
    if let Some(&first) = event.raw_instruction_data.first() {
        match first {
            9 | 11 => return "raydium_amm_v4",
            _ => {}
        }
    }

    // Check for PumpFun discriminators
    if event.raw_instruction_data.len() >= 8 {
        let disc: [u8; 8] = match event.raw_instruction_data[..8].try_into() {
            Ok(d) => d,
            Err(_) => return "unknown",
        };
        // PumpFun buy: [102, 6, 61, 18, 1, 218, 235, 234]
        if disc == [102, 6, 61, 18, 1, 218, 235, 234] {
            return "pumpfun";
        }
        // PumpFun sell: [51, 230, 181, 142, 76, 52, 84, 155]
        if disc == [51, 230, 181, 142, 76, 52, 84, 155] {
            return "pumpfun";
        }
    }

    "unknown"
}