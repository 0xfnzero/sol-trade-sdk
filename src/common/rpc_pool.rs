//! RpcConnectionPool — HTTP connection reuse for RPC endpoints.
//!
//! Maintains a pool of pre-configured `reqwest::Client` instances keyed by
//! endpoint URL. Each client reuses TCP connections (HTTP keep-alive), avoiding
//! TLS handshake + DNS overhead per RPC call.
//!
//! # Design
//! - `DashMap<String, reqwest::Client>` — concurrent, lock-free reads
//! - Same client used for both primary and fallback RPC endpoints
//! - Configurable pool size (idle connections per endpoint)
//! - Health check: `get_height()` ping to verify endpoint is operational
//! - Metrics: tracks request count, error count per endpoint

use dashmap::DashMap;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// RpcConnectionPool
// ---------------------------------------------------------------------------

/// Pool of pre-configured HTTP clients, one per RPC endpoint URL.
pub struct RpcConnectionPool {
    clients: Arc<DashMap<String, Client>>,
    /// Max idle connections per endpoint.
    pool_size: usize,
    /// Request timeout.
    timeout: Duration,
    /// Per-endpoint error counts (URL → consecutive errors).
    errors: Arc<DashMap<String, u32>>,
    /// Per-endpoint request counters.
    requests: Arc<DashMap<String, u64>>,
    /// Max consecutive errors before marking endpoint degraded.
    max_errors_before_degraded: u32,
}

impl RpcConnectionPool {
    /// Create a new connection pool.
    pub fn new(pool_size: usize, timeout_secs: u64, max_errors_before_degraded: u32) -> Self {
        Self {
            clients: Arc::new(DashMap::new()),
            pool_size,
            timeout: Duration::from_secs(timeout_secs),
            errors: Arc::new(DashMap::new()),
            requests: Arc::new(DashMap::new()),
            max_errors_before_degraded,
        }
    }

    /// Create with sensible defaults: 4-way connection pool per endpoint, 30s timeout.
    pub fn with_defaults() -> Self {
        Self::new(4, 30, 5)
    }

    /// Get (or create) an HTTP client for the given RPC URL.
    pub fn get_client(&self, url: &str) -> Client {
        self.clients
            .entry(url.to_string())
            .or_insert_with(|| self.build_client(url))
            .value()
            .clone()
    }

    /// Build a new reqwest client for the given URL.
    fn build_client(&self, url: &str) -> Client {
        let mut builder = Client::builder()
            .timeout(self.timeout)
            .pool_idle_timeout(Some(Duration::from_secs(90)))
            .pool_max_idle_per_host(self.pool_size)
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .user_agent("sol-trade-sdk/5.0.0");

        // If the URL uses HTTPS, enable native TLS
        if url.starts_with("https") {
            let mut root_store = rustls::RootCertStore::empty();
            let native = rustls_native_certs::load_native_certs();
            for cert in native.certs {
                if let Err(e) = root_store.add(cert) {
                    tracing::warn!("Failed to add native cert: {e}");
                }
            }
            for err in &native.errors {
                tracing::warn!("Native cert load error: {err}");
            }
            builder = builder.use_preconfigured_tls(
                rustls::ClientConfig::builder()
                    .with_root_certificates(root_store)
                    .with_no_client_auth(),
            );
        }

        builder.build().expect("RpcConnectionPool: reqwest client build")
    }

    /// Check if an endpoint is healthy by calling `getHeight`.
    pub async fn check_health(&self, url: &str) -> anyhow::Result<()> {
        let client = self.get_client(url);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getHeight",
            "params": []
        });

        let resp = client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Health check failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("Health check returned HTTP {status}");
        }

        let text = resp
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Read health response: {e}"))?;

        let parsed: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Health JSON parse: {e}"))?;

        if parsed.get("error").is_some() {
            anyhow::bail!("Health check RPC error: {}", parsed["error"]);
        }

        Ok(())
    }

    /// Record a successful RPC request (resets error counter).
    pub fn record_success(&self, url: &str) {
        self.errors.insert(url.to_string(), 0);
        *self.requests.entry(url.to_string()).or_insert(0) += 1;
    }

    /// Record a failed RPC request (increments error counter).
    pub fn record_failure(&self, url: &str) -> u32 {
        let mut errs = self.errors.entry(url.to_string()).or_insert(0);
        *errs += 1;
        *errs
    }

    /// Returns `true` if the endpoint has exceeded the error threshold.
    pub fn is_degraded(&self, url: &str) -> bool {
        self.errors.get(url).map_or(false, |e| *e >= self.max_errors_before_degraded)
    }

    /// Return the request count for an endpoint.
    pub fn request_count(&self, url: &str) -> u64 {
        self.requests.get(url).map_or(0, |r| *r)
    }

    /// Return the error count for an endpoint.
    pub fn error_count(&self, url: &str) -> u32 {
        self.errors.get(url).map_or(0, |e| *e)
    }

    /// Remove a client from the pool (forces rebuild on next access).
    pub fn evict(&self, url: &str) {
        self.clients.remove(url);
        self.errors.remove(url);
    }

    /// Number of cached clients.
    #[inline]
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// True if no clients have been created yet.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensure rustls CryptoProvider is installed for TLS client configs.
    fn install_provider_once() {
        use std::sync::Once;
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    #[test]
    fn test_pool_creates_and_reuses_client() {
        install_provider_once();
        let pool = RpcConnectionPool::with_defaults();
        let url = "https://api.mainnet-beta.solana.com";

        let c1 = pool.get_client(url);
        let c2 = pool.get_client(url);

        // Same client should be returned (same reqwest::Client internally)
        // Can't compare reqwest::Client directly, but pool size should be 1
        assert_eq!(pool.len(), 1);
        assert!(!pool.is_empty());
    }

    #[test]
    fn test_multiple_endpoints() {
        install_provider_once();
        let pool = RpcConnectionPool::with_defaults();
        let url1 = "https://api.mainnet-beta.solana.com";
        let url2 = "https://solana-api.projectserum.com";

        let _ = pool.get_client(url1);
        let _ = pool.get_client(url2);
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_error_tracking() {
        install_provider_once();
        let pool = RpcConnectionPool::with_defaults();
        let url = "https://test.example.com";

        assert_eq!(pool.error_count(url), 0);
        assert!(!pool.is_degraded(url));

        pool.record_failure(url);
        pool.record_failure(url);
        pool.record_failure(url);
        assert_eq!(pool.error_count(url), 3);

        pool.record_success(url);
        assert_eq!(pool.error_count(url), 0);
    }

    #[test]
    fn test_degraded_threshold() {
        let pool = RpcConnectionPool::new(4, 30, 3);
        let url = "https://test.example.com";

        pool.record_failure(url);
        pool.record_failure(url);
        assert!(!pool.is_degraded(url));

        pool.record_failure(url); // now at threshold
        assert!(pool.is_degraded(url));
    }

    #[test]
    fn test_evict() {
        install_provider_once();
        let pool = RpcConnectionPool::with_defaults();
        let url = "https://test.example.com";

        let _ = pool.get_client(url);
        assert_eq!(pool.len(), 1);

        // Record some errors before evict
        pool.record_failure(url);
        pool.evict(url);
        assert_eq!(pool.len(), 0);
        assert_eq!(pool.error_count(url), 0);
    }

    #[test]
    fn test_request_count() {
        let pool = RpcConnectionPool::with_defaults();
        let url = "https://test.example.com";

        assert_eq!(pool.request_count(url), 0);
        pool.record_success(url);
        pool.record_success(url);
        pool.record_success(url);
        assert_eq!(pool.request_count(url), 3);
    }
}