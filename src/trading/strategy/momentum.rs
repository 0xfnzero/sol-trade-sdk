//! Momentum tracker — sliding-window buy/sell ratio.
//!
//! Tracks the ratio of buy vs sell swap events over a configurable time window.
//! Extracted from the original `ShadowEngine` implementation and enhanced with:
//!
//! - Multiple window sizes (fast, medium, slow)
//! - Per-mint tracking
//! - Configurable eviction
//! - Volume-weighted momentum

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// SlidingWindowMomentum — single-window momentum tracker
// ---------------------------------------------------------------------------

/// Tracks buy/sell swap event momentum over a single sliding time window.
///
/// Momentum is computed as `(buys / total) * 2 - 1`, yielding a value in
/// `[-1.0, +1.0]`:
///   - `+1.0` = all buys
///   - `-1.0` = all sells
///   - `0.0` = equal or no data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlidingWindowMomentum {
    /// Window duration in microseconds.
    window_micros: i64,
    /// Events: (timestamp_micros, is_buy, volume).
    events: Vec<(i64, bool, f64)>,
}

impl SlidingWindowMomentum {
    /// Create a new tracker with the given window duration (in microseconds).
    /// e.g. `2_000_000` for a 2-second window.
    pub fn new(window_micros: i64) -> Self {
        Self {
            window_micros,
            events: Vec::with_capacity(256),
        }
    }

    /// Record a swap event.
    pub fn record(&mut self, timestamp_micros: i64, is_buy: bool, volume: f64) {
        self.events.push((timestamp_micros, is_buy, volume));
        self.evict_old(timestamp_micros);
    }

    /// Remove events outside the window.
    fn evict_old(&mut self, now: i64) {
        let cutoff = now - self.window_micros;
        self.events.retain(|(ts, _, _)| *ts >= cutoff);
    }

    /// Compute the momentum value. Also evicts stale events.
    pub fn momentum(&mut self, now: i64) -> f64 {
        self.evict_old(now);
        if self.events.is_empty() {
            return 0.0;
        }
        let buys = self.events.iter().filter(|(_, is_buy, _)| *is_buy).count();
        let total = self.events.len();
        (buys as f64 / total as f64) * 2.0 - 1.0
    }

    /// Volume-weighted momentum: each event is weighted by its size.
    pub fn volume_weighted_momentum(&mut self, now: i64) -> f64 {
        self.evict_old(now);
        if self.events.is_empty() {
            return 0.0;
        }
        let total_vol: f64 = self.events.iter().map(|(_, _, v)| v).sum();
        if total_vol <= 0.0 {
            return self.momentum(now);
        }
        let buy_vol: f64 = self
            .events
            .iter()
            .filter(|(_, is_buy, _)| *is_buy)
            .map(|(_, _, v)| v)
            .sum();
        (buy_vol / total_vol) * 2.0 - 1.0
    }

    /// Total number of events in the current window.
    pub fn total_events(&self) -> usize {
        self.events.len()
    }

    /// Clear all events.
    pub fn clear(&mut self) {
        self.events.clear();
    }
}

// ---------------------------------------------------------------------------
// MomentumTracker — multi-window momentum with configurable windows
// ---------------------------------------------------------------------------

/// Multi-window momentum tracker. Maintains three window sizes:
///
/// | Window   | Duration | Use Case                         |
/// |----------|----------|----------------------------------|
/// | Fast     | 2s       | Immediate momentum burst detect  |
/// | Medium   | 5s       | Short-term trend confirmation    |
/// | Slow     | 10s      | Medium-term trend filter         |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MomentumTracker {
    /// Fast window (default: 2 seconds).
    pub fast: SlidingWindowMomentum,
    /// Medium window (default: 5 seconds).
    pub medium: SlidingWindowMomentum,
    /// Slow window (default: 10 seconds).
    pub slow: SlidingWindowMomentum,
    /// Window sizes in microseconds.
    pub fast_window_micros: i64,
    pub medium_window_micros: i64,
    pub slow_window_micros: i64,
}

impl MomentumTracker {
    /// Create a new multi-window momentum tracker with default window sizes.
    /// Fast=2s, Medium=5s, Slow=10s.
    pub fn new() -> Self {
        Self::with_windows(2_000_000, 5_000_000, 10_000_000)
    }

    /// Create with custom window sizes (in microseconds).
    pub fn with_windows(fast_micros: i64, medium_micros: i64, slow_micros: i64) -> Self {
        Self {
            fast: SlidingWindowMomentum::new(fast_micros),
            medium: SlidingWindowMomentum::new(medium_micros),
            slow: SlidingWindowMomentum::new(slow_micros),
            fast_window_micros: fast_micros,
            medium_window_micros: medium_micros,
            slow_window_micros: slow_micros,
        }
    }

    /// Record a swap event across all windows.
    pub fn record(&mut self, timestamp_micros: i64, is_buy: bool, volume: f64) {
        self.fast.record(timestamp_micros, is_buy, volume);
        self.medium.record(timestamp_micros, is_buy, volume);
        self.slow.record(timestamp_micros, is_buy, volume);
    }

    /// Get fast momentum (2s window).
    pub fn fast_momentum(&mut self, now: i64) -> f64 {
        self.fast.momentum(now)
    }

    /// Get medium momentum (5s window).
    pub fn medium_momentum(&mut self, now: i64) -> f64 {
        self.medium.momentum(now)
    }

    /// Get slow momentum (10s window).
    pub fn slow_momentum(&mut self, now: i64) -> f64 {
        self.slow.momentum(now)
    }

    /// Get all momentums as a tuple.
    pub fn all_momentums(&mut self, now: i64) -> (f64, f64, f64) {
        (self.fast_momentum(now), self.medium_momentum(now), self.slow_momentum(now))
    }

    /// Get fast volume-weighted momentum.
    pub fn fast_vw_momentum(&mut self, now: i64) -> f64 {
        self.fast.volume_weighted_momentum(now)
    }

    /// Compute momentum divergence: fast - slow. Positive means
    /// momentum is accelerating (buying pressure increasing).
    pub fn divergence(&mut self, now: i64) -> f64 {
        self.fast_momentum(now) - self.slow_momentum(now)
    }

    /// Clear all windows.
    pub fn clear(&mut self) {
        self.fast.clear();
        self.medium.clear();
        self.slow.clear();
    }
}

impl Default for MomentumTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sliding_window_basic() {
        let mut sw = SlidingWindowMomentum::new(1_000_000); // 1s window
        let now = 1_000_000;

        // 3 buys, 1 sell
        sw.record(now - 500_000, true, 1.0);
        sw.record(now - 400_000, false, 1.0);
        sw.record(now - 300_000, true, 1.0);
        sw.record(now - 200_000, true, 1.0);

        let m = sw.momentum(now);
        assert!((m - 0.5).abs() < 0.01, "Expected ~0.5, got {m}");

        // Event outside window should be evicted
        sw.record(now - 2_000_000, false, 1.0);
        let m2 = sw.momentum(now);
        assert!((m2 - 0.5).abs() < 0.01, "Old events should be evicted, got {m2}");
    }

    #[test]
    fn test_multi_window_tracker() {
        let mut mt = MomentumTracker::new();
        let now = 1_000_000;

        // Record 4 buys in the last 1s
        mt.record(now - 800_000, true, 1.0);
        mt.record(now - 600_000, true, 1.0);
        mt.record(now - 400_000, false, 1.0);
        mt.record(now - 200_000, true, 1.0);

        let (fast, medium, slow) = mt.all_momentums(now);
        assert!((fast - 0.5).abs() < 0.01, "Fast momentum ~0.5, got {fast}");
        assert!((medium - 0.5).abs() < 0.01, "Medium momentum ~0.5, got {medium}");
        assert!((slow - 0.5).abs() < 0.01, "Slow momentum ~0.5, got {slow}");

        // Divergence should be ~0 since all windows have the same data
        let div = mt.divergence(now);
        assert!((div).abs() < 0.1, "Divergence ~0, got {div}");
    }

    #[test]
    fn test_volume_weighted() {
        let mut sw = SlidingWindowMomentum::new(1_000_000);
        let now = 1_000_000;

        // 1 sell with large volume, 5 buys with tiny volume
        sw.record(now - 500_000, false, 100.0);  // big sell
        sw.record(now - 400_000, true, 1.0);
        sw.record(now - 300_000, true, 1.0);
        sw.record(now - 200_000, true, 1.0);
        sw.record(now - 100_000, true, 1.0);
        sw.record(now - 50_000, true, 1.0);

        let raw = sw.momentum(now);
        let vw = sw.volume_weighted_momentum(now);

        // Raw momentum is buy-heavy (0.67), but volume-weighted is sell-heavy
        assert!(raw > 0.0, "Raw momentum should be positive, got {raw}");
        assert!(vw < 0.0, "Volume-weighted momentum should be negative, got {vw}");
    }

    #[test]
    fn test_empty_window() {
        let mut sw = SlidingWindowMomentum::new(1_000_000);
        assert!((sw.momentum(1_000_000)).abs() < f64::EPSILON);
        assert!((sw.volume_weighted_momentum(1_000_000)).abs() < f64::EPSILON);
    }
}