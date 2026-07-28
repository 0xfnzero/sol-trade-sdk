# ShredStream Integration Architecture

> Design document for integrating ShredStream UDP shred reception into sol-trade-sdk.
> Target runtime: Ubuntu Linux VPS | Primary language: Rust
> ShredStream SDK reference commit: `274db4909bf597783541989305a67046bc4eed33`

---

## Table of Contents

1. [Overview](#1-overview)
2. [Architecture: Receive Pipeline](#2-architecture-receive-pipeline)
3. [ShredStream Receive Loop Spec](#3-shredstream-receive-loop-spec)
4. [Socket Configuration (Ubuntu VPS)](#4-socket-configuration-ubuntu-vps)
5. [Reconstruction Policy](#5-reconstruction-policy)
6. [Filtering Policy](#6-filtering-policy)
7. [Deduplication Strategy](#7-deduplication-strategy)
8. [Bounded Queue Policy](#8-bounded-queue-policy)
9. [Observability](#9-observability)
10. [Hot-Path Contract](#10-hot-path-contract)
11. [Integration Points with Existing SDK](#11-integration-points-with-existing-sdk)
12. [Threading and CPU Allocation](#12-threading-and-cpu-allocation)
13. [Kernel Bypass Evaluation](#13-kernel-bypass-evaluation)
14. [ShredStream SDK Adapter Design](#14-shredstream-sdk-adapter-design)
15. [LaserStream Fallback Strategy](#15-laserstream-fallback-strategy)

---

## 1. Overview

### 1.1 Why ShredStream

ShredStream delivers raw Solana shreds over UDP **hundreds of milliseconds before** the same data is available through standard RPC or Geyser gRPC endpoints. For HFT, MEV, and sniping workloads, this time advantage is the difference between capturing and missing an opportunity.

The SDK (`shredstream` v2.0 on crates.io) is a thin Rust layer over the wire that handles:
- UDP socket creation and buffer tuning (64 MB default SO_RCVBUF)
- Shred parsing into typed structures
- Transaction decoding into `VersionedTransaction`
- Per-slot assembly with configurable FEC
- Metrics exposure (counters on `&ShredListener`)

### 1.2 Design Principles

1. **Receive path never blocks** — the UDP thread does only three things: `recvfrom()`, timestamp, enqueue. All parsing, filtering, and strategy work happens downstream.
2. **All queues are bounded** — unbounded memory growth is unacceptable in a low-latency system. Explicit drop policies are defined for every queue.
3. **Stale work is rejected over late work** — an event processed 50ms after reception is worse than an event never processed at all (it produces a trade on stale state).
4. **Observability is built in, not bolted on** — every pipeline stage produces timestamps and counters. Stage-level latency is traceable per event.
5. **Integration uses the shredstream SDK as a dependency**, not a fork. The SDK's `ShredListener::from_socket()` or `ShredListener::bind_with_options()` are the entry points.

---

## 2. Architecture: Receive Pipeline

### 2.1 Top-level Pipeline

```
┌────────────────────────────────────────────────────────────────────┐
│                      RECEIVE PIPELINE                               │
│  (Dedicated thread, pinned to isolated CPU core)                    │
│                                                                     │
│  UDP recvfrom() ──► timestamp ──► validate ──► bounded SPSC queue  │
└──────────────────────────┬─────────────────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────────────────┐
│                   RECONSTRUCTION THREAD                              │
│  (CPU core 1, or shared with decode)                                │
│                                                                     │
│  SPSC queue ──► FEC/recovery ──► slot assembly ──► reconstruct     │
│                              ──► transaction decode                  │
└──────────────────────────┬─────────────────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────────────────┐
│                   EVENT CLASSIFICATION THREAD                        │
│  (CPU core 2)                                                        │
│                                                                     │
│  program filter ──► instruction/event decode ──► dedup              │
│              ──► state transition ──► strategy gate                  │
└──────────────────────────┬─────────────────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────────────────┐
│                   EXECUTION PATH                                     │
│  (CPU core 3: signing, submission, reconciliation)                  │
│                                                                     │
│  build tx ──► sign ──► submit (SWQOS/Jito/RPC) ──► reconcile       │
└────────────────────────────────────────────────────────────────────┘
```

### 2.2 Data Flow Detail

```
UDP datagram (~1203 bytes per shred)
  │
  ▼
[UDP Receive Thread]              ← pinned CPU 0
  │ recvfrom() → tsc_now()
  │ validate: len >= MIN_SHRED_SIZE, magic byte check
  │ push to SPSC<RawShredPacket>
  │
  ▼
[Reconstruction Thread]           ← pinned CPU 1 or shared
  │ pop from SPSC
  │ classify: data shred vs code shred
  │   ├─ data shred → per-slot buffer (index-ordered)
  │   ├─ code shred → FEC set buffer
  │   └─ when slot complete → decode transactions
  │ push to MPSC<SlotEvent>
  │
  ▼
[Event Classification Thread]     ← pinned CPU 2
  │ pop from MPSC
  │ for each transaction in slot:
  │   ├─ program filter (reject irrelevant program IDs)
  │   ├─ instruction decode (identify swap/create/etc.)
  │   ├─ dedup by (slot, signature, event_type)
  │   ├─ classify event type (swap, create pool, add liq, remove liq, ...)
  │   └─ push to bounded MPSC<ClassifiedEvent>
  │
  ▼
[State & Strategy Thread]         ← pinned CPU 2 (same or sibling hyperthread)
  │ pop from MPSC
  │ ├─ update local pool state
  │ ├─ evaluate strategy gates
  │ └─ if trade triggered → create trade intent → MPSC<TradeIntent>
  │
  ▼
[Execution Thread]                ← pinned CPU 3
  │ pop from MPSC<TradeIntent>
  │ ├─ build transaction (cached blockhash, nonce, ixns)
  │ ├─ sign
  │ └─ submit via configured route (Jito/Helius/RPC)
```

---

## 3. ShredStream Receive Loop Spec

### 3.1 Socket Configuration

```rust
use shredstream::{ShredListener, ListenerOptions, AccumulatorConfig};
use std::time::Duration;

let opts = ListenerOptions {
    recv_buf: 64 * 1024 * 1024,     // 64 MiB SO_RCVBUF
    max_age: 3,                       // 3-slot retention window
    busy_poll_us: Some(200),          // SO_BUSY_POLL 200µs on Linux
    pool_size: 4096,                  // 4096 × 2 KiB zero-copy buffers
    enable_fec: true,                 // Reed-Solomon recovery enabled
    disable_salvage_delivery: false,  // deliver salvaged tail txs
    accumulator: AccumulatorConfig {
        max_fec_sets_per_slot: 32,
        stuck_batch_timeout: Duration::from_millis(50),
    },
};

let listener = ShredListener::bind_with_options(PORT, opts)
    .expect("Failed to bind ShredStream listener");
```

### 3.2 Receive Loop (Owner-Allocated Thread, NOT async)

The receive loop MUST run in a dedicated `std::thread`, NOT in a tokio task, to:
1. Avoid tokio scheduler jitter on the hot path
2. Enable reliable CPU pinning
3. Avoid any cooperative multitasking overhead

```rust
// Pseudocode — do NOT use tokio::spawn for this thread
fn shredstream_receive_loop(
    port: u16,
    opts: ListenerOptions,
    tx: crossbeam_channel::Sender<ShredPacket>,
    metrics: Arc<ShredstreamMetrics>,
) -> io::Result<()> {
    // 1. Pin this thread to dedicated CPU core (e.g., core 0)
    shredstream::pin_current_thread_to_cpu(0)?;

    // 2. Bind listener with configured options
    let listener = ShredListener::bind_with_options(port, opts)?;

    // 3. Enter tight receive loop
    for (slot, transactions) in listener.transactions() {
        // Stage 1 timestamp (immediately on receipt — SDK handles this internally)
        let tsc_received = listener.bytes_received(); // or use fast_timing::now_micros()

        // Minimal validation — SDK handles packet-level validation
        // We validate at the transaction level

        // Bounded enqueue — push to SPSC channel
        if tx.send(ShredPacket { slot, transactions, received_at: tsc_received }).is_err() {
            // Downstream disconnected — receiver should stop
            warn!("ShredStream receive channel disconnected, stopping");
            break;
        }

        // Update metrics
        metrics.packets_received.fetch_add(1, Ordering::Relaxed);
        metrics.bytes_received.fetch_add(
            transactions.iter().map(|t| t.tx_data().len() as u64).sum::<u64>(),
            Ordering::Relaxed,
        );
    }

    Ok(())
}
```

### 3.3 Buffer Sizing

| Parameter | Target | Rationale |
|-----------|--------|-----------|
| `SO_RCVBUF` | 64 MiB | Prevents kernel drops under burst. SDK default is 64 MiB. |
| `pool_size` | 4096 buffers | Each buffer is 2 KiB, total ~8 MiB pre-allocated. Sufficient for burst absorption. |
| SPSC channel cap | 32,768 packets | At ~1200 bytes/shred, ~39 MiB max in-flight. Tune after profiling. |

### 3.4 Crossbeam SPSC Channel

Use `crossbeam_channel::bounded(32_768)` for the UDP receive → reconstruction handoff.

- **Single producer** (UDP thread) + **single consumer** (reconstruction thread)
- Bounded to 32,768 elements prevents memory growth
- `try_send()` from the hot path; on `Full`, apply the [queue capacity policy](#8-bounded-queue-policy)
- `recv()` on the consumer side blocks when empty (no busy-poll waste on reconstruction thread)

---

## 4. Socket Configuration (Ubuntu VPS)

### 4.1 sysctl Configuration

```bash
# Apply immediately
sudo sysctl -w net.core.rmem_max=67108864         # 64 MiB cap (before kernel doubling)
sudo sysctl -w net.core.rmem_default=67108864
sudo sysctl -w net.core.busy_read=200               # 200 µs busy poll (SO_BUSY_POLL)
sudo sysctl -w net.core.busy_poll=200               # 200 µs busy poll (system-wide)

# Persist across reboots (add to /etc/sysctl.conf or /etc/sysctl.d/60-shredstream.conf)
cat <<'EOF' | sudo tee /etc/sysctl.d/60-shredstream.conf
net.core.rmem_max = 67108864
net.core.rmem_default = 67108864
net.core.busy_read = 200
net.core.busy_poll = 200
EOF
```

### 4.2 Verification

```bash
# Verify sysctl values
sysctl net.core.rmem_max net.core.rmem_default net.core.busy_read

# Monitor kernel UDP drops
netstat -su | grep -E "packet receive errors|RcvbufErrors"

# Per-socket buffer stats
ss -u -a -n -i | grep -A1 "src.*:8001"  # replace with your port
```

### 4.3 Understanding Linux Buffer Doubling

Linux doubles the `SO_RCVBUF` value set via `setsockopt()`. When you request 64 MiB, the kernel allocates 128 MiB (half for data, half for metadata/skbuffs). The `rmem_max` sysctl is the upper bound **before** doubling, so `rmem_max=67108864` allows a 64 MiB request (→ 128 MiB effective).

| Requested | Effective allocation | rmem_max required |
|-----------|---------------------|-------------------|
| 64 MiB    | 128 MiB             | ≥ 64 MiB          |
| 25 MiB (SDK default) | 50 MiB   | ≥ 25 MiB          |

Our 64 MiB target is a tuning hypothesis, documented for profiling. Start with 64 MiB request, monitor kernel drops with `netstat -su`, and increase if `RcvbufErrors` climbs.

### 4.4 SO_REUSEPORT vs Single Socket

**Recommendation: Single socket (no SO_REUSEPORT for initial implementation).**

Rationale:
- SO_REUSEPORT distributes incoming UDP packets across multiple kernel queues, each served by its own socket. This is useful when the receive thread cannot keep up with a single stream.
- At Solana shred rates (~10k-50k pps) and with a 64 MiB buffer, a single dedicated thread on a pinned core can handle the load.
- SO_REUSEPORT complicates ordering — packets from the same slot may arrive on different sockets, requiring cross-socket coordination.
- If profiling shows the single receive thread is saturated (CPU > 80% on pinned core), add SO_REUSEPORT with multiple receive threads and a shared SPSC merger.

Implementation note: The shredstream SDK uses a single `UdpSocket` internally. To use SO_REUSEPORT, we would need to bind multiple sockets with `SO_REUSEPORT` set (via raw socket), then pass them to `ShredListener::from_socket()`. This is an advanced optimization for later.

---

## 5. Reconstruction Policy

### 5.1 ShredStream SDK Reconstruction (Built-in)

The ShredStream SDK handles slot assembly and FEC internally. Key parameters:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `max_age` | 3 | Maximum slots retained in the window |
| `enable_fec` | true | Reed-Solomon recovery on dropped data shreds |
| `max_fec_sets_per_slot` | 32 | Per-slot FEC buffer cap |
| `stuck_batch_timeout` | 50ms | Force-finalize a stuck batch |
| `disable_salvage_delivery` | false | Drop tail transactions for lowest p99 |

### 5.2 Incomplete Slot Policy

The SDK exposes these lifecycle counters:
- `slots_completed_total` — fully reconstructed slots
- `slots_evicted_by_age` — incomplete slots evicted by max_age
- `batches_force_finalized_timeout_total` — batches finalized due to `stuck_batch_timeout`
- `salvaged_tail_tx_total` — tail transactions delivered after partial slot

**Design Decision: Accept incomplete slots.**

In a trading bot, waiting for a complete slot means waiting for all shreds, including ones we may never receive (dropped UDP). The `stuck_batch_timeout` (50ms default) force-finalizes partially-received batches. Transactions received before the timeout are delivered; missing transactions are lost.

This is acceptable because:
1. Shred loss is detected via FEC recovery counters.
2. The trading strategy must operate on incomplete data anyway — waiting for full slot reconstruction adds unbounded latency.
3. Missing a trade because we waited for an incomplete slot is worse than acting on partial data and having a competing strategy beat us.

### 5.3 Slot Retention Policy

- **3-slot window** (SDK default `max_age: 3`): The SDK retains up to 3 slots worth of shreds. Older slots are evicted and their incomplete batches force-finalized.
- This is appropriate because Solana produces one block roughly every 400ms. A 3-slot window (~1.2s) covers fork resolution while bounding memory.
- Monitor `slots_evicted_by_age` — if this counter climbs rapidly, increase `max_age` or investigate network latency.

### 5.4 FEC Handling

- **Enabled by default** (`enable_fec: true`).
- The SDK uses Reed-Solomon erasure coding to recover up to the code-shred redundancy factor per FEC set.
- Monitor `fec_recoveries_total` / `fec_recovery_failures_total`.
- A rising `fec_recovery_failures_total` indicates packet loss exceeding FEC redundancy — increase socket buffer or investigate network path.

---

## 6. Filtering Policy

### 6.1 Program ID Filter

ShredStream delivers **all shreds** from the network. We must filter to only relevant program IDs to reduce processing load.

**Relevant Program IDs** (from current SDK):

| Protocol | Program ID |
|----------|------------|
| Raydium AMM v4 | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` |
| Raydium CPMM | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` |
| Raydium CLMM | `CAMMCzo5YLJwj5aFRA44UfgSXU4z8LkZ4NBsZifQkD5` |
| PumpFun | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` |
| PumpSwap | `pAMMBay6oceH9fJKBRHqp5k4tNcS7sMq3Z3zCbPxWC` |
| Meteora DLMM | `LBUZKhRxPF3XUp4mMfH3XYGJmTVC9dFGjH1VxN3Fg` |
| Token Program | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` |
| Token 2022 | `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` |
| Associated Token | `ATokenGPvbdGVxr1b2hvZbsiqW5xr25ix9fJh4bDqQgA` |
| Bonk | `BonnKiFkGJDHrMgvDAnSqE2tC6Lb1Fq3PjLvCjKJ` |

**Filter implementation**: After transaction decode, check if any of the `message.account_keys` intersects with the allowlist. Skip transactions containing no matching program IDs.

**Important**: The program ID allowlist is configurable. It includes all protocols the bot currently supports. When adding new protocol support, add its program ID to the filter.

### 6.2 Vote Transaction Filter

Skip all vote transactions immediately (they are the majority of Solana traffic). The ShredStream SDK does not filter votes natively; we filter after decode:

```rust
// Vote instructions target Vote111111111111111111111111111111111111111
if transaction.message.account_keys.iter().any(|pk| pk.to_string() == "Vote111111111111111111111111111111111111111") {
    continue; // skip
}
```

### 6.3 Fast-Path Drop (Before Full Decode)

For maximum throughput, drop by shred variant byte before full parsing:

```rust
use shredstream::classify_variant;

let variant = classify_variant(shred_header[0]);
if !variant.is_data() {
    // Code shred — skip to FEC module, don't push to decode
    continue;
}
```

This filter happens in the reconstruction thread, not the UDP receive thread.

---

## 7. Deduplication Strategy

### 7.1 Dedup Key

Deduplicate by the composite key: **(slot, signature, event_type, instruction_index)**

```rust
#[derive(Hash, Eq, PartialEq)]
struct DedupKey {
    slot: u64,
    signature: [u8; 64],  // ed25519 signature bytes
    event_type: EventType, // Swap, CreatePool, AddLiquidity, etc.
    instruction_index: u8, // which instruction in the tx
}
```

### 7.2 Why This Key

| Component | Handles |
|-----------|---------|
| `slot` | Same tx appearing in multiple slots (unlikely but possible on fork) |
| `signature` | Same tx delivered by redundant streams |
| `event_type` | Same tx may have multiple relevant instructions |
| `instruction_index` | Same tx, same program, different swap instructions |

### 7.3 Dedup Set Design

```rust
// Bounded LRU cache: 262,144 entries (~ 3.5 MiB for keys + overhead)
use lru::LruCache;
use std::num::NonZeroUsize;

struct DedupCache {
    inner: LruCache<DedupKey, ()>,
}

impl DedupCache {
    fn new() -> Self {
        Self {
            inner: LruCache::new(NonZeroUsize::new(262_144).unwrap()),
        }
    }

    fn check_and_insert(&mut self, key: DedupKey) -> bool {
        // Returns true if NOT seen before (i.e., new event)
        !self.inner.contains(&key) && self.inner.put(key, ()).is_none()
    }
}
```

### 7.4 Dedup Coverage

| Scenario | Handled | Notes |
|----------|---------|-------|
| Duplicate UDP packet (network retransmission) | ✅ | Same (slot, sig, event_type) → dedup |
| Redundant ShredStream streams (multi-region) | ✅ | Same tx delivered twice → dedup by signature |
| Process restart (volatile cache lost) | ❌ | Acceptable — brief window of potential re-processing; trade safety checks prevent duplicate submission |
| Multi-lane submission | ✅ | Dedup by (slot, signature); but our own submitted txs shouldn't arrive via ShredStream (they're new on-chain) |

### 7.5 LRU Eviction

- **Capacity**: 262,144 entries
- **Eviction policy**: LRU — oldest seen signatures are evicted
- **Age bound**: Transactions older than ~300 slots (roughly 2 minutes at 400ms/slot) are unlikely to be re-delivered. With 262k entries at ~50 txs/slot, this covers ~5,000 slots of unique transactions.
- Monitor dedup hit rate via `dedup_hit_count` / `dedup_total_count` metrics.

---

## 8. Bounded Queue Policy

### 8.1 Queues in the Pipeline

| Queue | Type | Capacity | Producer | Consumer |
|-------|------|----------|----------|----------|
| `Q1: raw_packets` | `crossbeam_channel::bounded(32_768)` | 32,768 shreds | UDP receive thread | Reconstruction thread |
| `Q2: decoded_slots` | `crossbeam_channel::bounded(1_024)` | 1,024 slots | Reconstruction thread | Event classification thread |
| `Q3: classified_events` | `crossbeam_channel::bounded(8_192)` | 8,192 events | Event classification thread | State & strategy thread |
| `Q4: trade_intents` | `crossbeam_channel::bounded(256)` | 256 intents | State & strategy thread | Execution thread |

### 8.2 Policy on Full Queue

All queues must have explicit behavior when full:

| Queue | Full Behavior | Rationale |
|-------|---------------|-----------|
| Q1 | Drop oldest shred (drop oldest) | Newest shreds are most likely to complete a slot |
| Q2 | Block consumer (backpressure) | If decode can't keep up, slow the receive path |
| Q3 | Drop newest event | A new event is less stale than an old one, but dropping prevents memory DoS |
| Q4 | Drop oldest intent | Old intents are based on stale state; better to skip than trade on old data |

Implementation: Use `try_send()` on all non-backpressure paths. On `Full`:
- Q1: `crossbeam_channel::try_send()` → on `Full`, `recv()` from the same channel to pop oldest, then retry `try_send()`. Count drop in metric.
- Q2: `crossbeam_channel::send()` (blocks) — backpressure is intentional here.
- Q3, Q4: `try_send()` → on `Full`, increment drop counter, drop newest.

### 8.3 Degraded Mode

If **any queue** exceeds 75% capacity sustained for >5 seconds:
1. Emit `DEGRADED` status metric.
2. Optionally: drop low-priority programs from the filter (configurable).
3. Optionally: stop accepting new trade intents.
4. When queue depth falls below 25%, exit degraded mode.

### 8.4 Stop-Trading Threshold

If **Q4** is full for >500ms consecutively, **stop all new trading**:
- The strategy is producing intents faster than the execution pipeline can submit.
- Continuing to produce intents will build a backlog of stale trades.
- Recovery: drain Q4 to <10% capacity, re-evaluate strategy, then resume.

---

## 9. Observability

### 9.1 Stage-Level Timestamps

Every classified event carries stage timestamps in a compact bit-field or array:

```
packet_received → (assigned by SDK on recvfrom())
reconstruction_complete → (assigned after slot assembly + tx decode)
transaction_decoded → (same as above in practice)
event_classified → (after instruction decode + dedup)
dedup_complete → (after dedup check passes)
state_updated → (after pool state mutation)
strategy_complete → (after strategy evaluation, set to 0 if no trade)
```

Implementation: use `common::fast_timing::now_micros()` for all stage timestamps. The `HighPerformanceClock` provides sub-microsecond monotonic timestamps with minimal overhead.

```rust
pub struct EventTrace {
    pub slot: u64,
    pub signature: [u8; 64],
    pub stages: [i64; 7],     // 0 = unset
    pub stage_names: [&'static str; 7] = [
        "packet_received",
        "reconstruction_complete",
        "transaction_decoded",
        "event_classified",
        "dedup_complete",
        "state_updated",
        "strategy_complete",
    ];
}
```

Export as log structured data (not per-event to stdout) — write to a trace buffer that is periodically sampled or flushed on trade intent.

### 9.2 Metrics (Prometheus-compatible)

Use `std::sync::atomic` counters in `Arc<ShredstreamMetrics>`:

```
# Throughput
shredstream_packets_received_total
shredstream_bytes_received_total
shredstream_slots_completed_total
shredstream_transactions_decoded_total

# Kernel / transport
shredstream_kernel_drops_total        # from netstat -su monitoring thread
shredstream_fec_recoveries_total
shredstream_fec_recovery_failures_total
shredstream_fec_sets_discarded_total

# Filtering
shredstream_transactions_filtered_total{reason="program_id"}
shredstream_transactions_filtered_total{reason="vote"}
shredstream_events_classified_total{event_type="swap|create|add_liq|remove_liq"}

# Dedup
shredstream_dedup_hits_total
shredstream_dedup_misses_total

# Queues
shredstream_queue_depth{queue="raw_packets|decoded_slots|classified_events|trade_intents"}
shredstream_queue_drops_total{queue="raw_packets|classified_events|trade_intents"}
shredstream_queue_full_duration_seconds{queue="..."}

# Latency (p50/p95/p99 per stage)
shredstream_stage_latency_seconds{stage="packet_to_reconstruct|reconstruct_to_classify|classify_to_state|state_to_strategy"}

# Slot quality
shredstream_incomplete_slots_total
shredstream_slots_evicted_by_age_total
shredstream_batches_force_finalized_total{cause="timeout|corrupted"}

# Stale events
shredstream_stale_events_dropped_total
shredstream_event_age_seconds
```

### 9.3 Latency Pipeline

The critical end-to-end latency measurement is:

```rust
// In the event classification thread, after dedup:
let now = fast_timing::now_micros();
let e2e_latency = now - trace.packet_received;
// record in histogram
```

Target: **<50ms p95 from packet_received → strategy_complete** for competitive HFT scenarios.

### 9.4 Metrics Export Strategy

- **Hot path**: Atomic counters only (zero allocation, no blocking).
- **Export**: A background thread reads atomics every 5 seconds and pushes to a Prometheus endpoint or writes to a structured log file.
- **Latency histograms**: Use `hdrhistogram` crate for high-dynamic-range latency recording. Export p50/p95/p99/p999 every 60 seconds.

---

## 10. Hot-Path Contract

### 10.1 What the Receive Path MUST NOT Do

The UDP receive thread and the reconstruction thread are the **hot path**. They must NEVER perform:

```
❌ RPC calls (HTTP/gRPC client calls)
❌ Database writes (file I/O, RocksDB, SQLite)
❌ JSON serialization / deserialization
❌ Strategy calculations (price evaluation, risk checks, trade decisions)
❌ Token security analysis (rug check, mint/burn analysis)
❌ Transaction building (constructing new transactions for submission)
❌ Signing (ed25519 key operations)
❌ Submission (sending to Jito/Helius/RPC)
❌ High-volume human-readable logging (println!, per-packet tracing logs)
❌ DNS resolution
❌ TLS/QUIC handshake
❌ Heap allocation on the critical recvfrom() path (use pre-allocated buffer pool)
```

### 10.2 What the Receive Path MAY Do

```
✅ recvfrom() syscall (mandatory)
✅ TSC/monotonic clock read (fast_timing::now_micros())
✅ Bounded SPSC push (try_send with no allocation)
✅ Minimal validation (packet length check, magic byte check)
✅ Atomic counter increment (metrics that are hot-path-critical)
```

### 10.3 What the Event Classification Thread MUST NOT Do

The classification thread (after the reconstruction thread) is still latency-sensitive:

```
❌ Same restrictions as receive path, PLUS:
❌ Synchronous blockhash queries
❌ Token account existence queries (ATA checks)
❌ Price feed lookups (Pyth, Switchboard)
❌ State queries to RPC (balance, reserve, slot)
```

### 10.4 Permitted in Classification Thread

```
✅ Program ID matching (compile-time generated bloom filter or hash set)
✅ Instruction discriminator matching (first 8 bytes of instruction data)
✅ Dedup cache check (LRU contains check + insert)
✅ Dedup statistics (atomic counters)
✅ Enqueue to classified_events channel
```

---

## 11. Integration Points with Existing SDK

### 11.1 What Exists

The current SDK already has:

- **`common/clock.rs`**: `HighPerformanceClock` with monotonic + base UTC microsecond. Provides `now_micros()`. Already designed for consistent timestamps.
- **`common/fast_timing.rs`**: `FastTimer` / `FastStopwatch` with syscall bypass. Provides `fast_now_nanos()`, `fast_now_micros()`.
- **`perf/ultra_low_latency.rs`**: `LockFreeEventDispatcher` with `ArrayQueue` + CPU affinity. Contains patterns for bounded queue management.
- **`perf/kernel_bypass.rs`**: Ring buffer RX/TX queues (stub). Provides CPU affinity helper via `libc::sched_setaffinity`.
- **`trading/core/`**: `TradeExecutor` trait, `ExecutionPath`, `InstructionProcessor`, `Prefetch` helper.
- **`swqos/`**: Transaction submission providers (Jito, Helius, RPC, etc.).
- **`client/`**: `TradingClient` — current entry point using RPC for data and submission.

### 11.2 What Needs to Be Added

| Component | Location | Description |
|-----------|----------|-------------|
| `ShredstreamConfig` | `src/perf/shredstream/mod.rs` | Configuration struct matching `ListenerOptions` |
| `ShredstreamAdapter` | `src/perf/shredstream/adapter.rs` | Wraps `ShredListener`, owns receive thread |
| `ShredstreamMetrics` | `src/perf/shredstream/metrics.rs` | Atomic counters for observability |
| `DedupCache` | `src/perf/shredstream/dedup.rs` | Bounded LRU dedup by (slot, sig, event_type, ix_index) |
| `EventClassifier` | `src/perf/shredstream/classifier.rs` | Program filter + instruction decode + event type |
| `PipelineConfig` | `src/perf/shredstream/config.rs` | All queue sizes, timeouts, filter lists |

### 11.3 New Module Structure

```
src/perf/shredstream/
├── mod.rs              # Re-exports, top-level Pipeline struct
├── config.rs           # ShredstreamConfig, QueueConfig
├── adapter.rs          # ShredstreamAdapter wrapping ShredListener
├── receive_loop.rs     # Raw receive loop (UDP SPSC producer)
├── reconstruction.rs   # Slot assembly, FEC monitoring
├── classifier.rs       # Program filter, event decode, dedup
├── dedup.rs            # LRU dedup cache
├── metrics.rs          # Atomic metrics
├── state.rs            # Pipeline state machine (running, degraded, stopped)
└── tests.rs            # Unit tests
```

### 11.4 Integration with Existing Types

- `EventMessage` in `perf/ultra_low_latency.rs` currently uses `fzstream_common::EventMessage`. The new shredded pipeline should define its own `ShredEvent` type that bridges to the existing event system.
- The `TradeExecutor` trait in `trading/core/traits.rs` takes `SwapParams` — the classified event must be convertible to `SwapParams` for the strategy gate.

---

## 12. Threading and CPU Allocation

### 12.1 Initial Allocation (Hypothesis)

```
CPU 0: ShredStream UDP receive   (std::thread, isolated core)
CPU 1: Reconstruction + decode   (std::thread, isolated core)
CPU 2: Event classification + state + strategy  (std::thread or tokio::task)
CPU 3: Signing + submission + reconciliation  (tokio runtime, async)
```

This is a test hypothesis. Profile to validate before assuming ideal isolation.

### 12.2 Thread Pinning

Use `shredstream::pin_current_thread_to_cpu(cpu_id)` which maps to:
- **Linux**: `sched_setaffinity()` via libc
- **macOS**: thread priority hint (no-op for affinity)
- **Other**: no-op

### 12.3 Async vs Sync Decision

| Thread | Sync/Async | Reason |
|--------|-----------|--------|
| UDP receive | Sync (`std::thread`) | Tight loop, no async needed, avoids scheduler jitter |
| Reconstruction | Sync or dedicated | CPU-bound shred parsing |
| Classification | Sync | CPU-bound program matching + decode |
| State + Strategy | Possibly async | May need to read cached state |
| Execution | Async (tokio) | RPC calls, submission, timers |

### 12.4 NUMA Awareness

On multi-socket VPS instances, ensure all pinned threads are on the same NUMA node to avoid cross-socket memory access penalties. Use `numactl --cpunodebind=0` or programmatic binding.

---

## 13. Kernel Bypass Evaluation

### 13.1 When to Consider Kernel Bypass

The existing `perf/kernel_bypass.rs` contains a stub `KernelBypassUDP` struct. **Do not use kernel bypass in the initial implementation.**

Kernel bypass (DPDK, AF_XDP, io_uring) is warranted only when:

1. **Evidence shows the kernel UDP stack is the bottleneck.** Measure: `perf top` showing `skb_clone`, `__netif_receive_skb_core` consuming significant CPU.
2. **Packet loss persists after socket buffer tuning.** If `netstat -su` shows `RcvbufErrors` after setting 64 MiB buffer, the problem may be kernel-side.
3. **We deploy on bare metal or VPS with DPDK-capable NICs.** Most cloud VPS instances don't support DPDK (requires SR-IOV or virtio with vDPA).

### 13.2 Kernel Bypass Decision Matrix

| Condition | Action |
|-----------|--------|
| Kernel drops < 0.01% with 64 MiB buffer | Kernel bypass not needed |
| Kernel drops > 0.1% after buffer tuning | Investigate: is the receive thread keeping up? |
| CPU core saturated on recvfrom + copy | Consider GRO/LRO, then AF_XDP |
| Sub-10µs latency required | Consider DPDK or AF_XDP |
| Cloud VPS (AWS, GCP, Hetzner) | Stick with kernel UDP — DPDK usually unavailable |

### 13.3 AF_XDP Path (Future)

If kernel bypass is needed, prefer **AF_XDP** over DPDK:
- Works on any Linux 4.18+ with `CONFIG_XDP=y`
- No kernel module required
- Standard socket API (sendmsg/recvmsg) for control path
- Zero-copy with UMEM + fill queue + completion ring

---

## 14. ShredStream SDK Adapter Design

### 14.1 Configuration

```rust
pub struct ShredstreamConfig {
    // Network
    pub port: u16,
    pub bind_address: String,           // "0.0.0.0"

    // SDK options
    pub recv_buf: usize,                // 64 * 1024 * 1024
    pub max_age: u64,                   // 3
    pub busy_poll_us: Option<u32>,      // Some(200)
    pub pool_size: usize,               // 4096
    pub enable_fec: bool,               // true
    pub disable_salvage_delivery: bool, // false
    pub stuck_batch_timeout_ms: u64,   // 50

    // CPU affinity
    pub receive_core: Option<usize>,    // core 0
    pub reconstruct_core: Option<usize>,// core 1

    // Filter
    pub allowed_programs: Vec<String>,  // program IDs to keep
    pub filter_votes: bool,             // true

    // Queue sizing
    pub raw_packet_queue_capacity: usize,   // 32768
    pub decoded_slot_queue_capacity: usize, // 1024
    pub classified_event_queue_capacity: usize, // 8192
    pub trade_intent_queue_capacity: usize, // 256

    // Dedup
    pub dedup_cache_size: NonZeroUsize,     // 262144
}
```

### 14.2 Adapter Struct

```rust
pub struct ShredstreamAdapter {
    config: ShredstreamConfig,
    listener: Option<ShredListener>,
    metrics: Arc<ShredstreamMetrics>,
    state: Arc<AtomicU8>, // 0=stopped, 1=running, 2=degraded, 3=stopped
    // Channel handles
    raw_packet_tx: crossbeam_channel::Sender<ShredPacket>,
    // Thread join handles
    receive_thread: Option<JoinHandle<()>>,
    reconstruct_thread: Option<JoinHandle<()>>,
}
```

### 14.3 Lifecycle

```
adapter.start()        → binds socket, spawns receive thread, begins streaming
adapter.stop()         → signals threads to stop, drains queues, unbinds socket
adapter.restart()      → stop + start (for reconnection)
adapter.update_config() → hot-reload filter list, queue sizes (limited)
```

### 14.4 Error Handling on Bind Failure

If `ShredListener::bind_with_options()` fails:
1. Log structured error with port, IP, and OS error.
2. Retry up to 3 times with exponential backoff (100ms, 500ms, 2s).
3. If all retries fail, fall back to LaserStream gRPC if configured.

### 14.5 Connection Loss Detection

ShredStream delivers UDP; the connection is stateless. Detect loss by:
- **Packet rate monitoring**: If fewer than X packets received in Y seconds, emit a warning.
- **Slot gap detection**: If slot numbers jump by >10 without intervening packets, possible connection interruption.
- **No recovery**: Unlike TCP, there's no reconnection — the stream either delivers or doesn't. If no packets for 10 seconds, restart the adapter (which re-binds the socket).

---

## 15. LaserStream Fallback Strategy

### 15.1 When to Use LaserStream

LaserStream (Helius gRPC) is a **fallback** data source for scenarios where:
1. ShredStream is down or unreachable.
2. We need historical replay after a connection interruption (LaserStream supports up to 24 hours of replay).
3. We need confirmed/finalized state for reconciliation (processed-level ShredStream data may fork).

### 15.2 Dual-Feed Architecture (Future)

```
   ShredStream (UDP)     LaserStream (gRPC)
         │                      │
         │      ┌──────────┐    │
         └──────► Dedup    ◄────┘
                │ Merger   │
                └────┬─────┘
                     │
              processed events
```

The merger deduplicates by (slot, signature) — whichever feed delivers first wins. The LaserStream feed provides:
- **Processed level**: Same latency tier as ShredStream (when co-located)
- **Confirmed/Finalized level**: For fallback and reconciliation

### 15.3 Initial Implementation

Phase 1: ShredStream only (no LaserStream fallback). Monitor connection health. If ShredStream drops, the bot stops trading rather than degrading to slow data.

Phase 2: Add LaserStream as a secondary processed-level source for redundancy.

Phase 3: Add LaserStream confirmed/finalized for reconciliation.

---

## Appendix A: ShredStream SDK Reference (Key Types)

**`ShredListener` API** (from `shredstream` v2.0 crate):

| Method | Description |
|--------|-------------|
| `bind(port)` | Bind with defaults (64 MB recv buf, 3 slot window, FEC enabled) |
| `bind_with_options(port, opts)` | Custom configuration |
| `from_socket(socket, opts)` | Adopt existing `UdpSocket` |
| `transactions()` | Blocking iterator: `(slot, Vec<VersionedTransaction>)` |
| `shreds()` | Blocking iterator: `RawShred` headers (no decode) |
| `handle_packet(&[u8])` | Inject externally-received UDP datagram |

**`ListenerOptions`**:

| Field | Default | Description |
|-------|---------|-------------|
| `recv_buf` | 64 MiB | SO_RCVBUF size |
| `max_age` | 3 | Slot retention window |
| `busy_poll_us` | Some(200) | SO_BUSY_POLL µs (None disables) |
| `pool_size` | 4096 | Zero-copy buffer pool |
| `enable_fec` | true | Reed-Solomon recovery |
| `disable_salvage_delivery` | false | Drop salvaged txs for lowest p99 |
| `accumulator` | defaults | FEC and stuck-batch tuning |

**`AccumulatorConfig`**:

| Field | Default | Description |
|-------|---------|-------------|
| `max_fec_sets_per_slot` | 32 | Per-slot FEC buffer cap |
| `stuck_batch_timeout` | 50ms | Force-finalize stuck batch |

**Metrics on `&ShredListener`**:

| Group | Methods |
|-------|---------|
| Throughput | `data_shred_count_total`, `code_shred_count_total`, `bytes_received`, `slot_count` |
| Decoder | `batches_decoded_streaming_total`, `batches_decoded_fallback_total`, `batches_skipped_total`, `decode_errors_total` |
| FEC | `fec_recoveries_total`, `fec_recovery_failures_total`, `fec_sets_discarded_unused_total`, `fec_sets_evicted_early_total` |
| Unparseable | `unparseable_packets`, `unparseable_too_short`, `unparseable_variant`, `unparseable_payload`, `unparseable_slot_range` |
| Slot lifecycle | `slots_completed_total`, `slots_evicted_by_age`, `dropped_known_slots`, `harvested_batches_total`, `salvaged_tail_tx_total` |
| Tail control | `batches_force_finalized_corrupted_total`, `batches_force_finalized_timeout_total` |
| Pool/I/O | `pool_exhausted_count`, `last_io_error_kind`, `busy_poll_active` |

## Appendix B: Example Config File

```toml
[shredstream]
port = 8001
bind_address = "0.0.0.0"
receive_core = 0
reconstruct_core = 1

[socket]
recv_buf = 67108864      # 64 MiB
busy_poll_us = 200

[fec]
enabled = true
stuck_batch_timeout_ms = 50

[filter]
allowed_programs = [
    "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8",  # Raydium AMM v4
    "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C",  # Raydium CPMM
    "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",  # PumpFun
]
filter_votes = true

[queue]
raw_packet_capacity = 32768
decoded_slot_capacity = 1024
classified_event_capacity = 8192
trade_intent_capacity = 256

[dedup]
cache_size = 262144
```

---

*End of design document. This is a design-only artifact. No code has been implemented.*