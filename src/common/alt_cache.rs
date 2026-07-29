//! AltCache — LRU cache for Address Lookup Table accounts.
//!
//! Prevents redundant RPC calls for the same ALT address. The cache is
//! bounded by max entries (default 1024) and uses the `lru` crate.
//!
//! # Design
//! - `parking_lot::Mutex<LruCache<Pubkey, AddressLookupTableAccount>>`
//! - `get_or_fetch()` — non-blocking if cached, synchronous fetch on miss
//! - `invalidate()` — force re-fetch when an ALT is known to have changed
//! - Bounded memory: ~3.5 MiB for 1024 entries (each account ~3.5 KiB)

use lru::LruCache;
use parking_lot::Mutex;
use solana_message::AddressLookupTableAccount;
use solana_sdk::pubkey::Pubkey;
use std::num::NonZeroUsize;
use std::sync::Arc;

use crate::common::address_lookup;
use crate::common::SolanaRpcClient;

/// LRU cache for Address Lookup Table accounts.
pub struct AltCache {
    inner: Mutex<LruCache<Pubkey, Arc<AddressLookupTableAccount>>>,
    rpc_client: Arc<SolanaRpcClient>,
}

impl AltCache {
    /// Create a new ALT cache.
    ///
    /// * `capacity` — max number of cached ALTs (default 1024).
    /// * `rpc_url` — RPC endpoint for fetching ALT accounts on cache miss.
    pub fn new(capacity: NonZeroUsize, rpc_url: impl Into<String>) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(capacity)),
            rpc_client: Arc::new(SolanaRpcClient::new(rpc_url.into())),
        }
    }

    /// Create with the default capacity of 1024 entries.
    pub fn with_defaults(rpc_url: impl Into<String>) -> Self {
        Self::new(NonZeroUsize::new(1024).unwrap(), rpc_url)
    }

    /// Get a cached ALT, or fetch it from RPC and cache the result.
    pub async fn get_or_fetch(
        &self,
        key: &Pubkey,
    ) -> anyhow::Result<Arc<AddressLookupTableAccount>> {
        // Fast path: check cache
        {
            let mut cache = self.inner.lock();
            if let Some(entry) = cache.get(key) {
                return Ok(Arc::clone(entry));
            }
        }

        // Slow path: fetch from RPC
        let account = address_lookup::fetch_address_lookup_table_account(&self.rpc_client, key)
            .await?;
        let arc = Arc::new(account);

        // Store in cache
        let mut cache = self.inner.lock();
        cache.put(*key, Arc::clone(&arc));

        Ok(arc)
    }

    /// Get a cached ALT without fetching. Returns `None` if not in cache.
    #[inline]
    pub fn get(&self, key: &Pubkey) -> Option<Arc<AddressLookupTableAccount>> {
        self.inner.lock().get(key).map(Arc::clone)
    }

    /// Invalidate a specific ALT entry (forces re-fetch on next access).
    #[inline]
    pub fn invalidate(&self, key: &Pubkey) {
        self.inner.lock().pop(key);
    }

    /// Invalidate all cached ALTs.
    #[inline]
    pub fn clear(&self) {
        self.inner.lock().clear();
    }

    /// Number of entries currently cached.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// True if the cache is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }

    /// Pre-populate the cache with a known ALT (for testing or warmup).
    pub fn seed(&self, key: Pubkey, account: AddressLookupTableAccount) {
        let mut cache = self.inner.lock();
        cache.put(key, Arc::new(account));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn test_cache_basic() {
        let svc = AltCache::with_defaults("http://dummy");
        assert!(svc.is_empty());

        let key = Pubkey::new_unique();
        let alt = AddressLookupTableAccount {
            key,
            addresses: vec![],
        };
        svc.seed(key, alt);
        assert_eq!(svc.len(), 1);

        let cached = svc.get(&key);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().key, key);
    }

    #[test]
    fn test_lru_eviction() {
        let cap = NonZeroUsize::new(2).unwrap();
        let svc = AltCache::new(cap, "http://dummy");

        let k1 = Pubkey::new_unique();
        let k2 = Pubkey::new_unique();
        let k3 = Pubkey::new_unique();

        svc.seed(k1, AddressLookupTableAccount { key: k1, addresses: vec![] });
        svc.seed(k2, AddressLookupTableAccount { key: k2, addresses: vec![] });
        assert_eq!(svc.len(), 2);

        // Access k1 to make it recent
        let _ = svc.get(&k1);

        // Insert k3 — should evict k2 (LRU)
        svc.seed(k3, AddressLookupTableAccount { key: k3, addresses: vec![] });
        assert_eq!(svc.len(), 2);
        assert!(svc.get(&k1).is_some()); // k1 was recently accessed
        assert!(svc.get(&k2).is_none()); // k2 was evicted
        assert!(svc.get(&k3).is_some()); // k3 was just inserted
    }

    #[test]
    fn test_invalidate() {
        let svc = AltCache::with_defaults("http://dummy");
        let key = Pubkey::new_unique();
        svc.seed(key, AddressLookupTableAccount { key, addresses: vec![] });
        assert_eq!(svc.len(), 1);

        svc.invalidate(&key);
        assert!(svc.is_empty());
    }

    #[test]
    fn test_clear() {
        let svc = AltCache::with_defaults("http://dummy");
        let k1 = Pubkey::new_unique();
        let k2 = Pubkey::new_unique();
        svc.seed(k1, AddressLookupTableAccount { key: k1, addresses: vec![] });
        svc.seed(k2, AddressLookupTableAccount { key: k2, addresses: vec![] });
        assert_eq!(svc.len(), 2);

        svc.clear();
        assert_eq!(svc.len(), 0);
    }
}