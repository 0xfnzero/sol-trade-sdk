//! Multi-factor signal computation for the Strategy Engine.
//!
//! Each factor evaluates one aspect of the market state and produces a
//! normalized value between -1.0 (strong sell signal) and +1.0 (strong buy
//! signal), along with a confidence and the factor's configurable weight.

use crate::trading::strategy::market_state::MintMarketState;
use crate::trading::strategy::signal::FactorContribution;

// ---------------------------------------------------------------------------
// FactorConfig — per-factor configuration
// ---------------------------------------------------------------------------

/// Configuration for a single signal factor.
#[derive(Debug, Clone)]
pub struct FactorConfig {
    /// Normalized weight in [0.0, 1.0]. The engine normalizes all weights to sum to 1.0.
    pub weight: f64,
    /// Minimum factor value to contribute meaningfully to the composite.
    pub min_contribution: f64,
    /// Whether this factor is enabled.
    pub enabled: bool,
}

impl FactorConfig {
    pub fn new(weight: f64) -> Self {
        Self {
            weight,
            min_contribution: 0.0,
            enabled: true,
        }
    }

    pub fn with_min(mut self, min: f64) -> Self {
        self.min_contribution = min;
        self
    }

    pub fn disabled() -> Self {
        Self {
            weight: 0.0,
            min_contribution: 0.0,
            enabled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// FactorOutput — the result of evaluating one factor
// ---------------------------------------------------------------------------

/// The output of a single factor evaluation.
#[derive(Debug, Clone)]
pub struct FactorOutput {
    /// Factor name (e.g. "momentum").
    pub name: String,
    /// Value in [-1.0, +1.0]. Positive = buy signal, negative = sell signal.
    pub value: f64,
    /// Confidence in [0.0, 1.0].
    pub confidence: f64,
    /// Assigned weight for this factor.
    pub weight: f64,
}

impl FactorOutput {
    /// The weighted contribution to the composite signal.
    pub fn contribution(&self) -> f64 {
        self.value * self.weight * self.confidence
    }

    /// Convert to a `FactorContribution` for the signal's breakdown.
    pub fn to_factor_contribution(&self) -> FactorContribution {
        FactorContribution {
            name: self.name.clone(),
            value: self.value,
            weight: self.weight,
            contribution: self.contribution(),
        }
    }
}

// ---------------------------------------------------------------------------
// FactorWeights — raw weight values for constructing SignalFactors
// ---------------------------------------------------------------------------

/// Raw weight values for each factor. Intended for use with
/// [`SignalFactors::with_weights`] when constructing from config.
#[derive(Debug, Clone, Copy)]
pub struct FactorWeights {
    pub momentum: f64,
    pub volume_profile: f64,
    pub spread: f64,
    pub slot_freshness: f64,
    pub protocol_confidence: f64,
    pub momentum_divergence: f64,
}

impl Default for FactorWeights {
    fn default() -> Self {
        Self {
            momentum: 0.30,
            volume_profile: 0.15,
            spread: 0.20,
            slot_freshness: 0.10,
            protocol_confidence: 0.10,
            momentum_divergence: 0.15,
        }
    }
}

// ---------------------------------------------------------------------------
// SignalFactors — composite factor evaluator
// ---------------------------------------------------------------------------

/// Evaluates all configured signal factors against a mint's market state.
///
/// Each factor produces a `FactorOutput`, and the `aggregate` method computes
/// a weighted composite score in [-1.0, +1.0].
#[derive(Debug, Clone)]
pub struct SignalFactors {
    /// Momentum factor config.
    pub momentum: FactorConfig,
    /// Volume profile factor config.
    pub volume_profile: FactorConfig,
    /// Spread factor config.
    pub spread: FactorConfig,
    /// Slot freshness factor config.
    pub slot_freshness: FactorConfig,
    /// Protocol confidence factor config.
    pub protocol_confidence: FactorConfig,
    /// Momentum divergence factor (fast - slow).
    pub momentum_divergence: FactorConfig,
}

impl Default for SignalFactors {
    fn default() -> Self {
        Self {
            momentum: FactorConfig::new(0.30),
            volume_profile: FactorConfig::new(0.15),
            spread: FactorConfig::new(0.20),
            slot_freshness: FactorConfig::new(0.10),
            protocol_confidence: FactorConfig::new(0.10),
            momentum_divergence: FactorConfig::new(0.15),
        }
    }
}

impl SignalFactors {
    /// Create with custom configs.
    pub fn new(
        momentum: FactorConfig,
        volume_profile: FactorConfig,
        spread: FactorConfig,
        slot_freshness: FactorConfig,
        protocol_confidence: FactorConfig,
        momentum_divergence: FactorConfig,
    ) -> Self {
        Self {
            momentum,
            volume_profile,
            spread,
            slot_freshness,
            protocol_confidence,
            momentum_divergence,
        }
    }

    /// Create from raw weight values. Weights are used as-is (the engine
    /// normalizes via total_weight division during aggregate).
    pub fn with_weights(weights: FactorWeights) -> Self {
        Self {
            momentum: FactorConfig::new(weights.momentum),
            volume_profile: FactorConfig::new(weights.volume_profile),
            spread: FactorConfig::new(weights.spread),
            slot_freshness: FactorConfig::new(weights.slot_freshness),
            protocol_confidence: FactorConfig::new(weights.protocol_confidence),
            momentum_divergence: FactorConfig::new(weights.momentum_divergence),
        }
    }

    /// Evaluate all factors against the given market state.
    ///
    /// `now_micros` is the current time for age/delay calculations.
    /// Returns a tuple of `(Vec<FactorOutput>, composite_score)`.
    pub fn evaluate(
        &self,
        state: &mut MintMarketState,
        now_micros: i64,
        protocol: &str,
    ) -> (Vec<FactorOutput>, f64) {
        let mut outputs = Vec::with_capacity(6);

        // 1. Momentum factor — call directly on state to evict stale events
        if self.momentum.enabled && self.momentum.weight > 0.0 {
            let fast_m = state.momentum.fast_momentum(now_micros);
            let output = self.eval_momentum(fast_m);
            outputs.push(output);
        }

        // 2. Volume profile factor
        if self.volume_profile.enabled && self.volume_profile.weight > 0.0 {
            let output = self.eval_volume_profile(state);
            outputs.push(output);
        }

        // 3. Spread factor
        if self.spread.enabled && self.spread.weight > 0.0 {
            let output = self.eval_spread(state.spread_bps);
            outputs.push(output);
        }

        // 4. Slot freshness factor
        if self.slot_freshness.enabled && self.slot_freshness.weight > 0.0 {
            let output = self.eval_slot_freshness(state, now_micros);
            outputs.push(output);
        }

        // 5. Protocol confidence factor
        if self.protocol_confidence.enabled && self.protocol_confidence.weight > 0.0 {
            let output = self.eval_protocol_confidence(protocol);
            outputs.push(output);
        }

        // 6. Momentum divergence factor — call directly on state to evict stale events
        if self.momentum_divergence.enabled && self.momentum_divergence.weight > 0.0 {
            let divergence = state.momentum.divergence(now_micros);
            let output = self.eval_momentum_divergence(divergence);
            outputs.push(output);
        }

        // Compute composite score (weighted sum, normalized)
        let total_weight: f64 = outputs.iter().map(|o| o.weight).sum();
        let composite = if total_weight > 0.0 {
            outputs.iter().map(|o| o.contribution()).sum::<f64>() / total_weight
        } else {
            0.0
        };

        (outputs, composite.clamp(-1.0, 1.0))
    }

    // ── Individual factor evaluations ──

    /// Momentum factor: maps [-1.0, +1.0] momentum to a signal value.
    /// Threshold at ±0.2 for minimum signal.
    fn eval_momentum(&self, fast_momentum: f64) -> FactorOutput {
        let value = if fast_momentum.abs() < 0.2 {
            0.0
        } else {
            fast_momentum.clamp(-1.0, 1.0)
        };
        let confidence = fast_momentum.abs().min(1.0);
        FactorOutput {
            name: "momentum".into(),
            value,
            confidence,
            weight: self.momentum.weight,
        }
    }

    /// Volume profile factor: volume acceleration -> signal.
    /// Accelerating volume (recent spike) amplifies momentum signal.
    fn eval_volume_profile(&self, state: &MintMarketState) -> FactorOutput {
        let accel = state.volume_acceleration();
        // accel > 1.0 = volume accelerating (recent spike)
        // accel < 1.0 = volume decelerating
        // Map: accel of 2.0 -> signal boost of 0.5, accel of 4.0 -> 0.75
        let value = if accel > 1.5 {
            0.5 * (1.0 - (-(accel - 1.5)).exp()).min(1.0)
        } else if accel < 0.5 {
            -0.3 // decelerating volume is a mild negative signal
        } else {
            0.0
        };
        let confidence = (state.event_rate_1s() / 20.0).min(1.0);
        FactorOutput {
            name: "volume_profile".into(),
            value,
            confidence,
            weight: self.volume_profile.weight,
        }
    }

    /// Spread factor: wider spread = weaker signal (harder to enter profitably).
    /// Spread in bps. > 50 bps = no signal, < 10 bps = full signal.
    fn eval_spread(&self, spread_bps: f64) -> FactorOutput {
        let value = if spread_bps < 10.0 {
            1.0 // Tight spread: full signal pass-through
        } else if spread_bps > 50.0 {
            0.0 // Wide spread: no signal
        } else {
            // Linear interpolation between 10 and 50 bps
            1.0 - (spread_bps - 10.0) / 40.0
        };
        let confidence = if spread_bps < 20.0 { 1.0 } else { (50.0 - spread_bps) / 30.0 };
        FactorOutput {
            name: "spread".into(),
            value,
            confidence: confidence.max(0.0),
            weight: self.spread.weight,
        }
    }

    /// Slot freshness factor: older data = less confidence.
    fn eval_slot_freshness(&self, state: &MintMarketState, now_micros: i64) -> FactorOutput {
        let age_micros = state.age_micros(now_micros);
        let value = if age_micros > 10_000_000 {
            0.0 // > 10s old: no signal
        } else if age_micros < 1_000_000 {
            1.0 // < 1s old: full signal
        } else {
            // Linear decay between 1s and 10s
            1.0 - (age_micros - 1_000_000) as f64 / 9_000_000.0
        };
        let confidence = value;
        FactorOutput {
            name: "slot_freshness".into(),
            value,
            confidence,
            weight: self.slot_freshness.weight,
        }
    }

    /// Protocol confidence factor: some protocols have more reliable data.
    fn eval_protocol_confidence(&self, protocol: &str) -> FactorOutput {
        // Confidence varies by protocol based on known data quality
        let value = match protocol {
            "pumpfun" => 0.8,     // Well-understood program
            "pumpswap" => 0.7,
            "raydium_amm_v4" => 0.9,  // Mature, heavily tested
            "raydium_cpmm" => 0.85,
            "meteora_damm_v2" => 0.7,
            "bonk" => 0.5,
            _ => 0.5, // Unknown protocol — neutral
        };
        FactorOutput {
            name: "protocol_confidence".into(),
            value,
            confidence: 1.0,
            weight: self.protocol_confidence.weight,
        }
    }

    /// Momentum divergence: fast - slow momentum.
    /// Positive = accelerating buys, Negative = accelerating sells.
    fn eval_momentum_divergence(&self, divergence: f64) -> FactorOutput {
        let value = divergence.clamp(-1.0, 1.0);
        let confidence = divergence.abs().min(1.0);
        FactorOutput {
            name: "momentum_divergence".into(),
            value,
            confidence,
            weight: self.momentum_divergence.weight,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::strategy::market_state::MintMarketState;

    #[test]
    fn test_momentum_factor_positive() {
        let factors = SignalFactors::default();
        let mut state = MintMarketState::new("test");
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // Record mostly buy events to create positive momentum
        state.momentum.record(now - 100_000, true, 1.0);
        state.momentum.record(now - 200_000, true, 1.0);
        state.momentum.record(now - 300_000, true, 1.0);
        state.momentum.record(now - 400_000, false, 1.0);

        let output = factors.eval_momentum(state.momentum.fast_momentum(now));
        assert!(output.value > 0.0, "Momentum should be positive: {}", output.value);
        assert!(output.confidence > 0.0);
    }

    #[test]
    fn test_spread_factor_wide() {
        let factors = SignalFactors::default();
        let output = factors.eval_spread(100.0); // very wide spread
        assert!((output.value).abs() < f64::EPSILON, "Wide spread should zero signal");
        assert!((output.confidence).abs() < f64::EPSILON);
    }

    #[test]
    fn test_spread_factor_tight() {
        let factors = SignalFactors::default();
        let output = factors.eval_spread(5.0); // tight spread
        assert!((output.value - 1.0).abs() < 0.01, "Tight spread should pass full signal");
    }

    #[test]
    fn test_slot_freshness_stale() {
        let factors = SignalFactors::default();
        let mut state = MintMarketState::new("test");
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // Simulate old data
        state.last_update_micros = now - 15_000_000; // 15s old

        let output = factors.eval_slot_freshness(&state, now);
        assert!((output.value).abs() < f64::EPSILON, "Stale data should zero signal");
    }

    #[test]
    fn test_protocol_confidence() {
        let factors = SignalFactors::default();
        let raydium = factors.eval_protocol_confidence("raydium_amm_v4");
        let unknown = factors.eval_protocol_confidence("unknown");
        assert!(raydium.value > unknown.value, "Raydium should have higher confidence than unknown");
    }

    #[test]
    fn test_composite_evaluation() {
        let factors = SignalFactors::default();
        let mut state = MintMarketState::new("test");
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // Create a strong buy scenario
        state.momentum.record(now - 100_000, true, 10.0);
        state.momentum.record(now - 200_000, true, 10.0);
        state.momentum.record(now - 300_000, true, 10.0);
        state.momentum.record(now - 400_000, false, 5.0);
        state.last_update_micros = now - 100_000;
        state.spread_bps = 8.0;

        let (outputs, composite) = factors.evaluate(&mut state, now, "raydium_amm_v4");

        assert!(!outputs.is_empty(), "Should have factor outputs");
        assert!(composite > 0.0, "Composite should be positive in bullish scenario: {composite}");

        // Verify factor breakdown has correct structure
        let momentum_out = outputs.iter().find(|o| o.name == "momentum");
        assert!(momentum_out.is_some());
        assert!(momentum_out.unwrap().contribution() > 0.0);
    }
}