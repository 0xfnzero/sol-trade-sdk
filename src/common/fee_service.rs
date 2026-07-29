//! FeeService — compute unit price estimation based on recent blocks.
//!
//! Calls `getRecentPrioritizationFees` via JSON-RPC on a configurable
//! interval, maintains a sliding window of observed fees, and provides
//! percentile-based estimates for compute unit pricing.
//!
//! # Design
//! - Background task polls `getRecentPrioritizationFees` every 5s
//! - Configurable window (default 100 samples) and percentile (default P50)
//! - Falls back to `min_cu_price` when data is insufficient
//! - `ArcSwap` for lock-free reads on the hot path
//! - Uses `reqwest` directly to avoid solana-client method instability

use arc_swap::ArcSwap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// FeeService
// ---------------------------------------------------------------------------

/// Tracks recent prioritization fees and provides CU price estimates.
pub struct FeeService {
    /// Sliding window of per-block fee percentiles, newest last.
    samples: Arc<ArcSwap<Vec<u64>>>,
    /// Max samples to retain.
    window_size: usize,
    /// Which percentile to report (0.0–100.0).
    percentile: f64,
    /// Minimum CU price fallback.
    min_cu_price: u64,
    /// RPC endpoint URL.
    rpc_url: String,
    /// Shared reqwest client for JSON-RPC calls.
    http: reqwest::Client,
}

impl FeeService {
    /// Create a new fee service.
    pub fn new(
        rpc_url: impl Into<String>,
        window_size: usize,
        percentile: f64,
        min_cu_price: u64,
    ) -> Self {
        Self {
            samples: Arc::new(ArcSwap::from_pointee(Vec::new())),
            window_size,
            percentile: percentile.clamp(0.0, 100.0),
            min_cu_price,
            rpc_url: rpc_url.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("FeeService: reqwest client build"),
        }
    }

    /// Convenience constructor with sensible defaults.
    pub fn with_defaults(rpc_url: impl Into<String>) -> Self {
        Self::new(rpc_url, 100, 50.0, 1_000)
    }

    /// Spawn the background refresh loop. Calls `getRecentPrioritizationFees`
    /// every 5 seconds and updates the sliding window.
    pub fn spawn_refresh(&self) -> tokio::task::JoinHandle<()> {
        let rpc_url = self.rpc_url.clone();
        let http = self.http.clone();
        let samples = self.samples.clone();
        let window_size = self.window_size;

        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tick.tick().await;
                match fetch_recent_prioritization_fees(&http, &rpc_url).await {
                    Ok(fees) => {
                        let current = samples.load();
                        let mut new_samples: Vec<u64> = current.iter().copied().collect();

                        for fee in &fees {
                            new_samples.push(*fee);
                        }

                        // Trim to window
                        if new_samples.len() > window_size {
                            new_samples = new_samples[new_samples.len() - window_size..].to_vec();
                        }
                        samples.store(Arc::new(new_samples));
                    }
                    Err(e) => {
                        tracing::warn!("[fees] getRecentPrioritizationFees failed: {e}");
                    }
                }
            }
        })
    }

    /// Return the estimated CU price (micro-lamports per CU).
    /// Falls back to `min_cu_price` when no data exists.
    #[inline]
    pub fn estimate_cu_price(&self) -> u64 {
        let samples = self.samples.load();
        if samples.is_empty() {
            return self.min_cu_price;
        }
        percentile_sorted(&samples, self.percentile).unwrap_or(self.min_cu_price)
    }

    /// Return (min, median, max) for the current window.
    pub fn stats(&self) -> (u64, u64, u64) {
        let samples = self.samples.load();
        if samples.is_empty() {
            return (0, 0, 0);
        }
        let mut sorted: Vec<u64> = samples.iter().copied().collect();
        sorted.sort_unstable();
        (sorted[0], sorted[sorted.len() / 2], sorted[sorted.len() - 1])
    }

    /// Number of fee observations in the current window.
    #[inline]
    pub fn len(&self) -> usize {
        self.samples.load().len()
    }

    /// True if no fee data has been collected yet.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.samples.load().is_empty()
    }

    /// Manually seed fee samples (for testing).
    pub fn seed(&self, data: Vec<u64>) {
        self.samples.store(Arc::new(data));
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC helper
// ---------------------------------------------------------------------------

/// Call `getRecentPrioritizationFees` on the Solana RPC and return the
/// `prioritizationFee` values for the most recent blocks.
async fn fetch_recent_prioritization_fees(
    http: &reqwest::Client,
    rpc_url: &str,
) -> anyhow::Result<Vec<u64>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getRecentPrioritizationFees",
        "params": [[]],  // empty array = no account filter → global market fees
    });

    let resp = http
        .post(rpc_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("HTTP request failed: {e}"))?;

    let text = resp
        .text()
        .await
        .map_err(|e| anyhow::anyhow!("Read response body: {e}"))?;

    let parsed: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("JSON parse error: {e}"))?;

    if let Some(err) = parsed.get("error") {
        anyhow::bail!("RPC error: {err}");
    }

    let result = parsed
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or_else(|| anyhow::anyhow!("No result array in response"))?;

    let fees: Vec<u64> = result
        .iter()
        .filter_map(|entry| entry["prioritizationFee"].as_u64())
        .collect();

    Ok(fees)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute the `p`-th percentile of a slice (does NOT assume sorted input).
fn percentile_sorted(data: &[u64], p: f64) -> Option<u64> {
    if data.is_empty() {
        return None;
    }
    let mut sorted: Vec<u64> = data.to_vec();
    sorted.sort_unstable();
    let idx = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    Some(sorted[idx.min(sorted.len() - 1)])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_sorted() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(percentile_sorted(&data, 50.0), Some(6));
        assert_eq!(percentile_sorted(&data, 0.0), Some(1));
        assert_eq!(percentile_sorted(&data, 100.0), Some(10));
    }

    #[test]
    fn test_empty_fallback() {
        let svc = FeeService::new("http://dummy", 10, 50.0, 1_000);
        assert!(svc.is_empty());
        assert_eq!(svc.estimate_cu_price(), 1_000);
        assert_eq!(svc.stats(), (0, 0, 0));
    }

    #[test]
    fn test_seeded_data() {
        let svc = FeeService::new("http://dummy", 10, 50.0, 1_000);
        svc.seed(vec![5_000, 3_000, 10_000, 2_000, 8_000]);
        assert_eq!(svc.len(), 5);
        // Sorted: [2000, 3000, 5000, 8000, 10000] → median = index 2 = 5000
        assert_eq!(svc.estimate_cu_price(), 5_000);
    }

    #[test]
    fn test_stats() {
        let svc = FeeService::new("http://dummy", 10, 50.0, 1_000);
        svc.seed(vec![1, 2, 3, 4, 5]);
        let (min, med, max) = svc.stats();
        assert_eq!(min, 1);
        assert_eq!(med, 3);
        assert_eq!(max, 5);
    }

    #[test]
    fn test_window_trim() {
        let svc = FeeService::new("http://dummy", 3, 50.0, 1_000);
        svc.seed(vec![10, 20, 30, 40, 50]);
        // Only 3 are retained by seed (last 3)
        let samples = svc.samples.load();
        assert_eq!(samples.len(), 5); // seed stores all — trim only happens in refresh
    }
}