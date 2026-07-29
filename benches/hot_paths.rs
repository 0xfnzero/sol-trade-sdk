//! Hot-path benchmarks for sol-trade-sdk.
//!
//! Run: `cargo bench --features dev-insecure-tls` (add `--features perf-trace` for tracing).
//! Filter: `cargo bench --features dev-insecure-tls -- latency`

use divan::{black_box, Bencher};

fn main() {
    divan::main();
}

// ---------------------------------------------------------------------------
// LatencyHistogram benchmarks
// ---------------------------------------------------------------------------

mod latency_histogram {
    use super::*;
    use sol_trade_sdk::perf::LatencyHistogram;

    #[divan::bench]
    fn record_100ns(b: Bencher) {
        let h = LatencyHistogram::new();
        b.bench(|| h.record(100));
    }

    #[divan::bench]
    fn record_1ms(b: Bencher) {
        let h = LatencyHistogram::new();
        b.bench(|| h.record(1_000_000));
    }

    #[divan::bench]
    fn record_100ms(b: Bencher) {
        let h = LatencyHistogram::new();
        b.bench(|| h.record(100_000_000));
    }

    #[divan::bench]
    fn snapshot(b: Bencher) {
        let h = LatencyHistogram::new();
        for i in 0..10_000 {
            h.record((i as u64) * 1000);
        }
        b.bench(|| h.snapshot());
    }

    #[divan::bench]
    fn p50_estimation(b: Bencher) {
        let h = LatencyHistogram::new();
        for i in 0..10_000 {
            h.record((i as u64) * 1000);
        }
        let snap = h.snapshot();
        b.bench(|| snap.p50());
    }
}

// ---------------------------------------------------------------------------
// TraceSpan benchmarks
// ---------------------------------------------------------------------------

mod trace_span {
    use super::*;
    use sol_trade_sdk::perf::{LatencyHistogram, TraceSpan};
    use std::sync::Arc;

    #[divan::bench]
    fn span_overhead(b: Bencher) {
        let hist = Arc::new(LatencyHistogram::new());
        b.bench(|| {
            let _span = TraceSpan::new(hist.clone());
            std::hint::black_box(());
        });
    }

    #[divan::bench]
    fn span_with_work(b: Bencher) {
        let hist = Arc::new(LatencyHistogram::new());
        b.bench(|| {
            let _span = TraceSpan::new(hist.clone());
            let mut acc: u64 = 0;
            for i in 0..100 {
                acc = acc.wrapping_add(i);
            }
            std::hint::black_box(acc);
        });
    }
}

// ---------------------------------------------------------------------------
// PerfCounter benchmarks
// ---------------------------------------------------------------------------

mod perf_counter {
    use super::*;
    use sol_trade_sdk::perf::PerfCounter;

    static COUNTER: PerfCounter = PerfCounter::new("test_counter");

    #[divan::bench]
    fn increment(b: Bencher) {
        b.bench(|| COUNTER.increment(1));
    }

    #[divan::bench]
    fn read_value(b: Bencher) {
        COUNTER.store(42);
        b.bench(|| COUNTER.value());
    }
}

// ---------------------------------------------------------------------------
// Instruction building benchmarks (mock)
// ---------------------------------------------------------------------------

mod instruction_building {
    use super::*;
    use solana_sdk::instruction::Instruction;
    use solana_sdk::pubkey::Pubkey;
    use solana_system_interface::instruction as system_instruction;

    fn mock_instructions(count: usize) -> Vec<Instruction> {
        let from = Pubkey::new_unique();
        let to = Pubkey::new_unique();
        (0..count)
            .map(|i| system_instruction::transfer(&from, &to, (i as u64 + 1) * 1000))
            .collect()
    }

    #[divan::bench]
    fn build_5_instructions(b: Bencher) {
        b.bench(|| black_box(mock_instructions(5)));
    }

    #[divan::bench]
    fn build_20_instructions(b: Bencher) {
        b.bench(|| black_box(mock_instructions(20)));
    }

    #[divan::bench]
    fn serialize_5_instructions(b: Bencher) {
        let instructions = mock_instructions(5);
        b.bench(|| {
            let _ = bincode::serialize(black_box(&instructions));
        });
    }

    #[divan::bench]
    fn serialize_20_instructions(b: Bencher) {
        let instructions = mock_instructions(20);
        b.bench(|| {
            let _ = bincode::serialize(black_box(&instructions));
        });
    }
}

// ---------------------------------------------------------------------------
// Pubkey operations benchmarks
// ---------------------------------------------------------------------------

mod pubkey_ops {
    use super::*;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    static PUBKEY_STR: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

    #[divan::bench]
    fn parse_from_str(b: Bencher) {
        b.bench(|| Pubkey::from_str(black_box(PUBKEY_STR)).unwrap());
    }

    #[divan::bench]
    fn to_string(b: Bencher) {
        let pk = Pubkey::from_str(PUBKEY_STR).unwrap();
        b.bench(|| black_box(pk.to_string()));
    }

    #[divan::bench]
    fn clone(b: Bencher) {
        let pk = Pubkey::from_str(PUBKEY_STR).unwrap();
        b.bench(|| black_box(pk.clone()));
    }
}

// ---------------------------------------------------------------------------
// Hash operations benchmarks
// ---------------------------------------------------------------------------

mod hash_ops {
    use super::*;
    use solana_sdk::hash::Hash;
    use std::str::FromStr;

    #[divan::bench]
    fn new_unique(b: Bencher) {
        b.bench(|| Hash::new_unique());
    }

    #[divan::bench]
    fn hash_from_str(b: Bencher) {
        let s = "7aH9HbiqHoL1KPbGQJ2VqWjQqKzoKz8KQVbQqQbQqQbQ";
        b.bench(|| Hash::from_str(black_box(s)));
    }
}

// ---------------------------------------------------------------------------
// Byte serialization / deserialization benchmarks
// ---------------------------------------------------------------------------

mod serialization {
    use super::*;

    #[divan::bench]
    fn bincode_roundtrip_u64(b: Bencher) {
        let val = 42u64;
        b.bench(|| {
            let bytes = bincode::serialize(&black_box(val)).unwrap();
            let _decoded: u64 = bincode::deserialize(&bytes).unwrap();
        });
    }

    #[divan::bench]
    fn bincode_roundtrip_256bytes(b: Bencher) {
        let data: Vec<u8> = (0..255).collect();
        b.bench(|| {
            let bytes = bincode::serialize(black_box(&data)).unwrap();
            let _decoded: Vec<u8> = bincode::deserialize(&bytes).unwrap();
        });
    }

    #[divan::bench]
    fn base58_encode_32bytes(b: Bencher) {
        let data = [0u8; 32];
        b.bench(|| bs58::encode(black_box(&data)).into_string());
    }

    #[divan::bench]
    fn base58_decode_32bytes(b: Bencher) {
        let encoded = "7aH9HbiqHoL1KPbGQJ2VqWjQqKzoKz8KQVbQqQbQqQbQ";
        b.bench(|| bs58::decode(black_box(encoded)).into_vec().unwrap());
    }
}