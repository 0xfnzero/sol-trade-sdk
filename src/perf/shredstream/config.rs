//! Runtime configuration for the ShredStream receive pipeline.
//!
//! This config mirrors the design in `docs/shredstream-architecture.md` section 14.1.

use std::num::NonZeroUsize;

/// Top-level configuration for the ShredStream pipeline.
#[derive(Debug, Clone)]
pub struct ShredstreamConfig {
    // ── Network ──
    pub port: u16,
    pub bind_address: String,

    // ── SDK listener options ──
    pub recv_buf: usize,
    pub max_age: u64,
    pub busy_poll_us: Option<u32>,
    pub pool_size: usize,
    pub enable_fec: bool,
    pub disable_salvage_delivery: bool,
    pub stuck_batch_timeout_ms: u64,

    // ── CPU affinity ──
    pub receive_core: Option<usize>,
    pub reconstruct_core: Option<usize>,

    // ── Filter ──
    pub allowed_programs: Vec<String>,
    pub filter_votes: bool,

    // ── Queue sizing ──
    pub raw_packet_queue_capacity: usize,
    pub decoded_slot_queue_capacity: usize,
    pub classified_event_queue_capacity: usize,
    pub trade_intent_queue_capacity: usize,

    // ── Dedup ──
    pub dedup_cache_size: NonZeroUsize,

    // ── Degraded / stop thresholds ──
    pub degraded_queue_pct: f64,
    pub degraded_duration_secs: u64,
    pub stop_trading_queue_pct: f64,
    pub stop_trading_duration_msecs: u64,
}

impl Default for ShredstreamConfig {
    fn default() -> Self {
        Self {
            port: 8001,
            bind_address: "0.0.0.0".into(),
            recv_buf: 64 * 1024 * 1024, // 64 MiB
            max_age: 3,
            busy_poll_us: Some(200),
            pool_size: 4096,
            enable_fec: true,
            disable_salvage_delivery: false,
            stuck_batch_timeout_ms: 50,
            receive_core: Some(0),
            reconstruct_core: Some(1),
            allowed_programs: vec![
                "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".into(), // Raydium AMM V4
                "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C".into(), // Raydium CPMM
                "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".into(), // PumpFun
                "pAMMBay6oceH9fJKBRHqp5k4tNcS7sMq3Z3zCbPxWC".into(), // PumpSwap
                "LBUZKhRxPF3XUp4mMfH3XYGJmTVC9dFGjH1VxN3Fg".into(), // Meteora DLMM
            ],
            filter_votes: true,
            raw_packet_queue_capacity: 32_768,
            decoded_slot_queue_capacity: 1_024,
            classified_event_queue_capacity: 8_192,
            trade_intent_queue_capacity: 256,
            dedup_cache_size: NonZeroUsize::new(262_144).unwrap(),
            degraded_queue_pct: 0.75,
            degraded_duration_secs: 5,
            stop_trading_queue_pct: 0.75,
            stop_trading_duration_msecs: 500,
        }
    }
}