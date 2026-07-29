//! Shadow decision recording — bounded ring buffer of hypothetical trade decisions.
//!
//! Each [`ShadowDecision`] records what the signal engine WOULD have done at a
//! given moment: direction, confidence, spread, midpoint price, and the reason
//! for the decision. No real transactions are submitted.
//!
//! The [`DecisionBuffer`] holds the last N decisions in a thread-safe ring buffer
//! (Mutex-protected VecDeque). API consumers (HTTP `/shadow/decisions`) read
//! the most recent slice.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// ShadowSignal — what direction would have been taken
// ---------------------------------------------------------------------------

/// The hypothetical trade direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ShadowSignal {
    /// The strategy would have placed a BUY order.
    Buy = 0,
    /// The strategy would have placed a SELL order.
    Sell = 1,
    /// The strategy decided NOT to trade (NoTrade).
    Hold = 2,
}

impl ShadowSignal {
    /// Returns `true` if this signal is directional (Buy or Sell).
    pub fn is_directional(self) -> bool {
        matches!(self, ShadowSignal::Buy | ShadowSignal::Sell)
    }

    /// Returns `+1` for Buy, `-1` for Sell, `0` for Hold.
    pub fn direction_sign(self) -> i8 {
        match self {
            ShadowSignal::Buy => 1,
            ShadowSignal::Sell => -1,
            ShadowSignal::Hold => 0,
        }
    }
}

impl std::fmt::Display for ShadowSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShadowSignal::Buy => write!(f, "BUY"),
            ShadowSignal::Sell => write!(f, "SELL"),
            ShadowSignal::Hold => write!(f, "HOLD"),
        }
    }
}

// ---------------------------------------------------------------------------
// ShadowDecision — one hypothetical trade record
// ---------------------------------------------------------------------------

/// A single shadow-mode decision — what the strategy WOULD have done.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowDecision {
    /// Monotonically increasing decision ID (global across the session).
    pub id: u64,
    /// Monotonic clock micros when the decision was made.
    pub timestamp_micros: i64,
    /// Solana slot at decision time.
    pub slot: u64,
    /// Hypothetical signal direction.
    pub signal: ShadowSignal,
    /// Signal confidence (0.0–1.0).
    pub confidence: f64,
    /// The spread in cents (midpoint-based) at decision time.
    pub spread_cents: f64,
    /// The midpoint price at decision time.
    pub midpoint_price: f64,
    /// How long the evaluation took (microseconds).
    pub evaluation_latency_micros: u64,
    /// Human-readable reason for the decision.
    pub reason: String,
}

impl ShadowDecision {
    /// Create a new shadow decision at the current time.
    pub fn new(
        slot: u64,
        signal: ShadowSignal,
        confidence: f64,
        spread_cents: f64,
        midpoint_price: f64,
        evaluation_latency_micros: u64,
        reason: String,
    ) -> Self {
        Self {
            id: 0, // assigned by DecisionBuffer
            timestamp_micros: crate::common::fast_timing::fast_now_micros() as i64,
            slot,
            signal,
            confidence,
            spread_cents,
            midpoint_price,
            evaluation_latency_micros,
            reason,
        }
    }
}

// ---------------------------------------------------------------------------
// DecisionBuffer — bounded ring buffer (thread-safe)
// ---------------------------------------------------------------------------

/// Bounded ring buffer of shadow decisions, protected by a Mutex.
///
/// Default capacity: 10,000 decisions. Oldest entries are dropped when the
/// buffer is full.
pub struct DecisionBuffer {
    inner: Mutex<Inner>,
}

struct Inner {
    buffer: VecDeque<ShadowDecision>,
    capacity: usize,
    next_id: u64,
}

impl DecisionBuffer {
    /// Create a new buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                buffer: VecDeque::with_capacity(capacity),
                capacity,
                next_id: 0,
            }),
        }
    }

    /// Push a new decision into the buffer. Assigns the next ID.
    pub fn push(&self, mut decision: ShadowDecision) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        decision.id = inner.next_id;
        inner.next_id += 1;
        let id = decision.id;
        if inner.buffer.len() >= inner.capacity {
            inner.buffer.pop_front();
        }
        inner.buffer.push_back(decision);
        id
    }

    /// Return the `n` most recent decisions (reversed: newest first).
    pub fn recent(&self, n: usize) -> Vec<ShadowDecision> {
        let inner = self.inner.lock().unwrap();
        inner.buffer.iter().rev().take(n).cloned().collect()
    }

    /// Return all decisions with IDs >= `from_id`.
    pub fn since(&self, from_id: u64) -> Vec<ShadowDecision> {
        let inner = self.inner.lock().unwrap();
        inner
            .buffer
            .iter()
            .rev()
            .take_while(|d| d.id >= from_id)
            .cloned()
            .collect()
    }

    /// Current buffer occupancy.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().buffer.len()
    }

    /// Total decisions ever pushed.
    pub fn total_decisions(&self) -> u64 {
        self.inner.lock().unwrap().next_id
    }

    /// Clear all decisions.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.buffer.clear();
        inner.next_id = 0;
    }
}

impl Default for DecisionBuffer {
    fn default() -> Self {
        Self::new(10_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_buffer_push_and_recent() {
        let buf = DecisionBuffer::new(100);
        assert_eq!(buf.len(), 0);

        let d1 = ShadowDecision::new(100, ShadowSignal::Buy, 0.6, 1.0, 0.5, 5, "test".into());
        let id1 = buf.push(d1);
        assert_eq!(id1, 0);
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.total_decisions(), 1);

        let recent = buf.recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, 0);
    }

    #[test]
    fn test_decision_buffer_ring_behavior() {
        let buf = DecisionBuffer::new(5);
        for i in 0..10u64 {
            let d = ShadowDecision::new(
                i,
                ShadowSignal::Buy,
                0.5,
                1.0,
                0.5,
                1,
                format!("decision {i}"),
            );
            buf.push(d);
        }
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.total_decisions(), 10);

        let recent = buf.recent(10);
        assert_eq!(recent.len(), 5); // only the last 5 retained
        assert_eq!(recent[0].slot, 9); // newest first
        assert_eq!(recent[4].slot, 5); // oldest in the buffer
    }

    #[test]
    fn test_shadow_signal_methods() {
        assert!(ShadowSignal::Buy.is_directional());
        assert!(ShadowSignal::Sell.is_directional());
        assert!(!ShadowSignal::Hold.is_directional());
        assert_eq!(ShadowSignal::Buy.direction_sign(), 1);
        assert_eq!(ShadowSignal::Sell.direction_sign(), -1);
        assert_eq!(ShadowSignal::Hold.direction_sign(), 0);
    }
}