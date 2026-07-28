//! Reconstruction thread monitoring.
//!
//! The ShredStream SDK handles slot assembly and FEC internally. This module
//! provides a monitoring thread that reads SDK counters and converts them to
//! metrics. It also provides a helper to spawn the reconstruction/classification
//! thread that reads from the raw packet queue and feeds the classifier.

use crossbeam_channel::{Receiver, Sender};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::perf::shredstream::classifier::{ClassifiedEvent, EventClassifier};
use crate::perf::shredstream::config::ShredstreamConfig;
use crate::perf::shredstream::metrics::{MetricsSnapshot, ShredstreamMetrics};
use crate::perf::shredstream::receive_loop::SlotTransactionBatch;

/// Spawn a reconstruction + classification thread.
///
/// This thread reads from the `raw_packets` channel (produced by the receive loop),
/// passes each batch through the `EventClassifier`, and pushes classified events
/// onto the `classified_events` channel.
///
/// CPU pinning is applied when configured.
pub fn spawn_reconstruct_thread(
    config: &ShredstreamConfig,
    raw_packet_rx: Receiver<SlotTransactionBatch>,
    classified_tx: Sender<ClassifiedEvent>,
    metrics: Arc<ShredstreamMetrics>,
) -> JoinHandle<()> {
    // Build the classifier once; it owns the dedup cache and program filter
    let mut classifier = EventClassifier::new(
        &config.allowed_programs,
        config.filter_votes,
        crate::perf::shredstream::dedup::DedupCache::new(config.dedup_cache_size),
        Arc::clone(&metrics),
    );
    let reconstruct_core = config.reconstruct_core;

    thread::Builder::new()
        .name("shredstream-reconstruct".into())
        .spawn(move || {
            // Pin to dedicated CPU core if configured
            if let Some(core_id) = reconstruct_core {
                let _ =
                    core_affinity::set_for_current(core_affinity::CoreId { id: core_id });
            }

            tracing::info!("ShredStream reconstruction/classification thread started");

            for batch in raw_packet_rx.iter() {
                // Process the slot through the classifier
                let classified = classifier.process_slot(
                    batch.slot,
                    &batch.transactions,
                    batch.received_at_micros,
                );

                // Update slot-level metrics from SDK counter proxy
                if !classified.is_empty() {
                    metrics.slots_completed.fetch_add(1, Ordering::Relaxed);
                    metrics
                        .transactions_decoded
                        .fetch_add(batch.transactions.len() as u64, Ordering::Relaxed);
                }

                // Push each classified event onto the next queue
                for event in classified {
                    if classified_tx.try_send(event).is_err() {
                        metrics
                            .queue_drops_classified_events
                            .fetch_add(1, Ordering::Relaxed);
                        tracing::warn!("Classified event queue full — dropping event");
                    }
                }

                // Update queue depth metrics
                metrics
                    .queue_depth_raw_packets
                    .store(raw_packet_rx.len() as u64, Ordering::Relaxed);
            }

            tracing::info!("ShredStream reconstruction/classification thread exiting");
        })
        .expect("Failed to spawn reconstruction thread")
}

/// Polling interval for SDK metrics counters (milliseconds).
const METRICS_POLL_MS: u64 = 5_000;

/// Spawn a background metrics monitoring thread.
///
/// This thread periodically reads SDK-level counters and updates the
/// `ShredstreamMetrics` atomics. This is separate from hot-path counting
/// (which happens inline in receive_loop and classifier).
pub fn spawn_metrics_monitor(metrics: Arc<ShredstreamMetrics>) -> JoinHandle<()> {
    thread::Builder::new()
        .name("shredstream-metrics".into())
        .spawn(move || loop {
            thread::sleep(Duration::from_millis(METRICS_POLL_MS));

            // Snapshot current metrics for periodic export
            let snapshot = MetricsSnapshot::from(metrics.as_ref());

            // Log a structured summary at debug level
            tracing::debug!(
                packets = snapshot.packets_received,
                slots = snapshot.slots_completed,
                dedup_hits = snapshot.dedup_hits,
                dedup_misses = snapshot.dedup_misses,
                drops_raw = snapshot.queue_drops_raw_packets,
                drops_classified = snapshot.queue_drops_classified_events,
                "ShredStream metrics tick"
            );
        })
        .expect("Failed to spawn metrics monitor")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;

    #[test]
    fn test_metrics_snapshot_default() {
        let snap = MetricsSnapshot::default();
        assert_eq!(snap.packets_received, 0);
        assert_eq!(snap.slots_completed, 0);
    }
}