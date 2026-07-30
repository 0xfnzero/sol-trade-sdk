//! StrategyEngine — the core signal evaluation pipeline.
//!
//! Orchestrates the multi-factor signal evaluation, applies gates, and produces
//! `StrategyOutcome` (TradeSignal or NoTrade). Designed to be shared between
//! Shadow Mode (observation) and Production Mode (execution).
//!
//! # Architecture
//!
//! ```text
//! ClassifiedEvent ──→ MarketStateTracker.update()
//!                         │
//!                     evaluate() [time-gated, configurable interval]
//!                         │
//!                    SignalFactors.evaluate()
//!                         │
//!                    SignalAggregator.gates()
//!                         │
//!                    StrategyOutcome
//!                    ├── Trade(TradeSignal) → Orchestrator or ShadowDecision
//!                    └── NoTrade(reason)
//! ```

use crate::perf::shredstream::classifier::ClassifiedEvent;
use crate::trading::strategy::factors::{FactorConfig, FactorOutput, SignalFactors};
use crate::trading::strategy::market_state::MarketStateTracker;
use crate::trading::strategy::signal::{
    FactorContribution, NoTrade, SignalStrength, StrategyOutcome, TradeSignal,
};

use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Gate configuration
// ---------------------------------------------------------------------------

/// Configuration for the signal aggregation gates.
#[derive(Debug, Clone)]
pub struct GateConfig {
    /// Minimum composite score to produce a directional signal (default: 0.25).
    pub min_composite_score: f64,
    /// Minimum confidence for a signal to pass (default: 0.4).
    pub min_confidence: f64,
    /// Maximum spread in bps for entry (default: 50 bps).
    pub max_spread_bps: f64,
    /// Maximum age of last update in microseconds (default: 10s).
    pub max_age_micros: i64,
    /// Minimum event rate (events/sec) to trust the signal (default: 1.0).
    pub min_event_rate: f64,
    /// Whether to check the event rate gate.
    pub enable_event_rate_gate: bool,
    /// Whether to enable fee-aware EV gate (simplified).
    pub enable_ev_gate: bool,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            min_composite_score: 0.25,
            min_confidence: 0.40,
            max_spread_bps: 50.0,
            max_age_micros: 10_000_000, // 10 seconds
            min_event_rate: 1.0,
            enable_event_rate_gate: true,
            enable_ev_gate: false,
        }
    }
}

// ---------------------------------------------------------------------------
// StrategyConfig — full config for the strategy engine
// ---------------------------------------------------------------------------

/// Full configuration for the StrategyEngine.
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    /// Evaluation interval in microseconds (default: 200ms = 200_000).
    pub eval_interval_micros: i64,
    /// Stale mint threshold in microseconds (default: 30s).
    pub stale_mint_threshold_micros: i64,
    /// Maximum number of tracked mints (default: 100).
    pub max_tracked_mints: usize,
    /// Gate configuration.
    pub gates: GateConfig,
    /// Per-factor configurations.
    pub factors: SignalFactors,
    /// Stale data threshold for eviction (default: 30s).
    pub eviction_stale_micros: i64,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            eval_interval_micros: 200_000, // 200ms
            stale_mint_threshold_micros: 30_000_000,
            max_tracked_mints: 100,
            gates: GateConfig::default(),
            factors: SignalFactors::default(),
            eviction_stale_micros: 30_000_000,
        }
    }
}

// ---------------------------------------------------------------------------
// StrategyEngine
// ---------------------------------------------------------------------------

/// The core strategy engine. Evaluates market state against configured
/// signal factors and produces trade signals or no-trade decisions.
pub struct StrategyEngine {
    /// Configuration.
    pub config: StrategyConfig,
    /// Market state tracker (per-mint).
    pub market_state: MarketStateTracker,
    /// Signal factors.
    pub factors: SignalFactors,
    /// Last evaluation time (monotonic micros).
    last_eval_micros: i64,
    /// Total evaluations performed.
    pub eval_count: u64,
    /// Total signals generated.
    pub signal_count: u64,
    /// Total no-trade decisions.
    pub notrade_count: u64,
    /// Last eviction run time.
    last_eviction: Instant,
    /// Eviction interval.
    eviction_interval: Duration,

    // ── Perf counters (gated) ──
    #[cfg(feature = "perf-trace")]
    eval_ns: std::sync::Arc<crate::perf::LatencyHistogram>,
    #[cfg(feature = "perf-trace")]
    signal_counter: u64,
    #[cfg(feature = "perf-trace")]
    notrade_counter: u64,
}

impl StrategyEngine {
    /// Create a new strategy engine with the given configuration.
    pub fn new(config: StrategyConfig) -> Self {
        let factors = config.factors.clone();

        #[cfg(feature = "perf-trace")]
        let eval_ns = {
            let h = std::sync::Arc::new(crate::perf::LatencyHistogram::new());
            crate::perf::PerfRegistry::global()
                .register_histogram("strategy.evaluate_ns", h.clone());
            h
        };

        Self {
            market_state: MarketStateTracker::with_config(
                config.eviction_stale_micros,
                config.max_tracked_mints,
            ),
            factors,
            last_eval_micros: 0,
            eval_count: 0,
            signal_count: 0,
            notrade_count: 0,
            last_eviction: Instant::now(),
            eviction_interval: Duration::from_secs(10),
            config,

            #[cfg(feature = "perf-trace")]
            eval_ns,
            #[cfg(feature = "perf-trace")]
            signal_counter: 0,
            #[cfg(feature = "perf-trace")]
            notrade_counter: 0,
        }
    }

    /// Create a default strategy engine.
    pub fn default_with_config() -> Self {
        Self::new(StrategyConfig::default())
    }

    // ── Event ingestion ──

    /// Process a classified event from the ShredStream pipeline.
    ///
    /// Uses the event's program_id as a proxy for the mint component of the
    /// composite key. This provides program-level isolation until the classifier
    /// is augmented to carry the actual token mint address.
    pub fn process_event(&mut self, event: &ClassifiedEvent, now_micros: i64) {
        // Use program_id as mint proxy since ClassifiedEvent doesn't carry
        // the token mint address yet. This provides per-program isolation.
        self.market_state
            .update(&event.program_id, &event.program_id.to_string(), event, now_micros);

        // Periodic eviction of stale mints
        if self.last_eviction.elapsed() >= self.eviction_interval {
            self.market_state.evict_stale(now_micros);
            self.market_state.enforce_max_mints();
            self.last_eviction = Instant::now();
        }
    }

    // ── Strategy evaluation ──

    /// Evaluate the current market state and produce a strategy outcome.
    ///
    /// Returns `Some(StrategyOutcome)` if the evaluation interval has elapsed,
    /// or `None` if it's too early for another evaluation.
    pub fn evaluate(
        &mut self,
        mint: &str,
        protocol: &str,
        slot: u64,
        midpoint_price: f64,
        now_micros: i64,
    ) -> Option<StrategyOutcome> {
        #[cfg(feature = "perf-trace")]
        let start_ns = crate::common::fast_timing::fast_now_micros() * 1000;

        // Time-based evaluation gate
        if now_micros - self.last_eval_micros < self.config.eval_interval_micros {
            return None;
        }
        self.last_eval_micros = now_micros;

        // ── Scope: mutable access to market_state ──
        // Extract all needed data from state, then release the mutable borrow.
        let (outputs, composite, spread_bps, age_micros, event_rate) = {
            // Try exact mint match first, then fallback to protocol-level match
            // This bridges the gap between update() storing with (program_id, program_id)
            // and evaluate() receiving (protocol, mint) from the caller.
            // TODO: When the classifier carries the actual token mint, update() will store
            // with (program_id, real_mint) and this fallback can be removed.
            let state = match self.market_state.find_by_mint_mut(mint) {
                Some(s) => s,
                None => match self.market_state.find_by_mint_mut(protocol) {
                    Some(s) => s,
                    None => {
                        self.notrade_count += 1;
                        return Some(StrategyOutcome::NoTrade(NoTrade {
                            reason: format!("No market state for mint {mint}"),
                            composite_score: 0.0,
                            factor_breakdown: vec![],
                            rejected_by: Some("state_missing".into()),
                        }));
                    }
                },
            };

            // Evaluate factors (takes &mut for momentum eviction)
            let (outputs, composite) = self.factors.evaluate(state, now_micros, protocol);
            let spread_bps = state.spread_bps;
            let age_micros = state.age_micros(now_micros);
            let event_rate = state.event_rate_1s();
            (outputs, composite, spread_bps, age_micros, event_rate)
        };
        // ── market_state mutable borrow released ──

        let factor_breakdown: Vec<FactorContribution> =
            outputs.iter().map(|o| o.to_factor_contribution()).collect();

        // Apply gates (only needs &self, no market state borrow)
        let gate_result = self.apply_gates(
            &outputs,
            composite,
            spread_bps,
            age_micros,
            event_rate,
            now_micros,
        );

        #[cfg(feature = "perf-trace")]
        {
            let elapsed_ns = (crate::common::fast_timing::fast_now_micros() * 1000)
                .saturating_sub(start_ns);
            if let Ok(h) = self.eval_ns.clone().lock() {
                h.record(elapsed_ns);
            }
        }

        self.eval_count += 1;

        match gate_result {
            Ok(signal) => {
                // Signal passed all gates
                let (strength, confidence) = self.composite_to_signal(signal, composite);

                self.signal_count += 1;
                #[cfg(feature = "perf-trace")]
                {
                    self.signal_counter += 1;
                }

                Some(StrategyOutcome::Trade(TradeSignal {
                    mint: mint.to_string(),
                    strength,
                    confidence,
                    protocol: protocol.to_string(),
                    spread_bps,
                    midpoint_price,
                    slot,
                    composite_score: composite,
                    factor_breakdown,
                    reason: format!(
                        "composite={:.3} threshold={:.2}",
                        composite, self.config.gates.min_composite_score
                    ),
                }))
            }
            Err(gate_name) => {
                self.notrade_count += 1;
                #[cfg(feature = "perf-trace")]
                {
                    self.notrade_counter += 1;
                }

                Some(StrategyOutcome::NoTrade(NoTrade {
                    reason: format!("Gate '{gate_name}' rejected: composite={:.3}", composite),
                    composite_score: composite,
                    factor_breakdown,
                    rejected_by: Some(gate_name),
                }))
            }
        }
    }

    // ── Gate application ──

    /// Apply all gates to a composite score.
    /// Returns `Ok(adjusted_score)` if all gates pass, or `Err(gate_name)` on failure.
    fn apply_gates(
        &self,
        outputs: &[FactorOutput],
        composite: f64,
        spread_bps: f64,
        state_age_micros: i64,
        state_event_rate_1s: f64,
        _now_micros: i64,
    ) -> Result<f64, String> {
        // Gate 1: Composite score threshold
        if composite.abs() < self.config.gates.min_composite_score {
            return Err("min_composite_score".into());
        }

        // Gate 2: Spread gate
        if spread_bps > self.config.gates.max_spread_bps {
            return Err("max_spread".into());
        }

        // Gate 3: Stale data gate
        if state_age_micros > self.config.gates.max_age_micros {
            return Err("stale_data".into());
        }

        // Gate 4: Event rate gate (optional)
        if self.config.gates.enable_event_rate_gate {
            if state_event_rate_1s < self.config.gates.min_event_rate {
                return Err("min_event_rate".into());
            }
        }

        // Gate 5: Min confidence across all active factors
        if self.config.gates.min_confidence > 0.0 {
            let factors_with_weight: Vec<&FactorOutput> = outputs
                .iter()
                .filter(|o| o.weight > 0.0 && o.name != "protocol_confidence")
                .collect();
            if !factors_with_weight.is_empty() {
                // The momentum factor must have at least minimum confidence
                let momentum = outputs.iter().find(|o| o.name == "momentum");
                if let Some(m) = momentum {
                    if m.value.abs() > 0.0 && m.confidence < self.config.gates.min_confidence {
                        return Err("min_confidence".into());
                    }
                }
            }
        }

        Ok(composite)
    }

    /// Convert a composite score and confidence into a SignalStrength.
    fn composite_to_signal(&self, score: f64, _composite: f64) -> (SignalStrength, f64) {
        let confidence = score.abs().min(1.0);
        let strength = if score > 0.6 {
            SignalStrength::StrongBuy
        } else if score > 0.25 {
            SignalStrength::Buy
        } else if score < -0.6 {
            SignalStrength::StrongSell
        } else if score < -0.25 {
            SignalStrength::Sell
        } else {
            SignalStrength::Neutral
        };
        (strength, confidence)
    }

    // ── Stats ──

    /// Get a snapshot of engine statistics.
    pub fn stats(&self) -> StrategyStats {
        StrategyStats {
            eval_count: self.eval_count,
            signal_count: self.signal_count,
            notrade_count: self.notrade_count,
            tracked_mints: self.market_state.len(),
            eviction_count: self.market_state.eviction_count(),
        }
    }

    /// Reset the evaluation timer and all counters.
    pub fn reset(&mut self) {
        self.last_eval_micros = 0;
        self.eval_count = 0;
        self.signal_count = 0;
        self.notrade_count = 0;
        self.market_state.clear();
    }
}

// ---------------------------------------------------------------------------
// StrategyStats
// ---------------------------------------------------------------------------

/// Snapshot of strategy engine statistics.
#[derive(Debug, Clone, Copy)]
pub struct StrategyStats {
    pub eval_count: u64,
    pub signal_count: u64,
    pub notrade_count: u64,
    pub tracked_mints: usize,
    pub eviction_count: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perf::shredstream::classifier::EventType;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::Signature;

    fn make_test_event(slot: u64, data: Vec<u8>) -> ClassifiedEvent {
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        ClassifiedEvent {
            slot,
            signature: Signature::new_unique(),
            event_type: EventType::Swap,
            instruction_index: 0,
            program_id: "So11111111111111111111111111111111111111112".parse().unwrap(),
            received_at_micros: now,
            decoded_at_micros: now,
            raw_instruction_data: data,
            trace: crate::perf::shredstream::EventTrace::new(now, now),
        }
    }

    fn setup_engine_with_bullish_data() -> StrategyEngine {
        let config = StrategyConfig {
            eval_interval_micros: 100_000, // 100ms for fast tests
            ..Default::default()
        };
        let mut engine = StrategyEngine::new(config);
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let mint = "So11111111111111111111111111111111111111112";

        // Feed buy-heavy events
        for i in 0..10 {
            let event = make_test_event(i as u64, vec![9, 0, 0, 0]); // Raydium buy
            engine.process_event(&event, now - 500_000 + (i as i64 * 100_000));
        }

        // Add a few sell events (minority)
        for i in 0..3 {
            let event = make_test_event(i as u64, vec![11, 0, 0, 0]); // Raydium sell
            engine.process_event(&event, now - 500_000 + (i as i64 * 100_000));
        }

        engine
    }

    #[test]
    fn test_engine_creates_trade_signal_in_bullish_scenario() {
        let mut engine = setup_engine_with_bullish_data();
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let mint = "So11111111111111111111111111111111111111112";

        // Set the state spread tight
        if let Some(state) = engine.market_state.get_mut(mint, mint) {
            state.spread_bps = 8.0;
            state.last_update_micros = now - 100_000;
        }

        let outcome = engine.evaluate(mint, "raydium_amm_v4", 100, 1.0, now);
        assert!(outcome.is_some(), "Should produce an outcome");

        if let Some(outcome) = outcome {
            assert!(outcome.is_trade(), "Should produce a trade signal in bullish scenario");
            let composite = outcome.composite_score();
            assert!(composite > 0.0, "Composite should be positive: {composite}");
        }
    }

    #[test]
    fn test_engine_rejects_on_wide_spread() {
        let mut engine = setup_engine_with_bullish_data();
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let mint = "So11111111111111111111111111111111111111112";

        // Set very wide spread
        if let Some(state) = engine.market_state.get_mut(mint, mint) {
            state.spread_bps = 100.0;
            state.last_update_micros = now - 100_000;
        }

        let outcome = engine.evaluate(mint, "raydium_amm_v4", 100, 1.0, now);
        assert!(outcome.is_some());
        if let Some(outcome) = outcome {
            assert!(!outcome.is_trade(), "Should reject on wide spread");
            if let StrategyOutcome::NoTrade(nt) = outcome {
                assert_eq!(nt.rejected_by.as_deref(), Some("max_spread"));
            }
        }
    }

    #[test]
    fn test_engine_rejects_on_stale_data() {
        let mut engine = setup_engine_with_bullish_data();
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let mint = "So11111111111111111111111111111111111111112";

        // Set very old data
        if let Some(state) = engine.market_state.get_mut(mint, mint) {
            state.spread_bps = 8.0;
            state.last_update_micros = now - 15_000_000; // 15s old
        }

        let outcome = engine.evaluate(mint, "raydium_amm_v4", 100, 1.0, now);
        assert!(outcome.is_some());
        if let Some(outcome) = outcome {
            assert!(!outcome.is_trade(), "Should reject on stale data");
        }
    }

    #[test]
    fn test_engine_stats() {
        let mut engine = setup_engine_with_bullish_data();
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let mint = "So11111111111111111111111111111111111111112";

        if let Some(state) = engine.market_state.get_mut(mint, mint) {
            state.spread_bps = 8.0;
            state.last_update_micros = now - 100_000;
        }

        let _ = engine.evaluate(mint, "raydium_amm_v4", 100, 1.0, now);
        let stats = engine.stats();
        assert!(stats.tracked_mints > 0);
        assert!(stats.eval_count > 0);
    }

    #[test]
    fn test_eval_interval_gate() {
        let mut engine = StrategyEngine::new(StrategyConfig {
            eval_interval_micros: 1_000_000, // 1s interval
            ..Default::default()
        });
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let mint = "So11111111111111111111111111111111111111112";
        let event = make_test_event(1, vec![9]);
        engine.process_event(&event, now);

        // First eval: no previous eval, so it should produce an outcome
        // With neutral data (only 1 event), it will be a NoTrade
        let outcome = engine.evaluate(mint, "pumpfun", 1, 1.0, now);
        assert!(outcome.is_some(), "First eval should produce an outcome");

        // But the engine state was updated
        assert_eq!(engine.market_state.len(), 1);
    }
}