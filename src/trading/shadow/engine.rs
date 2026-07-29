//! ShadowEngine — the core shadow-mode loop.
//!
//! Connects to the ShredStream pipeline (via ShredstreamAdapter), reads
//! classified events, maintains a simple event-driven state, evaluates a
//! directional strategy, records every hypothetical decision in the
//! DecisionBuffer, and registers directional decisions with the AccuracyChecker.
//!
//! # Design
//!
//! The engine runs on a background tokio task. It wakes on:
//!
//! 1. **Classified events** from ShredStream — each swap event updates the
//!    momentum tracker and may trigger a strategy evaluation.
//! 2. **Periodic accuracy check** — every 5 seconds, evaluates pending decisions
//!    against the current midpoint price.
//!
//! No wallet keypair is loaded. No transactions are constructed or submitted.
//! This is a pure observation + simulation pipeline.

use crate::common::fast_timing;
use crate::perf::shredstream::classifier::{ClassifiedEvent, EventType};
use crate::perf::shredstream::ShredstreamAdapter;
use crate::trading::shadow::accuracy::AccuracyChecker;
use crate::trading::shadow::decision::{DecisionBuffer, ShadowDecision, ShadowSignal};

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
// Momentum tracker (event-driven, no order book required)
// ---------------------------------------------------------------------------

/// Tracks buy/sell swap event momentum over a sliding window.
struct MomentumTracker {
    /// Sliding window of (timestamp_micros, is_buy) pairs.
    window_micros: i64,
    events: Vec<(i64, bool)>,
}

impl MomentumTracker {
    fn new(window_micros: i64) -> Self {
        Self {
            window_micros,
            events: Vec::with_capacity(256),
        }
    }

    fn record(&mut self, timestamp_micros: i64, is_buy: bool) {
        self.events.push((timestamp_micros, is_buy));
        self.evict_old(timestamp_micros);
    }

    fn evict_old(&mut self, now: i64) {
        let cutoff = now - self.window_micros;
        self.events.retain(|(ts, _)| *ts >= cutoff);
    }

    /// Momentum in [-1.0, +1.0]. +1.0 = all buys, -1.0 = all sells, 0.0 = equal or no data.
    fn momentum(&mut self, now: i64) -> f64 {
        self.evict_old(now);
        if self.events.is_empty() {
            return 0.0;
        }
        let buys = self.events.iter().filter(|(_, is_buy)| *is_buy).count();
        let total = self.events.len();
        (buys as f64 / total as f64) * 2.0 - 1.0
    }

    fn total_in_window(&self) -> usize {
        self.events.len()
    }
}

// ---------------------------------------------------------------------------
// ShadowEngine
// ---------------------------------------------------------------------------

/// The shadow mode engine. Runs the full evaluation pipeline without submitting
/// any transactions.
pub struct ShadowEngine {
    adapter: ShredstreamAdapter,
    decisions: Arc<DecisionBuffer>,
    accuracy: Arc<AccuracyChecker>,
    momentum: MomentumTracker,
    /// Last known midpoint price (inferred from swap events).
    last_price: f64,
    /// Spread tracking.
    last_spread_cents: f64,
    /// Evaluation interval in microseconds (default: 200ms = 200_000).
    eval_interval_micros: i64,
    /// Last evaluation time (micros).
    last_eval_micros: i64,
    /// Counter for total evaluations.
    eval_count: u64,
}

impl ShadowEngine {
    /// Create a new shadow engine from a ShredStream adapter.
    pub fn new(adapter: ShredstreamAdapter) -> Self {
        Self {
            adapter,
            decisions: Arc::new(DecisionBuffer::default()),
            accuracy: Arc::new(AccuracyChecker::default()),
            momentum: MomentumTracker::new(2_000_000), // 2-second window
            last_price: 100.0,  // placeholder initial price
            last_spread_cents: 1.0,
            eval_interval_micros: 200_000, // 200ms
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

    // ------------------------------------------------------------------
    // Evaluate from a classified event
    // ------------------------------------------------------------------

    /// Process a single classified event and optionally generate a shadow decision.
    fn evaluate_event(&mut self, event: &ClassifiedEvent) -> Option<ShadowDecision> {
        let now = fast_timing::fast_now_micros() as i64;
        let eval_start = now;

        // Determine if this is a buy or sell from the raw data (best-effort)
        let is_buy = classify_swap_direction(event);
        self.momentum.record(event.received_at_micros, is_buy);

        #[cfg(feature = "perf-trace")]
        perf_metrics::EVENTS_PROCESSED.increment(1);

        // Time-based evaluation gate: only evaluate every `eval_interval_micros`
        if now - self.last_eval_micros < self.eval_interval_micros {
            return None;
        }
        self.last_eval_micros = now;

        // In a real shadow setup, this would use an order book from RPC.
        // For now, derive a synthetic midpoint from the event context:
        let momentum = self.momentum.momentum(now);
        let midpoint_price = self.last_price;
        let spread_cents = self.last_spread_cents;

        // Simple momentum-based strategy:
        // - Strong buy momentum (> +0.3) → BUY signal
        // - Strong sell momentum (< -0.3) → SELL signal
        // - Otherwise → HOLD
        let (signal, confidence, reason) = if momentum > 0.3 {
            let c = ((momentum - 0.3) / 0.7).min(1.0);
            (ShadowSignal::Buy, c, format!("Buy momentum {:.3} ({} events)", momentum, self.momentum.total_in_window()))
        } else if momentum < -0.3 {
            let c = ((-momentum - 0.3) / 0.7).min(1.0);
            (ShadowSignal::Sell, c, format!("Sell momentum {:.3} ({} events)", momentum, self.momentum.total_in_window()))
        } else {
            let reason = format!("Neutral momentum {:.3} ({} events)", momentum, self.momentum.total_in_window());
            (ShadowSignal::Hold, 0.0, reason)
        };

        let latency = fast_timing::fast_now_micros().saturating_sub(eval_start as u64);

        #[cfg(feature = "perf-trace")]
        {
            match signal {
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
            signal,
            confidence,
            spread_cents,
            midpoint_price,
            latency,
            reason,
        );

        Some(decision)
    }

    // ------------------------------------------------------------------
    // Main run loop (called from the binary)
    // ------------------------------------------------------------------

    /// Run the shadow engine main loop.
    ///
    /// Reads classified events from the ShredStream adapter, evaluates them,
    /// records decisions, and periodically runs accuracy checks.
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
                self.accuracy.check_cycle(self.last_price);
                accuracy_check_at = Instant::now() + accuracy_check_interval;
            }

            // Try to receive an event with a short timeout so accuracy checks aren't starved
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    // Extract price signal from the event if available
                    self.update_price_from_event(&event);

                    if let Some(decision) = self.evaluate_event(&event) {
                        let id = self.decisions.push(decision);
                        self.accuracy.register_decision(
                            &self.decisions.recent(1)[0], // push already added it
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

        info!("ShadowEngine shutting down — {} evaluations, {} total decisions",
            self.eval_count, self.decisions.total_decisions());
    }

    /// Update the synthetic price signal from a classified event.
    fn update_price_from_event(&mut self, event: &ClassifiedEvent) {
        // For now, use a simple heuristic: the program ID and event type give
        // us a rough price signal. Full order-book-based pricing will come in
        // later shadow iterations.
        //
        // Here we derive a synthetic "price" from the instruction data checksum
        // as a placeholder. In production, this would come from the RPC order book.
        if !event.raw_instruction_data.is_empty() {
            // Use a simple random-walk perturbation so accuracy tracking has signal
            let noise = (event.slot as f64 * 0.0001).sin() * 0.1;
            if event.event_type == EventType::Swap {
                // Swap events mean we have a real transaction — assume price stability
                self.last_price = self.last_price + noise * 0.01;
            }
            // In production, this would be overridden by actual RPC price data
        }
    }
}

/// Best-effort classification of swap direction from instruction data.
fn classify_swap_direction(event: &ClassifiedEvent) -> bool {
    // Raydium AMM V4: [9] = swap_base_in (buy), [11] = swap_base_out (sell)
    if let Some(&first) = event.raw_instruction_data.first() {
        match first {
            9 => return true,   // buy
            11 => return false, // sell
            _ => {}
        }
    }

    // PumpFun: SHA256("global:buy") and SHA256("global:sell")
    if event.raw_instruction_data.len() >= 8 {
        let disc: [u8; 8] = match event.raw_instruction_data[..8].try_into() {
            Ok(d) => d,
            Err(_) => return false,
        };
        // buy: [102, 6, 61, 18, 1, 218, 235, 234]
        if disc == [102, 6, 61, 18, 1, 218, 235, 234] {
            return true;
        }
        // sell: [51, 230, 181, 142, 76, 52, 84, 155]
        if disc == [51, 230, 181, 142, 76, 52, 84, 155] {
            return false;
        }
    }

    // Default: treat unknown events as neutral (buy)
    true
}

// Use tracing::trace! directly for all trace logging.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::shredstream::classifier::EventType;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::Signature;

    fn make_test_event(
        slot: u64,
        event_type: EventType,
        data: Vec<u8>,
        is_buy: bool,
    ) -> ClassifiedEvent {
        ClassifiedEvent {
            slot,
            signature: Signature::new_unique(),
            event_type,
            instruction_index: 0,
            program_id: Pubkey::new_from_array([0u8; 32]),
            received_at_micros: crate::common::fast_timing::fast_now_micros() as i64,
            decoded_at_micros: crate::common::fast_timing::fast_now_micros() as i64,
            raw_instruction_data: data,
        }
    }

    #[test]
    fn test_momentum_tracker_basic() {
        let mut mt = MomentumTracker::new(1_000_000); // 1s window
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // Record 3 buys, 1 sell
        mt.record(now - 500_000, true);
        mt.record(now - 400_000, false);
        mt.record(now - 300_000, true);
        mt.record(now - 200_000, true);

        let m = mt.momentum(now);
        assert!((m - 0.5).abs() < 0.01, "Expected ~0.5 momentum, got {m}");

        // Record outside window
        mt.record(now - 2_000_000, false);
        let m2 = mt.momentum(now);
        assert!((m2 - 0.5).abs() < 0.01, "Old events should be evicted");
    }

    #[test]
    fn test_classify_swap_direction_raydium() {
        // Raydium AMM V4 swap_base_in = [9]
        let event = make_test_event(0, EventType::Swap, vec![9, 0, 0, 0], false);
        assert!(classify_swap_direction(&event));

        // Raydium AMM V4 swap_base_out = [11]
        let event = make_test_event(0, EventType::Swap, vec![11, 0, 0, 0], false);
        assert!(!classify_swap_direction(&event));
    }

    #[test]
    fn test_classify_swap_direction_pumpfun() {
        let event = make_test_event(0, EventType::Swap, vec![102, 6, 61, 18, 1, 218, 235, 234], false);
        assert!(classify_swap_direction(&event)); // buy

        let event = make_test_event(0, EventType::Swap, vec![51, 230, 181, 142, 76, 52, 84, 155], false);
        assert!(!classify_swap_direction(&event)); // sell
    }
}