//! Atomic counters for ShredStream observability.
//!
//! All counters are lock-free atomics suitable for hot-path incrementing.
//! A background thread reads and exports these on a periodic interval.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// ShredStream pipeline metrics — all fields are `AtomicU64` for lock-free hot-path updates.
#[derive(Debug, Default)]
pub struct ShredstreamMetrics {
    // ── Throughput ──
    pub packets_received: AtomicU64,
    pub bytes_received: AtomicU64,
    pub slots_completed: AtomicU64,
    pub transactions_decoded: AtomicU64,

    // ── Kernel / transport ──
    pub fec_recoveries: AtomicU64,
    pub fec_recovery_failures: AtomicU64,
    pub fec_sets_discarded: AtomicU64,
    pub unparseable_packets: AtomicU64,

    // ── Filtering ──
    pub filter_program_id: AtomicU64,
    pub filter_vote: AtomicU64,
    pub events_classified: AtomicU64,

    // ── Dedup ──
    pub dedup_hits: AtomicU64,
    pub dedup_misses: AtomicU64,

    // ── Queues ──
    pub queue_depth_raw_packets: AtomicU64,
    pub queue_depth_decoded_slots: AtomicU64,
    pub queue_depth_classified_events: AtomicU64,
    pub queue_depth_trade_intents: AtomicU64,
    pub queue_drops_raw_packets: AtomicU64,
    pub queue_drops_classified_events: AtomicU64,
    pub queue_drops_trade_intents: AtomicU64,

    // ── Slot quality ──
    pub incomplete_slots: AtomicU64,
    pub slots_evicted_by_age: AtomicU64,
    pub batches_force_finalized_timeout: AtomicU64,

    // ── Stale events ──
    pub stale_events_dropped: AtomicU64,

    // ── Degraded mode ──
    pub degraded_mode_entries: AtomicU64,
    pub degraded_mode_exits: AtomicU64,
    pub stop_trading_triggers: AtomicU64,
}

impl ShredstreamMetrics {
    /// Create a new metrics instance wrapped in `Arc`.
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

// ── Snapshot helper ──

/// A point-in-time snapshot of all ShredStream metrics.
#[derive(Debug, Clone, Default)]
pub struct MetricsSnapshot {
    pub packets_received: u64,
    pub bytes_received: u64,
    pub slots_completed: u64,
    pub transactions_decoded: u64,
    pub fec_recoveries: u64,
    pub fec_recovery_failures: u64,
    pub fec_sets_discarded: u64,
    pub unparseable_packets: u64,
    pub filter_program_id: u64,
    pub filter_vote: u64,
    pub events_classified: u64,
    pub dedup_hits: u64,
    pub dedup_misses: u64,
    pub queue_drops_raw_packets: u64,
    pub queue_drops_classified_events: u64,
    pub queue_drops_trade_intents: u64,
    pub incomplete_slots: u64,
    pub slots_evicted_by_age: u64,
    pub batches_force_finalized_timeout: u64,
    pub stale_events_dropped: u64,
    pub degraded_mode_entries: u64,
    pub degraded_mode_exits: u64,
    pub stop_trading_triggers: u64,
}

impl From<&ShredstreamMetrics> for MetricsSnapshot {
    fn from(m: &ShredstreamMetrics) -> Self {
        Self {
            packets_received: m.packets_received.load(Ordering::Relaxed),
            bytes_received: m.bytes_received.load(Ordering::Relaxed),
            slots_completed: m.slots_completed.load(Ordering::Relaxed),
            transactions_decoded: m.transactions_decoded.load(Ordering::Relaxed),
            fec_recoveries: m.fec_recoveries.load(Ordering::Relaxed),
            fec_recovery_failures: m.fec_recovery_failures.load(Ordering::Relaxed),
            fec_sets_discarded: m.fec_sets_discarded.load(Ordering::Relaxed),
            unparseable_packets: m.unparseable_packets.load(Ordering::Relaxed),
            filter_program_id: m.filter_program_id.load(Ordering::Relaxed),
            filter_vote: m.filter_vote.load(Ordering::Relaxed),
            events_classified: m.events_classified.load(Ordering::Relaxed),
            dedup_hits: m.dedup_hits.load(Ordering::Relaxed),
            dedup_misses: m.dedup_misses.load(Ordering::Relaxed),
            queue_drops_raw_packets: m.queue_drops_raw_packets.load(Ordering::Relaxed),
            queue_drops_classified_events: m.queue_drops_classified_events.load(Ordering::Relaxed),
            queue_drops_trade_intents: m.queue_drops_trade_intents.load(Ordering::Relaxed),
            incomplete_slots: m.incomplete_slots.load(Ordering::Relaxed),
            slots_evicted_by_age: m.slots_evicted_by_age.load(Ordering::Relaxed),
            batches_force_finalized_timeout: m.batches_force_finalized_timeout.load(Ordering::Relaxed),
            stale_events_dropped: m.stale_events_dropped.load(Ordering::Relaxed),
            degraded_mode_entries: m.degraded_mode_entries.load(Ordering::Relaxed),
            degraded_mode_exits: m.degraded_mode_exits.load(Ordering::Relaxed),
            stop_trading_triggers: m.stop_trading_triggers.load(Ordering::Relaxed),
        }
    }
}