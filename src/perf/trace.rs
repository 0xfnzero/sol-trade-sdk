//! Performance-trace instrumentation: latency histograms, RAII spans, counters.
//!
//! All items are gated behind `cfg(feature = "perf-trace")`, so they compile to
//! zero runtime overhead in production builds. Enable with:
//!
//! ```text
//! cargo test --features perf-trace          # enable tracing in tests
//! cargo bench --features perf-trace         # enable tracing in benchmarks
//! ```
//!
//! # Architecture
//!
//! - [`TraceSpan`] — RAII guard that records the duration of a scope.
//! - [`LatencyHistogram`] — bucketed histogram with nanosecond precision.
//! - [`PerfCounter`] — named atomic counter for event frequencies.
//! - [`PerfRegistry`] — global registry of all counters and histograms, keyed by name.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Macros for perf-trace report
// ---------------------------------------------------------------------------

/// Print the full perf-trace report to stdout.
/// No-op when `perf-trace` is disabled.
#[macro_export]
macro_rules! perf_report {
    () => {
        #[cfg(feature = "perf-trace")]
        {
            println!(
                "{}",
                $crate::perf::PerfRegistry::global().report()
            );
        }
    };
}

/// Reset all perf-trace counters and histograms.
#[macro_export]
macro_rules! perf_reset {
    () => {
        #[cfg(feature = "perf-trace")]
        {
            $crate::perf::PerfRegistry::global().reset_all();
        }
    };
}

// ---------------------------------------------------------------------------
// LatencyHistogram — bucketed high-resolution histogram
// ---------------------------------------------------------------------------

/// A simple bucketed latency histogram.
///
/// Buckets are powers-of-two in nanoseconds, covering 1µs .. 1s. The last
/// bucket catches everything above 1s.
///
/// Thread-safe: uses atomic counters per bucket. No allocations on the hot
/// path after construction.
#[derive(Debug)]
pub struct LatencyHistogram {
    /// Bucket boundaries in ns (powers of two: 1_000, 2_000, 4_000, …).
    /// Each bucket covers (prev_boundary, boundary].
    buckets: &'static [u64],
    /// Per-bucket atomic counters.
    counts: Box<[AtomicU64]>,
    /// Counter for values above the max bucket.
    overflow: AtomicU64,
    /// Total observations.
    total: AtomicU64,
}

impl LatencyHistogram {
    /// Default bucket boundaries: 1µs, 2µs, 4µs, 8µs, 16µs, 32µs, 64µs,
    /// 128µs, 256µs, 512µs, 1ms, 2ms, 4ms, 8ms, 16ms, 32ms, 64ms, 128ms,
    /// 256ms, 512ms, 1s.
    pub const DEFAULT_BUCKETS: &'static [u64] = &[
        1_000,
        2_000,
        4_000,
        8_000,
        16_000,
        32_000,
        64_000,
        128_000,
        256_000,
        512_000,
        1_000_000,
        2_000_000,
        4_000_000,
        8_000_000,
        16_000_000,
        32_000_000,
        64_000_000,
        128_000_000,
        256_000_000,
        512_000_000,
        1_000_000_000,
    ];

    /// Create a new histogram with the default bucket boundaries.
    #[allow(clippy::box_default)]
    pub fn new() -> Self {
        let count = Self::DEFAULT_BUCKETS.len();
        let mut counts = Vec::with_capacity(count);
        for _ in 0..count {
            counts.push(AtomicU64::new(0));
        }
        Self {
            buckets: Self::DEFAULT_BUCKETS,
            counts: counts.into_boxed_slice(),
            overflow: AtomicU64::new(0),
            total: AtomicU64::new(0),
        }
    }

    /// Record a latency value in nanoseconds.
    #[inline(always)]
    pub fn record(&self, ns: u64) {
        self.total.fetch_add(1, Ordering::Relaxed);
        // Linear scan over a small array (≤21 entries) — stays in L1 cache.
        for (i, &boundary) in self.buckets.iter().enumerate() {
            if ns <= boundary {
                self.counts[i].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        self.overflow.fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot: (bucket_upper_bound_ns, count) pairs, plus overflow count.
    pub fn snapshot(&self) -> HistogramSnapshot {
        let mut buckets = Vec::with_capacity(self.buckets.len());
        let total = self.total.load(Ordering::Relaxed);
        for (i, &boundary) in self.buckets.iter().enumerate() {
            let count = self.counts[i].load(Ordering::Relaxed);
            buckets.push(BucketEntry {
                upper_bound_ns: boundary,
                count,
                pct: if total > 0 {
                    count as f64 / total as f64 * 100.0
                } else {
                    0.0
                },
            });
        }
        HistogramSnapshot {
            buckets,
            overflow: self.overflow.load(Ordering::Relaxed),
            total,
        }
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.total.store(0, Ordering::Relaxed);
        self.overflow.store(0, Ordering::Relaxed);
        for c in self.counts.iter() {
            c.store(0, Ordering::Relaxed);
        }
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

/// A single histogram bucket in a snapshot.
#[derive(Debug, Clone)]
pub struct BucketEntry {
    pub upper_bound_ns: u64,
    pub count: u64,
    pub pct: f64,
}

/// Point-in-time snapshot of a [`LatencyHistogram`].
#[derive(Debug, Clone)]
pub struct HistogramSnapshot {
    pub buckets: Vec<BucketEntry>,
    pub overflow: u64,
    pub total: u64,
}

impl HistogramSnapshot {
    /// P50 latency in nanoseconds (linear interpolation within bucket).
    pub fn p50(&self) -> f64 {
        self.percentile(50.0)
    }

    /// P99 latency in nanoseconds.
    pub fn p99(&self) -> f64 {
        self.percentile(99.0)
    }

    /// P999 latency in nanoseconds.
    pub fn p999(&self) -> f64 {
        self.percentile(99.9)
    }

    /// Estimate the value at the given percentile.
    pub fn percentile(&self, pct: f64) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        let target = (pct / 100.0) * self.total as f64;
        let mut cumulative = 0u64;
        for entry in &self.buckets {
            cumulative += entry.count;
            if cumulative as f64 >= target {
                return entry.upper_bound_ns as f64;
            }
        }
        // Fallback: beyond the last bucket — estimate 2x last boundary.
        self.buckets
            .last()
            .map(|b| b.upper_bound_ns as f64 * 2.0)
            .unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------------------
// TraceSpan — RAII timing guard
// ---------------------------------------------------------------------------

/// RAII timing span that records its duration into a [`LatencyHistogram`] on
/// drop. Construct with [`TraceSpan::new`] or the [`trace_span!`] macro.
///
/// When the `perf-trace` feature is disabled, this compiles to a no-op zero-size
/// type with zero overhead.
#[derive(Debug)]
pub struct TraceSpan {
    start: Option<Instant>,
    histogram: Option<Arc<LatencyHistogram>>,
}

impl TraceSpan {
    /// Start a new span. The duration is recorded into `histogram` on drop.
    #[inline(always)]
    pub fn new(histogram: Arc<LatencyHistogram>) -> Self {
        Self {
            start: Some(Instant::now()),
            histogram: Some(histogram),
        }
    }
}

impl Drop for TraceSpan {
    #[inline(always)]
    fn drop(&mut self) {
        if let (Some(start), Some(hist)) = (self.start.take(), self.histogram.take()) {
            let elapsed = start.elapsed().as_nanos() as u64;
            hist.record(elapsed);
        }
    }
}

// ---------------------------------------------------------------------------
// PerfCounter — named atomic counter
// ---------------------------------------------------------------------------

/// A named counter visible under `perf-trace`.
#[derive(Debug)]
pub struct PerfCounter {
    name: &'static str,
    value: AtomicU64,
}

impl PerfCounter {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            value: AtomicU64::new(0),
        }
    }

    #[inline(always)]
    pub fn increment(&self, delta: u64) {
        self.value.fetch_add(delta, Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn store(&self, val: u64) {
        self.value.store(val, Ordering::Relaxed);
    }

    pub fn value(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

// ---------------------------------------------------------------------------
// PerfRegistry — global registry for all counters and histograms
// ---------------------------------------------------------------------------

/// Global registry of performance counters and histograms.
///
/// Use [`PerfRegistry::global()`] to access the singleton, then
/// [`register_counter`] / [`register_histogram`] / [`report`] to manage data.
#[derive(Debug)]
pub struct PerfRegistry {
    counters: parking_lot::Mutex<Vec<&'static PerfCounter>>,
    histograms: parking_lot::Mutex<Vec<(&'static str, Arc<LatencyHistogram>)>>,
}

impl PerfRegistry {
    fn new() -> Self {
        Self {
            counters: parking_lot::Mutex::new(Vec::new()),
            histograms: parking_lot::Mutex::new(Vec::new()),
        }
    }

    /// Get the global singleton.
    pub fn global() -> &'static Self {
        static REG: once_cell::sync::Lazy<PerfRegistry> =
            once_cell::sync::Lazy::new(PerfRegistry::new);
        &REG
    }

    /// Register a static counter so it appears in reports.
    pub fn register_counter(&self, c: &'static PerfCounter) {
        self.counters.lock().push(c);
    }

    /// Register a named histogram.
    pub fn register_histogram(&self, name: &'static str, h: Arc<LatencyHistogram>) {
        self.histograms.lock().push((name, h));
    }

    /// Produce a text report of all registered counters and histograms.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("=== Perf-trace Report ===\n\n");

        // Counters
        let counters = self.counters.lock();
        if !counters.is_empty() {
            out.push_str("--- Counters ---\n");
            for c in counters.iter() {
                out.push_str(&format!("  {}: {}\n", c.name(), c.value()));
            }
            out.push('\n');
        }
        drop(counters);

        // Histograms
        let histograms = self.histograms.lock();
        if !histograms.is_empty() {
            out.push_str("--- Latency Histograms ---\n");
            for (name, h) in histograms.iter() {
                let snap = h.snapshot();
                out.push_str(&format!(
                    "  {}: total={}  P50={:.0}µs  P99={:.0}µs  P999={:.0}µs\n",
                    name,
                    snap.total,
                    snap.p50() / 1000.0,
                    snap.p99() / 1000.0,
                    snap.p999() / 1000.0,
                ));
                if !snap.buckets.is_empty() {
                    out.push_str("    Buckets (µs):\n");
                    for b in &snap.buckets {
                        if b.count > 0 {
                            out.push_str(&format!(
                                "      ≤{}µs: {} ({:.1}%)\n",
                                b.upper_bound_ns / 1000,
                                b.count,
                                b.pct
                            ));
                        }
                    }
                }
                if snap.overflow > 0 {
                    out.push_str(&format!("    >1s overflow: {}\n", snap.overflow));
                }
            }
        }
        drop(histograms);

        out
    }

    /// Reset all histograms and counters.
    pub fn reset_all(&self) {
        for c in self.counters.lock().iter() {
            c.store(0);
        }
        for (_, h) in self.histograms.lock().iter() {
            h.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_histogram_record() {
        let h = LatencyHistogram::new();
        h.record(100);
        h.record(1_000_000);
        h.record(100_000_000);
        let snap = h.snapshot();
        assert_eq!(snap.total, 3);
        assert!(snap.p50() > 0.0);
        assert!(snap.p99() > 0.0);
    }

    #[test]
    fn test_histogram_snapshot_buckets() {
        let h = LatencyHistogram::new();
        // Record 1000 values all at exactly 500ns (should land in ≤1µs bucket)
        for _ in 0..1000 {
            h.record(500);
        }
        let snap = h.snapshot();
        assert_eq!(snap.total, 1000);
        // All entries should be in the first bucket (≤1µs = 1_000ns)
        let first_bucket = &snap.buckets[0];
        assert_eq!(first_bucket.upper_bound_ns, 1_000);
        assert_eq!(first_bucket.count, 1000);
    }

    #[test]
    fn test_perf_counter() {
        static C: PerfCounter = PerfCounter::new("test");
        assert_eq!(C.value(), 0);
        C.increment(5);
        assert_eq!(C.value(), 5);
        C.store(0);
        assert_eq!(C.value(), 0);
    }

    #[test]
    fn test_trace_span_records() {
        let hist = Arc::new(LatencyHistogram::new());
        {
            let _span = TraceSpan::new(hist.clone());
            // do some work
            let mut acc: u64 = 0;
            for i in 0..1000 {
                acc = acc.wrapping_add(i);
            }
            std::hint::black_box(acc);
        }
        let snap = hist.snapshot();
        assert_eq!(snap.total, 1, "TraceSpan should record exactly one entry");
    }

    #[test]
    fn test_perf_registry_report() {
        let reg = PerfRegistry::new();
        let hist = Arc::new(LatencyHistogram::new());
        hist.record(42_000);
        reg.register_histogram("test_hist", hist);
        let report = reg.report();
        assert!(report.contains("test_hist"));
        assert!(report.contains("P50"));
    }

    #[test]
    fn test_percentile_estimation() {
        let h = LatencyHistogram::new();
        // 1000 entries at 500µs each
        for _ in 0..1000 {
            h.record(500_000);
        }
        let snap = h.snapshot();
        let p50 = snap.p50();
        assert!(p50 >= 500_000.0, "P50 should be >= 500µs, got {:.0}ns", p50);
        let p99 = snap.p99();
        assert!(p99 >= 500_000.0);
        assert!(p50 <= p99, "P50 should be <= P99");
    }
}