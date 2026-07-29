//! BlockhashService — background-refresh blockhash cache.
//!
//! Fetches the latest blockhash from RPC at a configurable interval,
//! maintains a small pool of recent hashes, and provides a non-blocking
//! `get()` that returns the freshest known hash.
//!
//! # Design
//! - Background task spawned via `spawn_refresh()` — call once when tokio runtime is ready
//! - Pool of up to `capacity` recent hashes (default 3)
//! - `get()` returns `None` only if the pool is empty (not yet initialized)
//! - `get_next()` returns a different hash than `get()` for multi-lane submission
//! - Stale detection via `get_with_max_age()`
//! - The hot path is a single `Arc<RwLock<>>` read — no RPC, no I/O

use parking_lot::RwLock;
use solana_hash::Hash;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::common::SolanaRpcClient;

// ---------------------------------------------------------------------------
// Pool internals
// ---------------------------------------------------------------------------

struct BlockhashPool {
    /// Circular buffer of (hash, fetch_time), newest at back.
    hashes: VecDeque<(Hash, Instant)>,
    /// Max entries before the oldest is dropped.
    capacity: usize,
}

// ---------------------------------------------------------------------------
// BlockhashService
// ---------------------------------------------------------------------------

/// Shared blockhash cache with a background refresh task.
///
/// # Example
/// ```rust,ignore
/// let svc = BlockhashService::new("https://api.mainnet-beta.solana.com", 3, 200);
/// let handle = svc.spawn_refresh();
/// // ... on trade hot path:
/// if let Some(hash) = svc.get() {
///     // build transaction with hash
/// }
/// ```
pub struct BlockhashService {
    pool: Arc<RwLock<BlockhashPool>>,
    rpc_client: Arc<SolanaRpcClient>,
    refresh_interval: Duration,
}

impl BlockhashService {
    /// Create a new service. The RPC client is used by the background refresh
    /// task. Does NOT start the task — call [`Self::spawn_refresh`] when the
    /// tokio runtime is ready.
    pub fn new(rpc_url: impl Into<String>, capacity: usize, refresh_interval_ms: u64) -> Self {
        let rpc = Arc::new(SolanaRpcClient::new(rpc_url.into()));
        Self {
            pool: Arc::new(RwLock::new(BlockhashPool {
                hashes: VecDeque::with_capacity(capacity + 1),
                capacity,
            })),
            rpc_client: rpc,
            refresh_interval: Duration::from_millis(refresh_interval_ms),
        }
    }

    /// Create from a [`crate::common::config::BlockhashConfig`] and RPC URL.
    pub fn from_config(rpc_url: impl Into<String>, config: &crate::common::config::BlockhashConfig) -> Self {
        Self::new(rpc_url, 3, config.refresh_interval_ms)
    }

    /// Spawn the background refresh loop. Returns a [`tokio::task::JoinHandle`]
    /// the caller may store or detach.
    pub fn spawn_refresh(&self) -> tokio::task::JoinHandle<()> {
        let pool = self.pool.clone();
        let rpc = self.rpc_client.clone();
        let interval = self.refresh_interval;

        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tick.tick().await;
                // Fetch the latest blockhash with processed commitment (fastest)
                let result = rpc.get_latest_blockhash().await;
                match result {
                    Ok(hash) => {
                        let mut guard = pool.write();
                        guard.hashes.push_back((hash, Instant::now()));
                        while guard.hashes.len() > guard.capacity {
                            guard.hashes.pop_front();
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[blockhash] refresh failed: {e}");
                    }
                }
            }
        })
    }

    /// Return the freshest cached blockhash, or `None` if the pool is empty.
    #[inline]
    pub fn get(&self) -> Option<Hash> {
        self.pool.read().hashes.back().map(|(h, _)| *h)
    }

    /// Return the freshest blockhash only if it's younger than `max_age`.
    #[inline]
    pub fn get_with_max_age(&self, max_age: Duration) -> Option<Hash> {
        let guard = self.pool.read();
        guard.hashes.back().and_then(|(h, ts)| {
            if ts.elapsed() <= max_age { Some(*h) } else { None }
        })
    }

    /// Return a blockhash *different* from the current freshest one, if available.
    /// Useful for multi-lane submission where each lane should use a distinct hash.
    #[inline]
    pub fn get_next(&self) -> Option<Hash> {
        let guard = self.pool.read();
        if guard.hashes.len() >= 2 {
            guard.hashes.get(guard.hashes.len() - 2).map(|(h, _)| *h)
        } else {
            guard.hashes.back().map(|(h, _)| *h)
        }
    }

    /// Number of cached blockhashes.
    #[inline]
    pub fn len(&self) -> usize {
        self.pool.read().hashes.len()
    }

    /// True if no blockhash has been cached yet.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pool.read().hashes.is_empty()
    }

    /// Manually seed a blockhash (for testing or pre-initialization).
    pub fn seed(&self, hash: Hash) {
        let mut guard = self.pool.write();
        guard.hashes.push_back((hash, Instant::now()));
        while guard.hashes.len() > guard.capacity {
            guard.hashes.pop_front();
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
    fn test_pool_basic() {
        let svc = BlockhashService::new("http://dummy", 3, 200);
        assert!(svc.is_empty());
        assert!(svc.get().is_none());

        let h1 = Hash::new_unique();
        svc.seed(h1);
        assert_eq!(svc.len(), 1);
        assert_eq!(svc.get(), Some(h1));
    }

    #[test]
    fn test_capacity_eviction() {
        let svc = BlockhashService::new("http://dummy", 2, 200);
        let (h1, h2, h3) = (Hash::new_unique(), Hash::new_unique(), Hash::new_unique());

        svc.seed(h1);
        svc.seed(h2);
        assert_eq!(svc.len(), 2);

        svc.seed(h3);
        assert_eq!(svc.len(), 2);
        // h1 (oldest) evicted
        assert_eq!(svc.get(), Some(h3));
    }

    #[test]
    fn test_get_next_single() {
        let svc = BlockhashService::new("http://dummy", 3, 200);
        let h1 = Hash::new_unique();
        svc.seed(h1);
        // Only one entry → get_next falls back to newest
        assert_eq!(svc.get_next(), Some(h1));
    }

    #[test]
    fn test_get_next_multi() {
        let svc = BlockhashService::new("http://dummy", 3, 200);
        let (h1, h2) = (Hash::new_unique(), Hash::new_unique());
        svc.seed(h1);
        svc.seed(h2);
        // Two entries → get_next returns second-newest (h1)
        assert_eq!(svc.get_next(), Some(h1));
        // get() still returns newest (h2)
        assert_eq!(svc.get(), Some(h2));
    }

    #[test]
    fn test_stale() {
        let svc = BlockhashService::new("http://dummy", 3, 200);
        svc.seed(Hash::new_unique());
        // Just seeded — should be fresh
        assert!(svc.get_with_max_age(Duration::from_secs(60)).is_some());
    }
}