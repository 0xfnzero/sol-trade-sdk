//! Transport continuity and wall-clock freshness for the collection stream example.
use sol_parser_sdk::grpc::GrpcSubscriptionStatus;
use std::time::{Duration, Instant};

pub struct CollectionStreamGuard {
    status: Option<GrpcSubscriptionStatus>,
    block: Option<(u64, Instant)>,
    // Keep the progress watermark when timestamps invalidate readiness. Otherwise
    // a replay of the same slot could restore freshness without chain progress.
    last_valid_slot: Option<u64>,
    timeout: Duration,
}
impl CollectionStreamGuard {
    pub fn new(timeout: Duration) -> Self {
        Self { status: None, block: None, last_valid_slot: None, timeout }
    }
    /// Returns true when all cached/queued account state must be discarded.
    pub fn observe_status(&mut self, status: GrpcSubscriptionStatus) -> bool {
        let previous = self.status.replace(status);
        let changed = match previous {
            Some(previous) => previous.continuity_revision != status.continuity_revision,
            None => {
                status.generation > 1
                    || status.disconnects != 0
                    || status.dropped_events != 0
                    || status.continuity_revision > status.generation
            }
        };
        // Only the exact initial 0 -> 1 connection transition may preserve seeded
        // state. A filter change on generation 1 still requires rebuilding.
        let first_connection = previous.is_some_and(|p| {
            p.generation == 0
                && status.generation == 1
                && status.continuity_revision == p.continuity_revision + 1
                && p.disconnects == 0
                && p.dropped_events == 0
                && status.disconnects == 0
                && status.dropped_events == 0
        });
        let gap = changed && !first_connection;
        if gap {
            self.block = None;
            self.last_valid_slot = None;
        }
        gap
    }
    pub fn observe_block(&mut self, slot: u64, now: Instant) {
        if self.last_valid_slot.is_none_or(|previous| slot > previous) {
            self.last_valid_slot = Some(slot);
            self.block = Some((slot, now));
        }
    }
    /// Queue waiting time counts toward freshness. Unknown/future timestamps do
    /// not prove that the provider is currently delivering block progress.
    pub fn observe_received_block(
        &mut self,
        slot: u64,
        received_us: i64,
        now_us: i64,
        now: Instant,
    ) {
        let age = now_us
            .checked_sub(received_us)
            .filter(|age| received_us > 0 && *age >= 0)
            .map(|age| Duration::from_micros(age as u64));
        let received = age.filter(|age| *age <= self.timeout).and_then(|age| now.checked_sub(age));
        match received {
            Some(received) => self.observe_block(slot, received),
            None => self.block = None,
        }
    }
    pub fn slot(&self) -> Option<u64> {
        self.block.map(|(slot, _)| slot)
    }
    pub fn is_live(&self, now: Instant) -> bool {
        self.status.is_some_and(|s| s.connected)
            && self
                .block
                .is_some_and(|(_, time)| now.saturating_duration_since(time) <= self.timeout)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn connected() -> GrpcSubscriptionStatus {
        GrpcSubscriptionStatus {
            connected: true,
            generation: 1,
            continuity_revision: 1,
            ..Default::default()
        }
    }
    #[test]
    fn stalled_or_repeated_block_slot_cannot_keep_cached_payout_ready() {
        let start = Instant::now();
        let mut guard = CollectionStreamGuard::new(Duration::from_secs(2));
        assert!(!guard.observe_status(connected()));
        assert!(!guard.is_live(start));
        guard.observe_block(100, start);
        assert!(guard.is_live(start));
        guard.observe_block(100, start + Duration::from_secs(3));
        assert!(!guard.is_live(start + Duration::from_secs(3)));
        guard.observe_block(101, start + Duration::from_secs(3));
        assert!(guard.is_live(start + Duration::from_secs(3)));
    }
    #[test]
    fn fast_reconnect_between_consumer_checks_invalidates_old_snapshot() {
        let start = Instant::now();
        let mut guard = CollectionStreamGuard::new(Duration::from_secs(10));
        guard.observe_status(connected());
        guard.observe_block(100, start);
        assert!(guard.observe_status(GrpcSubscriptionStatus {
            connected: true,
            generation: 2,
            continuity_revision: 3,
            disconnects: 1,
            dropped_events: 0
        }));
        assert!(!guard.is_live(start));
        assert_eq!(guard.slot(), None);
        guard.observe_block(101, start);
        assert!(guard.is_live(start));
    }
    #[test]
    fn overflow_requires_rebuild_and_first_connection_does_not_discard_seed() {
        let mut guard = CollectionStreamGuard::new(Duration::from_secs(10));
        assert!(!guard.observe_status(GrpcSubscriptionStatus::default()));
        assert!(!guard.observe_status(connected()));
        assert!(guard.observe_status(GrpcSubscriptionStatus {
            dropped_events: 1,
            continuity_revision: 2,
            ..connected()
        }));
        assert!(!guard.is_live(Instant::now()));
    }
    #[test]
    fn changed_filter_on_first_connection_still_invalidates_state() {
        let mut guard = CollectionStreamGuard::new(Duration::from_secs(10));
        guard.observe_status(connected());
        guard.observe_block(100, Instant::now());
        assert!(
            guard.observe_status(GrpcSubscriptionStatus { continuity_revision: 2, ..connected() })
        );
        assert_eq!(guard.slot(), None);
    }
    #[test]
    fn queue_delay_counts_toward_block_freshness() {
        let now = Instant::now();
        let mut guard = CollectionStreamGuard::new(Duration::from_secs(2));
        guard.observe_status(connected());
        guard.observe_received_block(100, 1_000_000, 4_000_000, now);
        assert!(!guard.is_live(now));
        guard.observe_received_block(101, 4_000_000, 4_500_000, now);
        assert!(guard.is_live(now));
        // Already spent half a second queued, so two more seconds is too old.
        assert!(!guard.is_live(now + Duration::from_secs(2)));
    }
    #[test]
    fn missing_or_future_receive_timestamps_withdraw_readiness() {
        let now = Instant::now();
        let mut guard = CollectionStreamGuard::new(Duration::from_secs(2));
        guard.observe_status(connected());
        guard.observe_received_block(100, 1_000_000, 1_000_000, now);
        assert!(guard.is_live(now));
        guard.observe_received_block(101, 0, 1_000_000, now);
        assert!(!guard.is_live(now));
        guard.observe_received_block(102, 2_000_000, 1_000_000, now);
        assert!(!guard.is_live(now));
    }
    #[test]
    fn invalid_timestamp_then_same_slot_replay_cannot_restore_readiness() {
        let now = Instant::now();
        for (invalid_receive_us, invalid_now_us) in
            [(0, 1_000_000), (2_000_000, 1_000_000), (1, 4_000_000), (i64::MIN, i64::MAX)]
        {
            let mut guard = CollectionStreamGuard::new(Duration::from_secs(2));
            guard.observe_status(connected());
            guard.observe_received_block(100, 1_000_000, 1_000_000, now);
            assert!(guard.is_live(now));
            guard.observe_received_block(101, invalid_receive_us, invalid_now_us, now);
            assert!(!guard.is_live(now));
            for slot in [99, 100, 100] {
                guard.observe_received_block(slot, 1_000_000, 1_000_000, now);
                assert!(!guard.is_live(now), "replayed slot {slot} restored readiness");
            }
            guard.observe_received_block(101, 1_000_000, 1_000_000, now);
            assert!(guard.is_live(now));
        }
    }
    #[test]
    fn reconnect_resets_slot_watermark_for_a_new_snapshot() {
        let now = Instant::now();
        let mut guard = CollectionStreamGuard::new(Duration::from_secs(2));
        guard.observe_status(connected());
        guard.observe_block(100, now);
        assert!(guard.observe_status(GrpcSubscriptionStatus {
            generation: 2,
            continuity_revision: 3,
            disconnects: 1,
            ..connected()
        }));
        guard.observe_block(99, now);
        assert!(guard.is_live(now));
        assert_eq!(guard.slot(), Some(99));
    }
}
