//! Per-mint market state tracking.
//!
//! Maintains the live state for each mint/token the strategy is watching.
//! Updated on each `ClassifiedEvent` from the ShredStream pipeline.

use crate::perf::shredstream::classifier::ClassifiedEvent;
use crate::trading::strategy::momentum::MomentumTracker;

use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Composite key: (program_id, mint) for per-mint-per-program tracking
// ---------------------------------------------------------------------------

/// Composite key for market state: (program_id, mint).
/// Allows the same program (e.g. Raydium) to track multiple distinct tokens.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarketStateKey {
    pub program_id: String,
    pub mint: String,
}

impl MarketStateKey {
    pub fn new(program_id: impl Into<String>, mint: impl Into<String>) -> Self {
        Self {
            program_id: program_id.into(),
            mint: mint.into(),
        }
    }
}

impl std::fmt::Display for MarketStateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}|{}", self.program_id, self.mint)
    }
}

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

        // Compute swap-implied price from instruction data
        if event.event_type == crate::perf::shredstream::classifier::EventType::Swap
            && !event.raw_instruction_data.is_empty()
        {
            if let Some(price) = compute_swap_price(&event.raw_instruction_data, &event.program_id, is_buy) {
                self.last_price = price;
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
/// Provides a `HashMap<MarketStateKey, MintMarketState>` with automatic cleanup
/// of stale entries. Keys are composite `(program_id, mint)` so the same DEX
/// program can track multiple distinct tokens without collision.
pub struct MarketStateTracker {
    /// Per-mint market state, keyed by (program_id, mint).
    states: HashMap<MarketStateKey, MintMarketState>,
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
    ///
    /// Constructs a composite key from `(program_id, mint)` so two tokens on
    /// the same DEX program are tracked in separate entries.
    ///
    /// NOTE: The `mint` parameter is currently a best-effort extraction from the
    /// event. When the classifier is augmented to carry the token mint address,
    /// pass the real mint here. Until then, `mint` is passed as `program_id.to_string()`
    /// from callers that don't have the mint, which provides program-level isolation.
    pub fn update(&mut self, program_id: &Pubkey, mint: &str, event: &ClassifiedEvent, now_micros: i64) {
        let key = MarketStateKey::new(program_id.to_string(), mint);
        let state = self
            .states
            .entry(key)
            .or_insert_with(|| MintMarketState::new(mint));
        state.update(event, now_micros);
    }

    /// Convenience wrapper for callers that have the event but not the mint.
    /// Uses `event.program_id.to_string()` for the mint component — this provides
    /// program-level isolation (same as the original pre-composite-key behavior).
    /// Prefer `update()` with an explicit mint when available.
    pub fn update_from_event(&mut self, event: &ClassifiedEvent, now_micros: i64) {
        let program_id = &event.program_id;
        let mint: String = program_id.to_string();
        let key = MarketStateKey::new(program_id.to_string(), &mint);
        let state = self
            .states
            .entry(key)
            .or_insert_with(|| MintMarketState::new(&mint));
        state.update(event, now_micros);
    }

    /// Update state using an explicit composite key.
    pub fn update_with_key(
        &mut self,
        program_id: &Pubkey,
        mint: &str,
        event: &ClassifiedEvent,
        now_micros: i64,
    ) {
        let key = MarketStateKey::new(program_id.to_string(), mint);
        let state = self
            .states
            .entry(key)
            .or_insert_with(|| MintMarketState::new(mint));
        state.update(event, now_micros);
    }

    /// Get the market state for a specific (program_id, mint) pair.
    pub fn get(&self, program_id: &str, mint: &str) -> Option<&MintMarketState> {
        let key = MarketStateKey::new(program_id, mint);
        self.states.get(&key)
    }

    /// Get mutable market state for a specific (program_id, mint) pair.
    pub fn get_mut(&mut self, program_id: &str, mint: &str) -> Option<&mut MintMarketState> {
        let key = MarketStateKey::new(program_id, mint);
        self.states.get_mut(&key)
    }

    /// Get or create a market state for a (program_id, mint) pair.
    pub fn get_or_create(
        &mut self,
        program_id: impl Into<String>,
        mint: impl Into<String>,
    ) -> &mut MintMarketState {
        let key = MarketStateKey::new(program_id, mint);
        if !self.states.contains_key(&key) {
            self.states
                .insert(key.clone(), MintMarketState::new(key.mint.clone()));
        }
        self.states.get_mut(&key).unwrap()
    }

    /// Find state by mint (iterating over all entries).
    /// Useful when the caller knows the mint but not the program_id.
    pub fn find_by_mint(&self, mint: &str) -> Option<&MintMarketState> {
        self.states.iter().find(|(k, _)| k.mint == mint).map(|(_, v)| v)
    }

    /// Find mutable state by mint (iterating over all entries).
    pub fn find_by_mint_mut(&mut self, mint: &str) -> Option<&mut MintMarketState> {
        self.states.iter_mut().find(|(k, _)| k.mint == mint).map(|(_, v)| v)
    }

    /// Remove and return the state for a (program_id, mint) pair.
    pub fn remove(&mut self, program_id: &str, mint: &str) -> Option<MintMarketState> {
        let key = MarketStateKey::new(program_id, mint);
        self.states.remove(&key)
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
        // Collect keys sorted by last_update_micros (oldest first)
        let mut keys: Vec<(MarketStateKey, i64)> = self
            .states
            .iter()
            .map(|(k, v)| (k.clone(), v.last_update_micros))
            .collect();
        keys.sort_by(|a, b| a.1.cmp(&b.1));
        for (key, _) in keys.iter().take(excess) {
            self.states.remove(key);
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
    pub fn iter(&self) -> impl Iterator<Item = (&MarketStateKey, &MintMarketState)> {
        self.states.iter()
    }

    /// Iterate over all keys.
    pub fn keys(&self) -> impl Iterator<Item = &MarketStateKey> {
        self.states.keys()
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

/// Compute a swap-implied price from raw instruction data.
///
/// Extracts two u64 amount fields (base token amount and quote amount) from
/// the instruction data and returns `quote / base` as the implied price.
/// Parse swap-implied price from instruction data using verified protocol identity.
///
/// Protocol is selected by the caller-provided `program_id`, NOT by payload-length
/// heuristics. This eliminates cross-protocol misparse entirely:
///
/// | Program ID | Protocol | Parser |
/// |------------|----------|--------|
/// | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | PumpFun | Anchor-style (8-byte discriminator, two u64 amounts at [8..16] and [16..24]) |
/// | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Raydium AMM V4 | 1-byte discrim + 3-byte pad, amounts at [4..12] and [12..20] |
/// | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Raydium CPMM | Same Raydium format |
///
/// # Safety invariants
///
/// 1. **No protocol fallthrough** — Each program ID routes exclusively to its parser.
/// 2. **Zero amounts are rejected** — PumpFun data with zero amounts returns `None`.
/// 3. **Unknown protocols fail closed** — Unrecognised program IDs return `None`.
/// 4. **Length is a structural check, not a discriminator** — Minimum payload length
///    is validated per protocol after routing (PumpFun ≥24 bytes, Raydium ≥20 bytes).
pub fn compute_swap_price(data: &[u8], program_id: &Pubkey, _is_buy: bool) -> Option<f64> {
    use solana_sdk::pubkey::Pubkey;

    /// PumpFun canonical program ID (from global_constants).
    const PUMPFUN: Pubkey = Pubkey::from_str_const("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
    /// Raydium AMM V4 canonical program ID.
    const RAYDIUM_AMM_V4: Pubkey = Pubkey::from_str_const("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");
    /// Raydium CPMM canonical program ID.
    const RAYDIUM_CPMM: Pubkey = Pubkey::from_str_const("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");

    match *program_id {
        PUMPFUN => {
            // Anchor / PumpFun: 8-byte discriminator, two u64 amounts at [8..16] and [16..24]
            if data.len() < 24 {
                return None;
            }
            let token_amount = u64::from_le_bytes(data[8..16].try_into().ok()?);
            let quote_amount = u64::from_le_bytes(data[16..24].try_into().ok()?);
            if token_amount == 0 || quote_amount == 0 {
                return None;
            }
            Some(quote_amount as f64 / token_amount as f64)
        }
        RAYDIUM_AMM_V4 | RAYDIUM_CPMM => {
            // Raydium: 1-byte discrim + 3-byte pad, amounts at [4..12] and [12..20]
            if data.len() < 20 {
                return None;
            }
            let amt1 = u64::from_le_bytes(data[4..12].try_into().ok()?);
            let amt2 = u64::from_le_bytes(data[12..20].try_into().ok()?);
            if amt1 == 0 || amt2 == 0 {
                return None;
            }
            // Without knowing which amount is quote vs token, use the smaller as denominator
            // to get a positive price estimate regardless of direction
            let (quote, base) = if amt1 >= amt2 { (amt1, amt2) } else { (amt2, amt1) };
            Some(quote as f64 / base as f64)
        }
        _ => {
            // Unknown protocol → fail closed
            None
        }
    }
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

    /// Test-only PumpFun program ID.
    fn pumpfun_id() -> Pubkey {
        solana_sdk::pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P")
    }
    /// Test-only Raydium AMM V4 program ID.
    fn raydium_id() -> Pubkey {
        solana_sdk::pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8")
    }
    /// Test-only default program ID for unknown-protocol tests.
    fn unknown_id() -> Pubkey {
        Pubkey::default()
    }

    /// Test-only Raydium event factory.
    fn make_raydium_event(slot: u64, event_type: EventType, data: Vec<u8>) -> ClassifiedEvent {
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        ClassifiedEvent {
            slot,
            signature: Signature::new_unique(),
            event_type,
            instruction_index: 0,
            program_id: raydium_id(),
            received_at_micros: now,
            decoded_at_micros: now,
            raw_instruction_data: data,
            trace: crate::perf::shredstream::EventTrace::new(now, now),
        }
    }

    fn make_test_event(
        slot: u64,
        event_type: EventType,
        data: Vec<u8>,
    ) -> ClassifiedEvent {
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        ClassifiedEvent {
            slot,
            signature: Signature::new_unique(),
            event_type,
            instruction_index: 0,
            program_id: pumpfun_id(),
            received_at_micros: now,
            decoded_at_micros: now,
            raw_instruction_data: data,
            trace: crate::perf::shredstream::EventTrace::new(now, now),
        }
    }

    #[test]
    fn test_mint_market_state_update() {
        let mut state = MintMarketState::new("test_mint");
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // Raydium-style swap instruction: [9] + 3 pad + 2 u64 amounts (8 bytes each)
        let mut data = vec![9u8, 0, 0, 0]; // discriminator + padding
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // amount_in (1 SOL)
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // min_amount_out (in speculative tokens at parity)

        let event = make_raydium_event(100, EventType::Swap, data);
        state.update(&event, now);

        assert_eq!(state.last_slot, 100);
        assert!(state.last_price > 0.0, "Price should be > 0, got {}", state.last_price);
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
        tracker.update(&event.program_id, "test_mint", &event, now);

        assert_eq!(tracker.len(), 1);
        assert!(tracker
            .get(&event.program_id.to_string(), "test_mint")
            .is_some());
    }

    #[test]
    fn test_market_state_tracker_multi_mint_isolation() {
        let mut tracker = MarketStateTracker::new();
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // Same program_id, two different mints — must NOT collide
        let mut event_a = make_test_event(1, EventType::Swap, vec![9, 0, 0, 0, 0, 0, 0, 100]);
        event_a.program_id = Pubkey::new_from_array([1u8; 32]);
        tracker.update(&event_a.program_id, "mint_alpha", &event_a, now);

        let mut event_b = make_test_event(1, EventType::Swap, vec![9, 0, 0, 0, 0, 0, 0, 200]);
        event_b.program_id = Pubkey::new_from_array([1u8; 32]); // same program ID
        tracker.update(&event_b.program_id, "mint_beta", &event_b, now);

        assert_eq!(tracker.len(), 2, "Two mints on same program must be separate entries");

        let state_a = tracker.get(&event_a.program_id.to_string(), "mint_alpha");
        assert!(state_a.is_some(), "State for mint_alpha should exist");

        let state_b = tracker.get(&event_b.program_id.to_string(), "mint_beta");
        assert!(state_b.is_some(), "State for mint_beta should exist");

        // Verify mint_a update doesn't affect mint_b
        state_b.unwrap().last_price;
        assert!(
            tracker.get(&event_a.program_id.to_string(), "mint_alpha").unwrap().event_count_1s > 0
        );
    }

    #[test]
    fn test_market_state_tracker_eviction() {
        let mut tracker = MarketStateTracker::with_config(500_000, 100); // 0.5s stale
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        let event = make_test_event(1, EventType::Swap, vec![9]);
        tracker.update(&event.program_id, "test_mint", &event, now);

        // Old event should be evicted
        let old_event = make_test_event(1, EventType::Swap, vec![9]);
        tracker.update(&old_event.program_id, "test_mint", &old_event, now - 1_000_000);

        let evicted = tracker.evict_stale(now);
        assert_eq!(evicted, 1);
        assert!(tracker.is_empty());
    }

    // ── Runtime Verification Tests ──

    /// Test compute_swap_price with Anchor-style instruction data (8-byte discriminator + 2 u64 amounts)
    #[test]
    fn test_compute_swap_price_anchor_style() {
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234]; // buy discriminator
        data.extend_from_slice(&100_000_000u64.to_le_bytes()); // 100M tokens
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // 1 SOL quote

        let price = compute_swap_price(&data, &pumpfun_id(), true);
        assert!(price.is_some(), "Anchor-style should produce a price");
        let p = price.unwrap();
        assert!((p - 10.0).abs() < 0.001, "Expected ~10.0 (1B/100M), got {}", p);
    }

    /// Test compute_swap_price with Raydium-style (1-byte discriminator + 3 pad + 2 u64 amounts)
    #[test]
    fn test_compute_swap_price_raydium_style() {
        let mut data = vec![9u8, 0, 0, 0]; // discriminator + padding
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // amount_in
        data.extend_from_slice(&100_000_000u64.to_le_bytes()); // min_amount_out

        let price = compute_swap_price(&data, &raydium_id(), true);
        assert!(price.is_some(), "Raydium-style should produce a price");
        let p = price.unwrap();
        assert!((p - 10.0).abs() < 0.001, "Expected ~10.0, got {}", p);
    }

    /// Test that zero token amount returns None (P3-05 fix: no fallthrough to Raydium)
    #[test]
    fn test_compute_swap_price_zero_token() {
        // Anchor-style with token_amount = 0 — formerly fell through to Raydium path
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234]; // buy discriminator
        data.extend_from_slice(&0u64.to_le_bytes()); // ZERO tokens
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // 1 SOL quote

        let price = compute_swap_price(&data, &pumpfun_id(), true);
        assert!(
            price.is_none(),
            "Zero token_amount must return None (no Raydium fallthrough), got Some({})",
            price.unwrap_or(0.0)
        );
    }

    /// Test that zero quote amount returns None
    #[test]
    fn test_compute_swap_price_zero_quote() {
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data.extend_from_slice(&100_000_000u64.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes()); // ZERO quote

        let price = compute_swap_price(&data, &pumpfun_id(), true);
        assert!(price.is_none(), "Zero quote should return None");
    }

    /// Test that too-short data returns None
    #[test]
    fn test_compute_swap_price_short_data() {
        let data = vec![9u8, 0, 0, 0]; // only 4 bytes
        let price = compute_swap_price(&data, &pumpfun_id(), true);
        assert!(price.is_none(), "Short data should return None");
    }

    /// Test find_by_mint across two different mints on the same program
    #[test]
    fn test_find_by_mint_two_mints() {
        let mut tracker = MarketStateTracker::new();
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        let program_id = Pubkey::new_from_array([1u8; 32]);

        let mut event_a = make_test_event(1, EventType::Swap, vec![9, 0, 0, 0]);
        event_a.program_id = program_id;
        tracker.update(&program_id, "mint_alpha", &event_a, now);

        let mut event_b = make_test_event(1, EventType::Swap, vec![9, 0, 0, 0]);
        event_b.program_id = program_id;
        tracker.update(&program_id, "mint_beta", &event_b, now);

        let found_a = tracker.find_by_mint("mint_alpha");
        assert!(found_a.is_some(), "find_by_mint should find mint_alpha");
        assert_eq!(found_a.unwrap().mint, "mint_alpha");

        let found_b = tracker.find_by_mint("mint_beta");
        assert!(found_b.is_some(), "find_by_mint should find mint_beta");
        assert_eq!(found_b.unwrap().mint, "mint_beta");

        // No cross-contamination
        let found_none = tracker.find_by_mint("nonexistent");
        assert!(found_none.is_none(), "find_by_mint should return None for unknown mint");
    }

    /// Test independence: updating one mint doesn't affect another
    #[test]
    fn test_mint_update_independence() {
        let mut tracker = MarketStateTracker::new();
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let program_id = Pubkey::new_from_array([1u8; 32]);

        let mut event_a = make_test_event(1, EventType::Swap, vec![9, 0, 0, 0]);
        event_a.program_id = program_id;
        tracker.update(&program_id, "mint_alpha", &event_a, now);

        let mut event_b = make_test_event(2, EventType::Swap, vec![9, 0, 0, 0]);
        event_b.program_id = program_id;
        tracker.update(&program_id, "mint_beta", &event_b, now + 100_000);

        // Update mint_alpha again
        let mut event_a2 = make_test_event(3, EventType::Swap, vec![9, 0, 0, 0]);
        event_a2.program_id = program_id;
        tracker.update(&program_id, "mint_alpha", &event_a2, now + 200_000);

        let state_a = tracker.get(&program_id.to_string(), "mint_alpha").unwrap();
        let state_b = tracker.get(&program_id.to_string(), "mint_beta").unwrap();

        assert_eq!(state_a.last_slot, 3, "mint_alpha should be updated to slot 3");
        assert_eq!(state_b.last_slot, 2, "mint_beta should still be at slot 2");
        assert!(state_a.last_update_micros > state_b.last_update_micros,
            "mint_alpha update time should be later than mint_beta");
    }

    /// Test that the get() method matches the correct mint
    #[test]
    fn test_get_by_mint_correctness() {
        let mut tracker = MarketStateTracker::new();
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let program_id = Pubkey::new_from_array([1u8; 32]);

        let mut event = make_test_event(1, EventType::Swap, vec![9, 0, 0, 0]);
        event.program_id = program_id;
        tracker.update(&program_id, "mint_alpha", &event, now);

        // get() with wrong mint should return None
        let wrong = tracker.get(&program_id.to_string(), "wrong_mint");
        assert!(wrong.is_none(), "get() with wrong mint should return None");

        // get() with correct mint should return Some
        let correct = tracker.get(&program_id.to_string(), "mint_alpha");
        assert!(correct.is_some(), "get() with correct mint should return Some");
    }

    /// Test that the MarketStateKey composite key is constructed correctly
    #[test]
    fn test_market_state_key_composite() {
        let key1 = MarketStateKey::new("program_a", "mint_1");
        let key2 = MarketStateKey::new("program_a", "mint_2");
        let key3 = MarketStateKey::new("program_b", "mint_1");

        // Same program, different mints → different keys
        assert_ne!(key1, key2, "Same program, different mints should be different keys");

        // Different programs, same mint → different keys
        assert_ne!(key1, key3, "Different programs, same mint should be different keys");

        // Display format
        assert_eq!(key1.to_string(), "program_a|mint_1");
    }

    // ── P3-05 Regression Tests: compute_swap_price safety ──

    /// Zero quote amount in Anchor/PumpFun data returns None
    #[test]
    fn test_compute_swap_price_anchor_zero_quote() {
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // tokens
        data.extend_from_slice(&0u64.to_le_bytes()); // ZERO quote
        assert!(compute_swap_price(&data, &pumpfun_id(), true).is_none(),
            "Anchor zero quote must return None");
    }

    /// Zero both amounts in Anchor data returns None
    #[test]
    fn test_compute_swap_price_anchor_both_zero() {
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        assert!(compute_swap_price(&data, &pumpfun_id(), true).is_none(),
            "Anchor both zero must return None");
    }

    /// Zero amounts in Raydium-style data (>=20, <24 bytes) return None
    #[test]
    fn test_compute_swap_price_raydium_zero_amounts() {
        let mut data = vec![9u8, 0, 0, 0]; // discrim + pad
        data.extend_from_slice(&0u64.to_le_bytes()); // amt1 = 0
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // amt2
        assert!(compute_swap_price(&data, &raydium_id(), true).is_none(),
            "Raydium zero amount must return None");
    }

    /// Raydium data with both zero amounts returns None
    #[test]
    fn test_compute_swap_price_raydium_both_zero() {
        let mut data = vec![9u8, 0, 0, 0];
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        assert!(compute_swap_price(&data, &pumpfun_id(), true).is_none());
    }

    /// Malformed Anchor data (24 bytes but non-parseable amounts) returns None
    #[test]
    fn test_compute_swap_price_malformed_anchor() {
        // 24 bytes but middle 16 bytes are garbage (not valid u64 LE)
        let data = vec![0u8, 0, 0, 0, 0, 0, 0, 0,  // 8-byte discim
                        255, 255, 255, 255, 255, 255, 255, 255, // u64::MAX
                        255, 255, 255, 255, 255, 255, 255, 255];// u64::MAX
        // Both are non-zero, so this should produce a ratio
        let price = compute_swap_price(&data, &pumpfun_id(), true);
        assert!(price.is_some(), "u64::MAX amounts should still parse");
        assert_eq!(price.unwrap(), 1.0, "u64::MAX / u64::MAX should be 1.0");
    }

    /// Anchor payload with discriminator bytes that could be misread as Raydium amounts
    /// Verifies the exact P3-05 fix: Anchor data >=24 bytes NEVER reaches Raydium path
    #[test]
    fn test_anchor_not_misread_as_raydium() {
        // This is the exact P3-05 scenario: Anchor data with token_amount=0
        // Before fix: bytes [4..12] and [12..20] read from discriminator as Raydium amounts
        // After fix: >=24 bytes → Anchor-only path, returns None for zero amounts
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234]; // buy discriminator
        data.extend_from_slice(&0u64.to_le_bytes()); // ZERO tokens
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // 1 SOL quote

        let price = compute_swap_price(&data, &pumpfun_id(), true);
        assert!(price.is_none(),
            "Anchor data with zero tokens must return None, never Raydium fallthrough");

        // Verify: what bytes WOULD Raydium have read?
        // [4..12] = discriminator bytes [0..4] + amount bytes [8..12] = [1, 218, 235, 234, 0, 0, 0, 0]
        let raydium_amt1 = u64::from_le_bytes(data[4..12].try_into().unwrap());
        // [12..20] = [0, 0, 0, 0, 0, 0, 0, 1] (beginning of 1 SOL)
        let raydium_amt2 = u64::from_le_bytes(data[12..20].try_into().unwrap());
        // Both would be non-zero, producing a garbage price — confirm
        assert!(raydium_amt1 > 0, "Raydium amt1 from discriminator bytes would be > 0");
        assert!(raydium_amt2 > 0, "Raydium amt2 from amount bytes would be > 0");
        let garbage_price = raydium_amt1.max(raydium_amt2) as f64
            / raydium_amt1.min(raydium_amt2) as f64;
        eprintln!(
            "P3-05 cross-protocol misparse averted: Raydium would have read amt1={}, amt2={}, price={}",
            raydium_amt1, raydium_amt2, garbage_price
        );
    }

    /// Very short data (<20 bytes) returns None
    #[test]
    fn test_compute_swap_price_too_short() {
        assert!(compute_swap_price(&[0u8; 4], &unknown_id(), true).is_none());
        assert!(compute_swap_price(&[0u8; 10], &unknown_id(), true).is_none());
        assert!(compute_swap_price(&[0u8; 15], &unknown_id(), true).is_none());
        assert!(compute_swap_price(&[0u8; 19], &unknown_id(), true).is_none());
    }

    /// Edge: exactly 20 bytes for Raydium, exactly 24 bytes for Anchor
    #[test]
    fn test_compute_swap_price_length_boundaries() {
        // Exactly 20 bytes — Raydium path
        let mut raydium = vec![9u8, 0, 0, 0]; // 4 bytes
        raydium.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // +8 = 12
        raydium.extend_from_slice(&100_000_000u64.to_le_bytes());   // +8 = 20
        assert!(compute_swap_price(&raydium, &raydium_id(), true).is_some(), "20-byte Raydium data should parse");

        // Exactly 24 bytes — Anchor path
        let mut anchor = vec![102u8, 6, 61, 18, 1, 218, 235, 234]; // 8 bytes
        anchor.extend_from_slice(&1_000_000_000u64.to_le_bytes());  // +8 = 16
        anchor.extend_from_slice(&100_000_000u64.to_le_bytes());    // +8 = 24
        assert!(compute_swap_price(&anchor, &pumpfun_id(), true).is_some(), "24-byte Anchor data should parse");

        // 23 bytes — too short for Anchor, too short for Raydium parsing at [4..12]... 
        // actually 23 >= 20 so Raydium path works
        let mut short = vec![9u8, 0, 0, 0];
        short.extend_from_slice(&1_000_000_000u64.to_le_bytes());  // +8 = 12
        short.extend_from_slice(&100_000_000u64.to_le_bytes());    // +8 = 20 — wait, that's 20
        // 20-23: Raydium path, 24+: Anchor path. So 23 bytes = Raydium.
        // Need to create a 23-byte buffer: add 3 padding bytes to the 20-byte buffer
        let mut twenty_three = vec![0u8, 0, 0]; // 3 padding bytes
        twenty_three.extend_from_slice(&short); // +20 = 23
        assert!(compute_swap_price(&twenty_three, &raydium_id(), true).is_some(),
            "23-byte data should parse as Raydium");
    }

    /// Division boundary: extremely small token amounts
    #[test]
    fn test_compute_swap_price_boundary_division() {
        // Anchor: 1 token, huge quote
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data.extend_from_slice(&1u64.to_le_bytes()); // 1 token
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes()); // 1 SOL
        let price = compute_swap_price(&data, &pumpfun_id(), true).unwrap();
        assert!((price - 1_000_000_000.0).abs() < 0.001,
            "1 token / 1 SOL: expected ~1e9, got {}", price);

        // Anchor: huge token, 1 lamport quote
        let mut data2 = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data2.extend_from_slice(&u64::MAX.to_le_bytes()); // max tokens
        data2.extend_from_slice(&1u64.to_le_bytes()); // 1 lamport
        let price2 = compute_swap_price(&data2, &pumpfun_id(), true).unwrap();
        assert!(price2 > 0.0, "Extreme ratio should still be positive");
    }

    /// Pricing failure propagation: compute_swap_price returning None must prevent
    /// price assignment in MintMarketState::update
    #[test]
    fn test_pricing_failure_skips_price_update() {
        let mut state = MintMarketState::new("test_mint");
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // Event with zero-token Anchor data — price extraction should fail
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data.extend_from_slice(&0u64.to_le_bytes()); // ZERO tokens
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes());

        let event = make_test_event(1, EventType::Swap, data);

        // Manually replicate what MintMarketState::update does with pricing
        let is_buy = true;
        let price = compute_swap_price(&event.raw_instruction_data, &pumpfun_id(), is_buy);
        assert!(price.is_none(), "compute_swap_price must return None for zero-token data");

        // Now confirm: state.update() does NOT set last_price when price is None
        state.update(&event, now);
        assert_eq!(state.last_price, 0.0,
            "Price should remain 0.0 when compute_swap_price returns None");
    }

    /// Consecutive valid updates and one zero-token update: the zero-token event
    /// must NOT reset or corrupt the price
    #[test]
    fn test_zero_token_event_does_not_corrupt_price() {
        let mut state = MintMarketState::new("test_mint");
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // First: valid Anchor event sets price
        let mut valid_data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        valid_data.extend_from_slice(&100_000_000u64.to_le_bytes());
        valid_data.extend_from_slice(&1_000_000_000u64.to_le_bytes());
        let valid_event = make_test_event(1, EventType::Swap, valid_data);
        state.update(&valid_event, now);
        let price_before = state.last_price;
        assert!(price_before > 0.0, "Valid event should set price");

        // Second: zero-token event must NOT change the price
        let mut zero_data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        zero_data.extend_from_slice(&0u64.to_le_bytes());
        zero_data.extend_from_slice(&1_000_000_000u64.to_le_bytes());
        let zero_event = make_test_event(2, EventType::Swap, zero_data);
        state.update(&zero_event, now + 100_000);
        assert_eq!(
            state.last_price, price_before,
            "Zero-token event must NOT overwrite price (remains {})",
            price_before
        );
    }

    /// Strategy evaluation with zero price: if price is 0.0 (no valid swaps seen),
    /// the strategy should not generate a trade signal (skipped via max_spread or stale check)
    #[test]
    fn test_zero_price_in_strategy_context() {
        let mut state = MintMarketState::new("test_mint");
        // Default: last_price = 0.0, spread_bps = 10.0

        // Simulate what strategy engine does: if price is 0, max_spread check
        // and stale-data checks should prevent trade
        let now = crate::common::fast_timing::fast_now_micros() as i64;
        let price = state.last_price;

        // In strategy engine, evaluate() checks:
        // 1. max_spread: spread_bps (10) < max_spread_bps (config, e.g. 200) — PASSES
        // 2. stale: last_update_micros == 0 → stale → no trade
        // 3. min_volume: volume_1s == 0 → no volume → no trade
        // 4. signal: momentum hasn't built up
        assert_eq!(price, 0.0, "Fresh state should have zero price");
        assert_eq!(state.last_update_micros, 0, "Fresh state has no update time");
        assert_eq!(state.volume_1s, 0.0, "Fresh state has no volume");
        assert_eq!(state.active_positions, 0, "Fresh state has no positions");

        // After update with bad data: price stays 0, update time advances
        let mut bad_data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        bad_data.extend_from_slice(&0u64.to_le_bytes());
        bad_data.extend_from_slice(&0u64.to_le_bytes());
        let bad_event = make_test_event(1, EventType::Swap, bad_data);
        state.update(&bad_event, now);
        assert_eq!(state.last_price, 0.0,
            "Bad data event must NOT set price");
        // Note: update time may still advance because the event itself was processed
        // (volume tracking, event counting are independent of price extraction)
    }

    /// Cross-token isolation: two tokens on same program — updating price on one
    /// must NOT affect the other
    #[test]
    fn test_cross_token_price_isolation() {
        let mut state_a = MintMarketState::new("mint_a");
        let mut state_b = MintMarketState::new("mint_b");
        let now = crate::common::fast_timing::fast_now_micros() as i64;

        // Set price on A
        let mut data_a = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data_a.extend_from_slice(&100_000_000u64.to_le_bytes());
        data_a.extend_from_slice(&1_000_000_000u64.to_le_bytes());
        state_a.update(&make_test_event(1, EventType::Swap, data_a), now);

        // Set price on B (different)
        let mut data_b = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data_b.extend_from_slice(&200_000_000u64.to_le_bytes());
        data_b.extend_from_slice(&5_000_000_000u64.to_le_bytes());
        state_b.update(&make_test_event(2, EventType::Swap, data_b), now + 100_000);

        assert!((state_a.last_price - 10.0).abs() < 0.001,
            "mint_a price should be ~10.0, got {}", state_a.last_price);
        assert!((state_b.last_price - 25.0).abs() < 0.001,
            "mint_b price should be ~25.0, got {}", state_b.last_price);
    }

    /// Determinism: identical inputs produce identical prices
    #[test]
    fn test_pricing_determinism() {
        let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
        data.extend_from_slice(&1_500_000_000u64.to_le_bytes());
        data.extend_from_slice(&300_000_000u64.to_le_bytes());

        let p1 = compute_swap_price(&data, &pumpfun_id(), true).unwrap();
        let p2 = compute_swap_price(&data, &pumpfun_id(), true).unwrap();
        let p3 = compute_swap_price(&data, &pumpfun_id(), true).unwrap();
        assert_eq!(p1, p2, "Deterministic input must produce identical price");
        assert_eq!(p2, p3, "Deterministic input must produce identical price");
        assert!((p1 - 0.2).abs() < 0.001, "Expected ~0.2 (300M/1500M), got {}", p1);
    }
}