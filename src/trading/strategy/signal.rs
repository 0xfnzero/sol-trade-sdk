//! Signal types for the Strategy Engine.
//!
//! Defines the output of signal evaluation — `TradeSignal` when the strategy
//! wants to trade, and `NoTrade` when it doesn't — plus the intermediate
//! `SignalStrength` enum for composite scoring.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// SignalStrength — the raw directional strength from factor aggregation
// ---------------------------------------------------------------------------

/// The directional strength of a composite signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SignalStrength {
    /// Strong buy — cross-the-spread confidence
    StrongBuy = 0,
    /// Buy — enter at touch price (FOK or GTC)
    Buy = 1,
    /// Neutral — no directional bias
    Neutral = 2,
    /// Sell — enter at touch price
    Sell = 3,
    /// Strong sell — cross-the-spread confidence
    StrongSell = 4,
}

impl SignalStrength {
    /// Returns the sign of the direction: +1 for buy, -1 for sell, 0 for neutral.
    pub fn direction_sign(self) -> i8 {
        match self {
            SignalStrength::StrongBuy => 1,
            SignalStrength::Buy => 1,
            SignalStrength::Neutral => 0,
            SignalStrength::Sell => -1,
            SignalStrength::StrongSell => -1,
        }
    }

    /// Returns `true` if this signal is directional (buy or sell).
    pub fn is_directional(self) -> bool {
        !matches!(self, SignalStrength::Neutral)
    }

    /// Returns the numeric strength (0.0 = neutral, 1.0 = strong).
    pub fn magnitude(self) -> f64 {
        match self {
            SignalStrength::StrongBuy => 1.0,
            SignalStrength::Buy => 0.5,
            SignalStrength::Neutral => 0.0,
            SignalStrength::Sell => 0.5,
            SignalStrength::StrongSell => 1.0,
        }
    }
}

impl fmt::Display for SignalStrength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignalStrength::StrongBuy => write!(f, "STRONG_BUY"),
            SignalStrength::Buy => write!(f, "BUY"),
            SignalStrength::Neutral => write!(f, "NEUTRAL"),
            SignalStrength::Sell => write!(f, "SELL"),
            SignalStrength::StrongSell => write!(f, "STRONG_SELL"),
        }
    }
}

// ---------------------------------------------------------------------------
// TradeSignal — the output when the strategy wants to trade
// ---------------------------------------------------------------------------

/// A validated trade signal produced by the strategy engine.
///
/// Contains everything needed to construct a `TradePlan` or `ShadowDecision`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeSignal {
    /// The mint/token address being traded.
    pub mint: String,
    /// Directional signal.
    pub strength: SignalStrength,
    /// Signal confidence in [0.0, 1.0].
    pub confidence: f64,
    /// The protocol on which to execute (e.g. "pumpfun", "raydium_amm_v4").
    pub protocol: String,
    /// Current estimated spread in basis points.
    pub spread_bps: f64,
    /// Estimated midpoint price at signal time.
    pub midpoint_price: f64,
    /// Solana slot at signal time.
    pub slot: u64,
    /// Composite score from factor aggregation.
    pub composite_score: f64,
    /// Individual factor contributions for audit/debug.
    pub factor_breakdown: Vec<FactorContribution>,
    /// Human-readable reason for the signal.
    pub reason: String,
}

/// A single factor's contribution to the composite signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactorContribution {
    /// Factor name (e.g. "momentum", "volume_profile").
    pub name: String,
    /// Raw factor value.
    pub value: f64,
    /// Assigned weight for this factor.
    pub weight: f64,
    /// Weighted contribution (value * weight).
    pub contribution: f64,
}

impl fmt::Display for TradeSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TradeSignal({} {}@{:.2} spread={:.1}bp conf={:.2} score={:.2})",
            self.strength, self.mint, self.midpoint_price, self.spread_bps, self.confidence, self.composite_score
        )
    }
}

// ---------------------------------------------------------------------------
// NoTrade — the output when the strategy declines to trade
// ---------------------------------------------------------------------------

/// The strategy evaluated the market and decided NOT to trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoTrade {
    /// Human-readable reason for the rejection.
    pub reason: String,
    /// Composite score (may be below threshold).
    pub composite_score: f64,
    /// Individual factor contributions.
    pub factor_breakdown: Vec<FactorContribution>,
    /// Which gate rejected the signal (if any).
    pub rejected_by: Option<String>,
}

impl fmt::Display for NoTrade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(gate) = &self.rejected_by {
            write!(f, "NoTrade(rejected_by={}, reason={}, score={:.2})", gate, self.reason, self.composite_score)
        } else {
            write!(f, "NoTrade(reason={}, score={:.2})", self.reason, self.composite_score)
        }
    }
}

// ---------------------------------------------------------------------------
// StrategyOutcome — the unified output of strategy evaluation
// ---------------------------------------------------------------------------

/// The outcome of a single strategy evaluation cycle.
#[derive(Debug, Clone)]
pub enum StrategyOutcome {
    /// The strategy wants to trade.
    Trade(TradeSignal),
    /// The strategy declined to trade.
    NoTrade(NoTrade),
}

impl StrategyOutcome {
    /// Returns `true` if this is a trade signal.
    pub fn is_trade(&self) -> bool {
        matches!(self, StrategyOutcome::Trade(_))
    }

    /// Returns the composite score regardless of outcome.
    pub fn composite_score(&self) -> f64 {
        match self {
            StrategyOutcome::Trade(s) => s.composite_score,
            StrategyOutcome::NoTrade(n) => n.composite_score,
        }
    }
}

// ---------------------------------------------------------------------------
// TradeDirection (used internally — maps SignalStrength → TradeDirection)
// ---------------------------------------------------------------------------

/// Direction mapping from signal strength.
pub fn signal_to_trade_direction(strength: SignalStrength) -> crate::trading::core::state::TradeDirection {
    match strength {
        SignalStrength::StrongBuy | SignalStrength::Buy => crate::trading::core::state::TradeDirection::Buy,
        SignalStrength::StrongSell | SignalStrength::Sell => crate::trading::core::state::TradeDirection::Sell,
        SignalStrength::Neutral => {
            // Should not happen — caller must check is_directional first
            crate::trading::core::state::TradeDirection::Buy
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_strength_direction() {
        assert_eq!(SignalStrength::StrongBuy.direction_sign(), 1);
        assert_eq!(SignalStrength::StrongSell.direction_sign(), -1);
        assert_eq!(SignalStrength::Neutral.direction_sign(), 0);
        assert!(SignalStrength::Buy.is_directional());
        assert!(!SignalStrength::Neutral.is_directional());
    }

    #[test]
    fn test_trade_signal_display() {
        let signal = TradeSignal {
            mint: "So11111111111111111111111111111111111111112".into(),
            strength: SignalStrength::Buy,
            confidence: 0.65,
            protocol: "pumpfun".into(),
            spread_bps: 12.5,
            midpoint_price: 0.001234,
            slot: 123456789,
            composite_score: 0.52,
            factor_breakdown: vec![],
            reason: "momentum threshold exceeded".into(),
        };
        let s = format!("{signal}");
        assert!(s.contains("BUY"));
        assert!(s.contains("0.00"));   // rounded to 2 decimal places
        assert!(s.contains("12.5"));
    }

    #[test]
    fn test_no_trade_rejection() {
        let nt = NoTrade {
            reason: "spread too wide".into(),
            composite_score: 0.15,
            factor_breakdown: vec![],
            rejected_by: Some("spread_gate".into()),
        };
        assert!(format!("{nt}").contains("spread_gate"));
    }

    #[test]
    fn test_strategy_outcome() {
        let trade_outcome = StrategyOutcome::Trade(TradeSignal {
            mint: "x".into(), strength: SignalStrength::Buy, confidence: 0.6,
            protocol: "pumpfun".into(), spread_bps: 10.0, midpoint_price: 1.0,
            slot: 42, composite_score: 0.6, factor_breakdown: vec![], reason: "test".into(),
        });
        assert!(trade_outcome.is_trade());

        let notrade_outcome = StrategyOutcome::NoTrade(NoTrade {
            reason: "no signal".into(), composite_score: 0.0,
            factor_breakdown: vec![], rejected_by: None,
        });
        assert!(!notrade_outcome.is_trade());
    }
}