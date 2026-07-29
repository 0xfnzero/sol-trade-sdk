//! Shadow mode — live feed, no submission, outcome comparison.
//!
//! Phase 11 of the productionization roadmap: connect to live ShredStream data,
//! run the full evaluation pipeline, generate hypothetical trade decisions,
//! and track accuracy against actual market movement — all without submitting
//! any transactions. No wallet keypair required.
//!
//! # Architecture
//!
//! ```text
//! ShredstreamAdapter ──classified events──→ ShadowEngine
//!                                              │
//!                                              ├─ evaluate() → ShadowDecision
//!                                              ├─ DecisionBuffer (ring buffer, last 10K)
//!                                              └─ AccuracyChecker (every 5s)
//!                                                   └─ compares decision price vs current price
//! ```
//!
//! # Usage
//!
//! Run the `solshadow` binary:
//! ```text
//! solshadow --config /etc/solbot/config.toml
//!          [--metrics 127.0.0.1:9090] [--health 127.0.0.1:9091]
//!          [--shadow-decisions 127.0.0.1:9092]
//! ```
//!
//! # HTTP Endpoints
//!
//! - `GET /health` — build info
//! - `GET /metrics` — Prometheus-format counters from PerfRegistry
//! - `GET /shadow/decisions?n=20` — last N shadow decisions as JSON
//! - `GET /shadow/accuracy` — aggregate accuracy stats
//! - `GET /shadow/stats` — summary counters

mod accuracy;
mod decision;
mod engine;

pub use accuracy::{AccuracyChecker, AccuracySnapshot, HorizonStats};
pub use decision::{DecisionBuffer, ShadowDecision, ShadowSignal};
pub use engine::ShadowEngine;