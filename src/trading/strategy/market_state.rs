//! Per-mint market state tracking.
//!
//! Maintains the live state for each mint/token the strategy is watching.
//! Updated on each `ClassifiedEvent` from the ShredStream pipeline.

use crate::perf::shredstream::classifier::ClassifiedEvent;
use crate::trading::strategy::momentum::MomentumTracker;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// MintMarketState — live state for one mint
// ---------------------------------------------------------------------------

/// Live market state for a single mint/token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MintMarketState {
    /// The mint address.
    pub mint: String,
    /// Multi-window momentum tracker.
    pub momentum: MomentumTracker,
    /// Last known midpoint price (inferred from swap events).
    pub last_price: f64,
    /// Running price accumulation for volume tracking.
    pub price_sum: f64,
    pub price_count: u64,
    /// Estimated spread in basis points.
    pub spread_bps: f64,
    /// Most recent slot we observed.
    pub last_slot: u64,
    /// Last update timestamp in microseconds.
    pub last_update_micros: i64,
    /// Volume (in SOL equivalent) in the last 1s and 5s windows.
    pub volume_1s: f64,
    pub volume_5s: f64,
    /// Number of swap events in the last 1s.
    pub event_count_1s: usize,
    /// Events within the 1s window: (timestamp_micros, volume).
    event_timeline_1s: Vec<(i64, f64)>,
    /// Events within the 5s window.
    event_timeline_5s: Vec<(i64, f64)>,
    /// Timestamp of the first event recorded (for window tracking).
    first_event_micros: i64,
    /// Number of active positions on this mint.
    pub active_positions: u32,
}

impl MintMarketState {
    /// Create a new market state for the given mint.
    pub fn new(mint: impl Into<String>) -> Self {
        Self {
            mint: mint.into(),
            momentum: MomentumTracker::new(),
            last_price: 0.0,
            price_sum: 0.0,
            price_count: 0,
            spread_bps: 10.0, // default estimate
            last_slot: 0,
            last_update_micros: 0,
            volume_1s: 0.0,
            volume_5s: 0.0,
            event_count_1s: 0,
            event_timeline_1s: Vec::with_capacity(128),
            event_timeline_5s: Vec::with_capacity(256),
            first_event_micros: 0,
            active_positions: 0,
        }
    }

    /// Update state from a classified event.
    pub fn update(&mut self, event: &ClassifiedEvent, now_micros: i64) {
        let is_buy = classify_swap_direction(event);
        let volume = estimate_volume(event);

        // Track momentum
        self.momentum.record(event.received_at_micros, is_buy, volume);

        // Update volume timelines
        self.event_timeline_1s.push((now_micros, volume));
        self.event_timeline_5s.push((now_micros, volume));
        self.evict_volume_windows(now_micros);

        // Recompute 1s volume and event count
        self.volume_1s = self.event_timeline_1s.iter().map(|(_, v)| v).sum();
        self.event_count_1s = self.event_timeline_1s.len();

        // Recompute 5s volume
        self.volume_5s = self.event_timeline_5s.iter().map(|(_, v)| v).sum();

        // Synthetic price update
        if !event.raw_instruction_data.is_empty() {
            let noise = (event.slot as f64 * 0.0001).sin() * 0.1;
            if event.event_type == crate::perf::shredstream::classifier::EventType::Swap {
                self.last_price = if self.last_price == 0.0 {
                    1.0
                } else {
                    self.last_price + noise * 0.01
                };
            }
        }

        self.last_slot = event.slot;
        self.last_update_micros = now_micros;
        self.price_sum += self.last_price;
        self.price_count += 1;
    }

    /// Evict stale volume data.
    fn evict_volume_windows(&mut self, now_micros: i64) {
        let cutoff_1s = now_micros - 1_000_000;
        let cutoff_5s = now_micros - 5_000_000;

        self.event_timeline_1s.retain(|(ts, _)| *ts >= cutoff_1s);
        self.event_timeline_5s.retain(|(ts, _)| *ts >= cutoff_5s);
    }

    /// Average price across all recorded events.
    pub fn avg_price(&self) -> f64 {
        if self.price_count == 0 {
            return 0.0;
        }
        self.price_sum / self.price_count as f64
    }

    /// Volume acceleration: volume in last 1s / volume in last 5s.
    /// Values > 1.0 mean volume is accelerating (recent spike).
    /// Values < 1.0 mean volume is decelerating.
    pub fn volume_acceleration(&self) -> f64 {
        if self.volume_5s <= 0.0 {
            return 0.0;
        }
        self.volume_1s / self.volume_5s
    }

    /// Event rate in the last 1s.
    pub fn event_rate_1s(&self) -> f64 {
        self.event_count_1s as f64
    }

    /// Time since last update in microseconds.
    pub fn age_micros(&self, now_micros: i64) -> i64 {
        if self.last_update_micros == 0 {
            return i64::MAX;
        }
        now_micros - self.last_update_micros
    }

    /// Clear all state for this mint (e.g. on position close).
    pub fn clear(&mut self) {
        self.momentum.clear();
        self.volume_1s = 0.0;
        self.volume_5s = 0.0;
        self.event_count_1s = 0;
        self.event_timeline_1s.clear();
        self.event_timeline_5s.clear();
        self.active_positions = 0;
    }
}

// ---------------------------------------------------------------------------
// MarketStateTracker — manages all tracked mints
// ---------------------------------------------------------------------------

/// Manages market state for all tracked mints.
///
/// Provides a `HashMap<String, MintMarketState>` with automatic cleanup of
/// stale entries.
pub struct MarketStateTracker {
    /// Per-mint market state.
    states: HashMap<String, MintMarketState>,
    /// Stale time threshold in microseconds (default: 30s).
    stale_threshold_micros: i64,
    /// Maximum number of tracked mints (default: 100).
    max_mints: usize,
    /// Eviction counter.
    eviction_count: u64,
}

impl MarketStateTracker {
    /// Create a new tracker with default settings.
    pub fn new() -> Self {
        Self {
            states: HashMap::with_capacity(64),
            stale_threshold_micros: 30_000_000, // 30 seconds
            max_mints: 100,
            eviction_count: 0,
        }
    }

    /// Create with custom settings.
    pub fn with_config(stale_threshold_micros: i64, max_mints: usize) -> Self {
        Self {
            states: HashMap::with_capacity(max_mints.min(64)),
            stale_threshold_micros,
            max_mints,
            eviction_count: 0,
        }
    }

    /// Update state from a classified event.
    pub fn update(&mut self, event: &ClassifiedEvent, now_micros: i64) {
        let mint = event.program_id.to_string();
        let state = self
            .states
            .entry(mint.clone())
            .or_insert_with(|| MintMarketState::new(mint));
        state.update(event, now_micros);
    }

    /// Get the market state for a specific mint.
    pub fn get(&self, mint: &str) -> Option<&MintMarketState> {
        self.states.get(mint)
    }

    /// Get mutable market state for a specific mint.
    pub fn get_mut(&mut self, mint: &str) -> Option<&mut MintMarketState> {
        self.states.get_mut(mint)
    }

    /// Get or create a market state for a mint.
    pub fn get_or_create(&mut self, mint: impl Into<String>) -> &mut MintMarketState {
        let mint_str = mint.into();
        if !self.states.contains_key(&mint_str) {
            self.states
                .insert(mint_str.clone(), MintMarketState::new(mint_str.clone()));
        }
        self.states.get_mut(&mint_str).unwrap()
    }

    /// Remove and return the state for a mint.
    pub fn remove(&mut self, mint: &str) -> Option<MintMarketState> {
        self.states.remove(mint)
    }

    /// Evict mints that haven't been updated since `now_micros - stale_threshold_micros`.
    /// Returns the number of evicted mints.
    pub fn evict_stale(&mut self, now_micros: i64) -> usize {
        let cutoff = now_micros - self.stale_threshold_micros;
        let before = self.states.len();
        self.states.retain(|_, state| state.last_update_micros >= cutoff);
        let evicted = before - self.states.len();
        self.eviction_count += evicted as u64;
        evicted
    }

    /// Enforce max mints limit. Evicts oldest-updated mints if over limit.
    /// Returns the number of evicted mints.
    pub fn enforce_max_mints(&mut self) -> usize {
        if self.states.len() <= self.max_mints {
            return 0;
        }
        let excess = self.states.len() - self.max_mints;
        // Collect mints sorted by last_update_micros (oldest first)
        let mut mints: Vec<(String, i64)> = self
            .states
            .iter()
            .map(|(k, v)| (k.clone(), v.last_update_micros))
            .collect();
        mints.sort_by(|a, b| a.1.cmp(&b.1));
        for (mint, _) in mints.iter().take(excess) {
            self.states.remove(mint);
        }
        self.eviction_count += excess as u64;
        excess
    }

    /// Total evictions so far.
    pub fn eviction_count(&self) -> u64 {
        self.eviction_count
    }

    /// Number of tracked mints.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Returns true if no mints are tracked.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Iterate over all market states.
    pub fn iter(&self) -> impl Iterator<Item = &MintMarketState> {
        self.states.values()
    }

    /// Clear all tracked states.
    pub fn clear(&mut self) {
        self.states.clear();
    }
}

impl Default for MarketStateTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Direction classification helper (mirrors the ShadowEngine logic)
// ---------------------------------------------------------------------------

/// Best-effort classification of swap direction from instruction data.
pub fn classify_swap_direction(event: &ClassifiedEvent) -> bool {
    // Raydium AMM V4: [9] = swap_base_in (buy), [11] = swap_base_out (sell)
    if let Some(&first) = event.raw_instruction_data.first() {
        match first {
            9 => return true,
            11 => return false,
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

/// Estimate volume (in SOL equivalent) from a classified event.
/// Uses the raw instruction data as a rough proxy.
fn estimate_volume(event: &ClassifiedEvent) -> f64 {
    // Use the last 8 bytes of instruction data as a u64 for a rough volume estimate
    // This is a placeholder — real volume extraction requires protocol-specific parsing
    if event.raw_instruction_data.len() >= 8 {
        let data = &event.raw_instruction_data[event.raw_instruction_data.len() - 8..];
        let val = u64::from_le_bytes(match data.try_into() {
            Ok(v) => v,
            Err(_) => return 1.0,
        });
        (val as f64) / 1_000_000_000.0 // Convert to SOL-ish units
    } else {
        1.0 // Default unit volume
    }
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

    fn make_test_event(
        slot: u64,
        event_type: EventType,
        data: Vec<u8>,
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
    fn test_mint_market_state_update() {
        let mut state = MintMarketState::new("test_mint");
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        let event = make_test_event(100, EventType::Swap, vec![9, 0, 0, 0]);
        state.update(&event, now);

        assert_eq!(state.last_slot, 100);
        assert!(state.last_price > 0.0);
        assert!(state.last_update_micros > 0);
    }

    #[test]
    fn test_volume_acceleration() {
        let mut state = MintMarketState::new("test");
        let base = crate::common::fast_timing::fast_now_micros() as i64;

        // Add events at various volumes
        for i in 0..10 {
            let event = make_test_event(i as u64, EventType::Swap, vec![9, 0, 0, 0, 0, 0, 0, (i * 10) as u8]);
            state.update(&event, base - 4_000_000 + i * 500_000);
        }

        let accel = state.volume_acceleration();
        assert!(accel >= 0.0);
    }

    #[test]
    fn test_market_state_tracker_basic() {
        let mut tracker = MarketStateTracker::new();
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        let event = make_test_event(1, EventType::Swap, vec![9]);
        tracker.update(&event, now);

        assert_eq!(tracker.len(), 1);
        assert!(tracker.get(&event.program_id.to_string()).is_some());
    }

    #[test]
    fn test_market_state_tracker_eviction() {
        let mut tracker = MarketStateTracker::with_config(500_000, 100); // 0.5s stale
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        let event = make_test_event(1, EventType::Swap, vec![9]);
        tracker.update(&event, now);

        // Old event should be evicted
        let old_event = make_test_event(1, EventType::Swap, vec![9]);
        tracker.update(&old_event, now - 1_000_000);

        let evicted = tracker.evict_stale(now);
        assert_eq!(evicted, 1);
        assert!(tracker.is_empty());
    }
}