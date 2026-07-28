//! Raw UDP receive loop for ShredStream.
//!
//! This is the hottest path in the pipeline. It runs in a dedicated `std::thread`,
//! pinned to a dedicated CPU core. It does only: `recvfrom()`, timestamp, validate,
//! and bounded enqueue. No parsing, no filtering, no allocation beyond the buffer pool.

use crossbeam_channel::Sender;
use shredstream::{ListenerOptions, ShredListener};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::common::fast_timing;
use crate::perf::shredstream::config::ShredstreamConfig;
use crate::perf::shredstream::metrics::ShredstreamMetrics;

/// A batch of decoded transactions from a completed slot, passed from the
/// receive thread to the reconstruction / classification thread.
#[derive(Debug, Clone)]
pub struct SlotTransactionBatch {
    pub slot: u64,
    pub transactions: Vec<solana_sdk::transaction::VersionedTransaction>,
    pub received_at_micros: i64,
}

/// Spawn the ShredStream receive loop in a dedicated `std::thread`.
///
/// Binds the ShredListener on the configured port, enters a tight iterator over
/// `listener.transactions()`, and pushes each completed slot batch onto the
/// bounded SPSC channel.
pub fn spawn_receive_loop(
    config: &ShredstreamConfig,
    tx: Sender<SlotTransactionBatch>,
    metrics: Arc<ShredstreamMetrics>,
) -> std::io::Result<JoinHandle<()>> {
    let port = config.port;
    let recv_buf = config.recv_buf;
    let max_age = config.max_age;
    let busy_poll_us = config.busy_poll_us;
    let pool_size = config.pool_size;
    let enable_fec = config.enable_fec;
    let disable_salvage_delivery = config.disable_salvage_delivery;
    let stuck_batch_timeout_ms = config.stuck_batch_timeout_ms;
    let receive_core = config.receive_core;

    let handle = thread::Builder::new()
        .name("shredstream-receive".into())
        .spawn(move || {
            // Pin thread to dedicated CPU core if configured
            if let Some(core_id) = receive_core {
                let _ = core_affinity::set_for_current(core_affinity::CoreId { id: core_id });
            }

            // Build listener options
            let opts = ListenerOptions {
                recv_buf,
                max_age,
                busy_poll_us,
                pool_size,
                enable_fec,
                disable_salvage_delivery,
                accumulator: shredstream::AccumulatorConfig {
                    max_fec_sets_per_slot: 32,
                    stuck_batch_timeout: Duration::from_millis(stuck_batch_timeout_ms),
                },
            };

            // Bind listener (takes port as u16)
            let mut listener = match ShredListener::bind_with_options(port, opts) {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!(port, error = %e, "Failed to bind ShredStream listener");
                    return;
                }
            };

            tracing::info!(port, recv_buf, max_age, "ShredStream receive loop started");

            // Enter tight receive loop — SDK yields (slot, Vec<VersionedTransaction>) directly
            for (slot, transactions) in listener.transactions() {
                // Stage-1 timestamp immediately on receipt
                let received_at = fast_timing::fast_now_micros() as i64;

                // Count received
                metrics.packets_received.fetch_add(1, Ordering::Relaxed);
                let bytes: u64 = transactions
                    .iter()
                    .map(|t| {
                        // Estimate serialized size: message bytes + signatures overhead
                        let msg_bytes = bincode::serialize(&t.message)
                            .map(|v| v.len() as u64)
                            .unwrap_or(0);
                        msg_bytes + (t.signatures.len() as u64 * 64)
                    })
                    .sum();
                metrics.bytes_received.fetch_add(bytes, Ordering::Relaxed);

                let batch = SlotTransactionBatch {
                    slot,
                    transactions,
                    received_at_micros: received_at,
                };

                // Bounded enqueue — increment drop counter on full
                if tx.try_send(batch).is_err() {
                    metrics.queue_drops_raw_packets.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!("ShredStream raw packet queue full — dropping oldest batch");
                }
            }

            tracing::info!("ShredStream receive loop exiting");
        })?;

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_batch_creation() {
        let batch = SlotTransactionBatch {
            slot: 42,
            transactions: vec![],
            received_at_micros: 1234567890,
        };
        assert_eq!(batch.slot, 42);
        assert!(batch.transactions.is_empty());
        assert_eq!(batch.received_at_micros, 1234567890);
    }
}