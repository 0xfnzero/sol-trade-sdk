//! ShredStream: UDP receive pipeline for ultra-low-latency Solana transaction streaming.
//!
//! This module implements the ShredStream receive pipeline as described in
//! `docs/shredstream-architecture.md`. It is organized into:
//!
//! - `config` — runtime configuration matching `ListenerOptions`
//! - `metrics` — lock-free atomic counters
//! - `dedup` — bounded LRU deduplication cache
//! - `classifier` — program ID filter + instruction decode + event type
//! - `receive_loop` — raw UDP receive thread (hot path, dedicated `std::thread`)
//! - `reconstruction` — slot assembly monitoring + classifier bridge
//! - `adapter` — `ShredstreamAdapter` lifecycle controller

pub mod adapter;
pub mod classifier;
pub mod config;
pub mod dedup;
pub mod metrics;
pub mod receive_loop;
pub mod reconstruction;

pub use adapter::ShredstreamAdapter;
pub use classifier::{ClassifiedEvent, EventClassifier, EventTrace, EventType};
pub use config::ShredstreamConfig;
pub use dedup::{DedupCache, DedupKey};
pub use metrics::{MetricsSnapshot, ShredstreamMetrics};
pub use receive_loop::SlotTransactionBatch;