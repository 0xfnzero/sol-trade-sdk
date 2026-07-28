//! Bounded LRU deduplication cache for ShredStream events.
//!
//! Deduplicates by the composite key `(slot, signature, event_type, instruction_index)`.
//! Capacity is configurable; eviction follows LRU policy.
//!
//! NOTE: `contains` does NOT promote to MRU. This is intentional — we don't want
//! stale dedup keys to be kept alive by repeated checking. Only `put` promotes.

use lru::LruCache;
use std::num::NonZeroUsize;

/// The composite key used for deduplication.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct DedupKey {
    pub slot: u64,
    pub signature: [u8; 64],
    pub event_type: u8, // mapped from EventType discriminant
    pub instruction_index: u8,
}

/// A bounded LRU cache for deduplication.
///
/// Returns `true` from `check_and_insert` if the key is **new** (not seen before).
/// Returns `false` if the key was already present (duplicate).
#[derive(Debug)]
pub struct DedupCache {
    inner: LruCache<DedupKey, ()>,
}

impl DedupCache {
    /// Create a new dedup cache with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self { inner: LruCache::new(capacity) }
    }

    /// Check whether `key` has been seen before.
    ///
    /// Returns `true` if this is a **new** key (not in the cache).
    /// Returns `false` if the key is already present (duplicate).
    ///
    /// Inserts the key on first occurrence. Does NOT promote on duplicate check.
    pub fn check_and_insert(&mut self, key: DedupKey) -> bool {
        if self.inner.contains(&key) {
            false
        } else {
            self.inner.put(key, ());
            true
        }
    }

    /// Returns the number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Clear all entries from the cache.
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key(slot: u64, sig_byte: u8) -> DedupKey {
        DedupKey { slot, signature: [sig_byte; 64], event_type: 1, instruction_index: 0 }
    }

    #[test]
    fn test_first_seen_returns_true() {
        let mut cache = DedupCache::new(NonZeroUsize::new(1024).unwrap());
        assert!(cache.check_and_insert(test_key(1, 0xaa)));
    }

    #[test]
    fn test_duplicate_returns_false() {
        let mut cache = DedupCache::new(NonZeroUsize::new(1024).unwrap());
        let key = test_key(1, 0xaa);
        assert!(cache.check_and_insert(key.clone()));
        assert!(!cache.check_and_insert(key));
    }

    #[test]
    fn test_lru_eviction() {
        let mut cache = DedupCache::new(NonZeroUsize::new(3).unwrap());

        // Fill cache with 3 entries
        // Cache (LRU → MRU): [k1, k2, k3]
        assert!(cache.check_and_insert(test_key(1, 0x01)));
        assert!(cache.check_and_insert(test_key(2, 0x02)));
        assert!(cache.check_and_insert(test_key(3, 0x03)));
        assert_eq!(cache.len(), 3);

        // Insert key 4 — LRU eviction should evict key 1
        // Cache after: [k2, k3, k4]
        assert!(cache.check_and_insert(test_key(4, 0x04)));
        assert_eq!(cache.len(), 3);

        // Key 1 was evicted, so it should return true (new again)
        assert!(cache.check_and_insert(test_key(1, 0x01)));
    }

    #[test]
    fn test_len_and_empty() {
        let mut cache = DedupCache::new(NonZeroUsize::new(10).unwrap());
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        cache.check_and_insert(test_key(1, 0x01));
        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_different_event_type_not_deduped() {
        let mut cache = DedupCache::new(NonZeroUsize::new(1024).unwrap());
        let mut k1 = test_key(1, 0xaa);
        k1.event_type = 1;
        let mut k2 = test_key(1, 0xaa);
        k2.event_type = 2;
        assert!(cache.check_and_insert(k1));
        assert!(cache.check_and_insert(k2)); // different event type → new
    }
}