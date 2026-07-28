//! ShredStreamAdapter — lifecycle controller for the receive pipeline.
//!
//! Owns the receive, reconstruction, and metrics monitoring threads.
//! Provides `start()`, `stop()`, and `restart()` lifecycle methods.
//! Exposes classified events via a `Receiver<ClassifiedEvent>` for downstream
//! state and strategy threads.

use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::perf::shredstream::classifier::ClassifiedEvent;
use crate::perf::shredstream::config::ShredstreamConfig;
use crate::perf::shredstream::metrics::ShredstreamMetrics;
use crate::perf::shredstream::receive_loop::{self, SlotTransactionBatch};
use crate::perf::shredstream::reconstruction;

/// The main ShredStream adapter.
///
/// Usage:
/// ```ignore
/// let config = ShredstreamConfig::default();
/// let adapter = ShredstreamAdapter::new(config);
/// adapter.start()?;
/// // consume events from adapter.classified_event_rx()
/// // adapter.stop();
/// ```
pub struct ShredstreamAdapter {
    config: ShredstreamConfig,
    metrics: Arc<ShredstreamMetrics>,
    running: Arc<AtomicBool>,

    // Channel handles
    raw_packet_tx: Option<Sender<SlotTransactionBatch>>,
    classified_event_tx: Sender<ClassifiedEvent>,
    classified_event_rx: Receiver<ClassifiedEvent>,

    // Thread join handles
    receive_handle: Option<JoinHandle<()>>,
    reconstruct_handle: Option<JoinHandle<()>>,
    metrics_handle: Option<JoinHandle<()>>,
}

impl ShredstreamAdapter {
    /// Create a new adapter with the given config. Does not start the pipeline.
    pub fn new(config: ShredstreamConfig) -> Self {
        let (classified_tx, classified_rx) = bounded(config.classified_event_queue_capacity);

        Self {
            config,
            metrics: ShredstreamMetrics::new_arc(),
            running: Arc::new(AtomicBool::new(false)),
            raw_packet_tx: None,
            classified_event_tx: classified_tx,
            classified_event_rx: classified_rx,
            receive_handle: None,
            reconstruct_handle: None,
            metrics_handle: None,
        }
    }

    /// Start the pipeline: creates channels, spawns threads.
    ///
    /// Returns an error if the ShredStream listener cannot bind.
    pub fn start(&mut self) -> std::io::Result<()> {
        if self.running.load(Ordering::SeqCst) {
            tracing::warn!("ShredStreamAdapter already running");
            return Ok(());
        }

        // Create the raw packet channel (bounded SPSC)
        let (raw_tx, raw_rx) = bounded(self.config.raw_packet_queue_capacity);
        self.raw_packet_tx = Some(raw_tx);

        // Clone the classified event sender for the reconstruction thread
        let classified_tx = self.classified_event_tx.clone();
        let metrics = Arc::clone(&self.metrics);
        let _running = Arc::clone(&self.running);

        // Spawn metrics monitor first (low-risk)
        let metrics_handle = reconstruction::spawn_metrics_monitor(Arc::clone(&metrics));

        // Spawn reconstruction/classification thread
        let reconstruct_handle = reconstruction::spawn_reconstruct_thread(
            &self.config,
            raw_rx,
            classified_tx,
            Arc::clone(&metrics),
        );

        // Spawn receive loop (can fail if port is in use)
        let receive_handle = receive_loop::spawn_receive_loop(
            &self.config,
            self.raw_packet_tx.as_ref().unwrap().clone(),
            Arc::clone(&metrics),
        )?;

        self.receive_handle = Some(receive_handle);
        self.reconstruct_handle = Some(reconstruct_handle);
        self.metrics_handle = Some(metrics_handle);
        self.running.store(true, Ordering::SeqCst);

        tracing::info!("ShredStreamAdapter started successfully");
        Ok(())
    }

    /// Stop the pipeline gracefully.
    ///
    /// Drops the sender handle so the receive loop exits, then joins threads.
    pub fn stop(&mut self) {
        if !self.running.load(Ordering::SeqCst) {
            return;
        }

        tracing::info!("ShredStreamAdapter stopping...");

        // Drop sender to signal receive loop to exit
        self.raw_packet_tx.take();

        // Join threads with a timeout
        if let Some(handle) = self.receive_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.reconstruct_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.metrics_handle.take() {
            let _ = handle.join();
        }

        self.running.store(false, Ordering::SeqCst);
        tracing::info!("ShredStreamAdapter stopped");
    }

    /// Restart the pipeline (stop + start).
    pub fn restart(&mut self) -> std::io::Result<()> {
        self.stop();
        self.start()
    }

    // ── Accessors ──

    /// Returns a reference to the metrics.
    pub fn metrics(&self) -> &ShredstreamMetrics {
        &self.metrics
    }

    /// Returns a reference to the classified event receiver.
    ///
    /// This is the downstream interface: the state & strategy thread reads from
    /// this channel.
    pub fn classified_event_rx(&self) -> &Receiver<ClassifiedEvent> {
        &self.classified_event_rx
    }

    /// Returns whether the pipeline is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Returns a metrics snapshot.
    pub fn metrics_snapshot(&self) -> crate::perf::shredstream::metrics::MetricsSnapshot {
        crate::perf::shredstream::metrics::MetricsSnapshot::from(self.metrics.as_ref())
    }
}

impl Drop for ShredstreamAdapter {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_initial_state() {
        let adapter = ShredstreamAdapter::new(ShredstreamConfig::default());
        assert!(!adapter.is_running());
        assert!(adapter.classified_event_rx().is_empty()); // no events yet
    }

    #[test]
    fn test_adapter_start_stop() {
        // Verify stop is safe even when not started
        let mut adapter = ShredstreamAdapter::new(ShredstreamConfig::default());
        adapter.stop();
        assert!(!adapter.is_running());
    }
}