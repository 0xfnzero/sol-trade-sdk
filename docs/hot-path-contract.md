# Hot-Path Contract

> Defines what the low-latency event-to-submit path MUST and MUST NOT do.
> This contract applies to every thread in the ShredStream receive pipeline.
> Violations must be detectable via instrumentation.

---

## 1. The Qualifying Path

The qualifying hot path from packet arrival to transaction submission is:

```text
UDP recvfrom()
→ timestamp (TSC/monotonic)
→ minimal validation (packet size, magic bytes)
→ bounded SPSC enqueue
→ FEC/reconstruction (SDK-managed)
→ transaction decode
→ program ID filter
→ instruction/event decode
→ deduplication
→ state transition
→ strategy evaluation (deterministic gates only)
→ create validated trade intent
→ fee engine check (pre-computed; no RPC)
→ risk controls check (in-memory, no I/O)
→ build transaction from cached dependencies
→ sign
→ submit (configured lane)
```

### 1.1 Updated Hot-Path Pipeline (SDK-Accurate)

The actual SDK hot path in `sol-trade-sdk` is:

```text
Event (ShredStream/gRPC)
→ filter by program ID
→ deduplicate (bounded LRU, DashMap)
→ reject stale event (slot age check)
→ map post-trade state → build ProtocolParams (PumpFunParams::from_trade, etc.)
→ strategy evaluation → trade intent
→ FeeEngine::compute_fees()
→ RiskGovernor::check_*()
→ compute_budget_manager::extend_compute_budget_instructions (cached arc)
→ build_transaction (transaction_pool::acquire_builder)
→ build_versioned_transaction
→ builder.build_zero_alloc()  (locked)
→ sign (ed25519: try_sign_message)
→ submit (SWQOS lane: Jito/Helius/NextBlock/etc.)
```

### 1.2 What Belongs in the Hot Path

✅ **Permitted operations** (in order of the pipeline):

| Stage | Operation | Details |
|---|---|---|
| Filter | Program ID match | Compile-time hash table lookup (SIMD compare) |
| Deduplicate | Signature check | Bounded DashMap (100K entries, O(1)) |
| Reject stale | Slot age comparison | `current_slot - event_slot > max_staleness` |
| State update | Pool reserve update | In-place struct mutation, no alloc |
| Strategy | Deterministic gates only | No AI/ML, no external data fetches |
| Fee engine | Compute fees | In-memory cache hit; no RPC |
| Risk check | Verify controls | In-memory counters; no I/O |
| Build | Instruction construction | Cached Arc instructions, extend_from_slice |
| Compute budget | Extend cached | DashMap hit returns `Arc<SmallVec>` |
| Transaction build | acquire_builder → build → release | Pooled TransactionBuilder |
| Sign | ed25519_sign | CPU-bound but necessary |
| Submit | SWQOS lane send | HTTP/QUIC post (non-blocking via dedicated sender thread) |

## 2. Hot Path: MUST NOT

### 2.1 UDP Receive Thread

The UDP receive thread (dedicated, pinned to CPU core) performs the absolute minimum work. It MUST NOT:

| ❌ Prohibited | Rationale |
|--------------|-----------|
| RPC calls (HTTP/gRPC) | Network I/O blocks the receive loop; kernel buffer fills → drops |
| Database writes | Disk I/O is 1000x slower than memory; blocks the hot path |
| JSON serialization/deserialization | Allocates memory, CPU-intensive for binary shred data |
| Strategy calculations | Strategy belongs in a downstream stage; the receive thread does not evaluate trades |
| Token security analysis | "Rug check" is not time-critical in the receive path |
| Transaction building | Building new transactions belongs in the execution thread |
| Signing | ed25519 operations are CPU-intensive and unnecessary before strategy evaluation |
| Submission | Sending data to Jito/Helius/RPC requires network I/O |
| High-volume human-readable logging | `println!`, `eprintln!`, and per-packet `info!` logs create string formatting overhead |
| DNS resolution | DNS lookups are network-dependent and can stall |
| TLS/QUIC handshake | Cryptographic handshake + network I/O |
| Heap allocation | `Vec::new()`, `String::new()`, `format!()` on the critical recvfrom() path |
| Mutex acquisition (contended) | Lock contention can stall the receive thread |
| Tokio task spawning | Cooperative scheduling adds jitter to the tight receive loop |

### 2.2 Reconstruction & Classification Thread

These threads (CPU cores 1-2) are still on the latency-critical path. They MUST NOT:

| ❌ Prohibited | Rationale |
|--------------|-----------|
| All items from §2.1 (above) | Same reasoning applies |
| Synchronous blockhash queries | Blockhash service runs in a background thread; hot path never queries it synchronously |
| Token account existence queries (ATA checks) | ATA lookup requires RPC; cache ATA state out-of-band |
| Price feed queries (Pyth, Switchboard, Jupiter) | Price feeds change every slot; cache and update asynchronously |
| Pool reserve queries via RPC | Reserves are updated from ShredStream events, not RPC polls |
| SPL Token account balance queries | Balance is derived from event observation, not on-demand RPC |
| External HTTP requests of any kind | Network I/O of any form blocks event processing |
| File I/O (read or write) | Disk is too slow for the hot path |
| Cross-thread synchronization with execution thread | Use bounded channels, not shared mutexes |

### 2.3 Prohibited Patterns (All Threads)

| ❌ Prohibited Pattern | Example | Alternative |
|----------------------|---------|-------------|
| `Instant::now()` in hot loop | Per-packet `Instant::now()` calls | Use `HighPerformanceClock::now_micros()` (monotonic + base offset) |
| `format!()` in hot loop | Building strings for every event | Use structured log macros that are sampled or gated |
| `Vec::push()` unsized | Growing vectors without reserve | Pre-allocate with `Vec::with_capacity()` or use fixed-size arrays |
| `HashMap::insert()` in hot loop | Unbounded map growth | Use bounded `LruCache` for dedup; pre-size known maps |
| `.clone()` on large structs | Cloning `VersionedTransaction` | Pass references or `Arc`; avoid cloning in the hot path |
| Channel `.send()` without bounded check | Unbounded channels grow without bound | Always use bounded channels with explicit drop policy |
| `unwrap()` or `expect()` on `Result` | Panicking on transient errors | Handle errors gracefully with fallbacks or drop counters |
| `println!`/`eprintln!` in production | Synchronous stdout/stderr writes | Use structured logging with sampling, off the hot path |
| `tokio::spawn()` in the receive loop | New async task per packet | Dedicated `std::thread` for receive; no async in receive loop |

## 3. Hot Path: MAY Do

### 3.1 UDP Receive Thread

| ✅ Permitted | Details |
|-------------|---------|
| `recvfrom()` syscall | Mandatory — the only way to receive UDP data |
| `fast_timing::now_micros()` | Nanosecond-cost timestamp via `HighPerformanceClock` |
| `classify_variant()` on shred header byte | Byte-level check, no allocation |
| `try_send()` to bounded SPSC channel | Non-blocking; on failure, apply queue drop policy |
| Atomic counter increments | `AtomicU64::fetch_add(1, Relaxed)` for metrics |
| Memory prefetch hints | `_mm_prefetch()` for next buffer cache line |

### 3.2 Reconstruction Thread

| ✅ Permitted | Details |
|-------------|---------|
| All items from §3.1 | UDP data arrives after dequeuing |
| SDK slot assembly | ShredStream SDK manages per-slot buffers internally |
| Program ID matching | Compile-time hash set; linear scan of up to ~32 account keys per tx |
| Instruction discriminator matching | First 8 bytes of instruction data |
| Dedup cache check + insert | Bounded LRU; cache hit is O(1) |
| Stage timestamp recording | `now_micros()` for each pipeline stage |
| Counter increment for FEC/metrics | Atomic counters only |

### 3.3 Classification Thread

| ✅ Permitted | Details |
|-------------|---------|
| All items from §3.2 | Plus: |
| Event type classification | Match decoded instructions to known event types (swap, create, add liquidity, etc.) |
| Extract trade-relevant fields | Pool ID, amounts, mints, price from decoded instructions |
| Enqueue to classified_events channel | Bounded `try_send()` with drop policy |
| Staleness check | Compare event slot to current slot; reject if too old (configurable, default 2 slots) |

## 4. Detection of Violations

### 4.1 Instrumented Checks

The hot path should be instrumented so violations are detectable:

```rust
// At the start of the receive loop
#[cfg(debug_assertions)]
static RECEIVE_LOOP_ENTERED: AtomicBool = AtomicBool::new(false);

impl ShredstreamAdapter {
    pub fn assert_receive_thread() {
        #[cfg(debug_assertions)]
        assert!(
            RECEIVE_LOOP_ENTERED.load(Ordering::Relaxed),
            "Operation called outside receive thread"
        );
    }
}
```

### 4.2 Monitoring for Violations

| Signal | What It Indicates |
|--------|-------------------|
| Queue near-full sustained (>75%) | Downstream not keeping up with receive rate |
| Queue drops > 0.1% of total events | Queue capacity too small or consumer too slow |
| Kernel UDP drops > 0.01% | Receive thread not draining fast enough or buffer too small |
| Receive thread CPU > 90% | Thread doing too much work; offload more to downstream |
| Reconstruction latency > 10ms | FEC recovery or slot assembly is bottleneck |
| End-to-end latency > 100ms (p50) | Pipeline stage too slow; investigate each stage |
| Blocking calls detected via strace/perf | syscall frequency reveals hidden blocking operations |

## 5. Queue Capacity Design

| Queue | Capacity | On Full | Location |
|-------|----------|---------|----------|
| Raw shreds → Reconstruction | 32,768 | Drop oldest | UDP receive thread |
| Decoded slots → Classification | 1,024 | Block (backpressure) | Reconstruction thread |
| Classified events → Strategy | 8,192 | Drop newest | Classification thread |
| Trade intents → Execution | 256 | Drop oldest | Strategy thread |

### 5.1 Why Block on Decoded Slots

The `decoded_slots` queue intentionally blocks (backpressure) because:
- Reconstruction is CPU-bound and memory-intensive (slot assembly + FEC).
- If the classification thread cannot keep up, it's better to slow the reconstruction thread than to pile up incomplete slots in memory.
- The crossbeam channel's `send()` blocks the producer, naturally throttling the reconstruction rate.

### 5.2 Why Drop Newest on Classified Events

The `classified_events` queue drops the **newest** event when full because:
- If strategy is already overloaded, adding more events won't help.
- Dropping newest means we process old events first — they're less likely to be stale.
- Strategy should catch up on existing events before receiving more.

### 5.3 Why Drop Oldest on Trade Intents

The `trade_intents` queue drops the **oldest** intent when full because:
- Old intents are based on stale pool state; executing them would trade on outdated data.
- Newer intents reflect more current state and are more likely to be profitable/safe.
- If intents queue to capacity, the strategy is producing faster than execution can submit.

## 6. Staleness Policy

An event is considered stale and MUST be dropped if:

```rust
fn is_stale(event_slot: u64, current_slot: u64, max_staleness_slots: u64) -> bool {
    current_slot.saturating_sub(event_slot) > max_staleness_slots
}
```

- **Default max_staleness_slots**: 2 (two slots behind current).
- The current slot is tracked from ShredStream: the SDK's `slot_count()` and the maximum slot value seen in recent batches.
- Stale events must NOT be used for state updates or strategy evaluation.
- Exception: if the bot just restarted and the state engine needs to catch up, an RPC refresh is used instead of replaying stale ShredStream events.

## 7. Degraded Mode Entry/Exit

### 7.1 Enter Degraded

Enter degraded mode when any of:
- Any queue > 75% capacity for >5 seconds.
- End-to-end latency > 100ms for >5 seconds.
- FEC recovery failure rate > 1% for >30 seconds.
- Kernel UDP drops detected.

### 7.2 Behavior in Degraded

- Stop accepting new trade intents (drain Q4).
- Emit `DEGRADED` status metric.
- Log a single structured warning per degradation event (not per-packet).
- Continue processing events without triggering trades.

### 7.3 Exit Degraded

Exit degraded mode when all of:
- All queue depths < 25% capacity.
- End-to-end latency < 50ms for >5 seconds.
- FEC recovery failure rate < 0.1% for >30 seconds.

### 7.4 Stop Trading

Stop trading entirely when:
- Trade intent queue (Q4) full for >500ms consecutively.
- Strategy producing intents faster than execution can submit.
- Recovery requires human or automated intervention: drain queue, re-evaluate strategy parameters.

## 8. Verification

Every implementation of the hot path must pass:

1. **Static analysis**: No prohibited imports (tokio::net, reqwest, hyper) in receive/reconstruction modules.
2. **Code review**: Every call on the receive thread path is audited.
3. **Profiling**: `perf stat` shows zero syscalls beyond `recvfrom()` and `clock_gettime` on the receive thread.
4. **Load testing**: At 2x sustained shred rate, no queue exceeds 50% capacity.
5. **No false drops**: Under normal load, queue drops are zero.
6. **Latency budgets**: Each stage latency budget is measured and enforced.

---

## 9. Hot Path: FORBIDDEN (Expanded)

In addition to all items in §2, the following are **STRICTLY FORBIDDEN** anywhere on the hot path (receive → submit):

| ❌ FORBIDDEN | Why |
|---|---|
| **RPC calls** (HTTP/gRPC of any kind) | Network I/O blocks; the hot path never waits for RPC |
| **Disk writes** (file I/O, state persistence) | 1000x slower than memory; use async background flush |
| **DNS resolution** | Stalls; DNS must be resolved at startup |
| **New connections** (TCP/TLS/QUIC handshake) | Handshake latency is unbounded; connections must be pre-warmed |
| **ALT creation** | Lookup table creation requires RPC + confirmation; create at startup |
| **Synchronous queries** of any kind | Blocking awaits on hot path destroy latency |
| **AI inference** | Unbounded, non-deterministic latency |
| **Lock contention** (async Mutex across stages) | Use bounded channels; never share mutable state with locks |
| **Heap allocation** (Vec::new, String::new, format!) | Pre-allocate and reuse; zero-allocation target |
| **Ed25519 verification** during filter | CPU-intensive; only sign, never verify, on hot path |
| **Clone of VersionedTransaction** | Large struct; pass Arc or reconstruct from pool |
| **println! / eprintln!** in production | Synchronous stdout write blocks the thread |
| **tokio::spawn** inside the event loop | Adds scheduling jitter; pinned threads only |
| **Backpressure-oblivious channel sends** | Always use bounded `try_send()` with explicit drop policy |

## 10. CPU / Threading Design (Section 18)

### 10.1 Initial Core Allocation Hypothesis

On a 4+ core machine, the recommended CPU topology is:

| CPU Core | Role | Threads | Notes |
|---|---|---|---|
| **Core 0** | UDP receive | 1 pinned thread | `recvfrom()`, minimal validation, enqueue to SPSC |
| **Core 1** | Decode + Classify | 1 pinned thread | Shred decode, FEC recovery, transaction decode, program ID filter, dedup, staleness check |
| **Core 2** | State + Strategy | 1 pinned thread | Pool reserve update, strategy evaluation, risk check, fee engine, trade intent creation |
| **Core 3** | Sign + Submit | 1 pinned thread | `acquire_builder`, build transaction, sign, submit to SWQOS lane (fire-and-forget) |
| **Core N+** | SWQOS sender threads | N dedicated threads | HTTP/QUIC senders — use `swqos_cores_from_end` to avoid contention |

On >=8 core machines, the SWQOS sender threads should take the **last N cores** (configured via `swqos_cores_from_end = true` in `TradeConfig`).

### 10.2 What to Measure on Each Core

| Metric | Tool | What it tells you |
|---|---|---|
| CPU utilization (%) | `top -pid` / `htop` | Is the core saturated? Target < 80% |
| Context switches | `perf stat -e context-switches` | Unnecessary wakeups; target < 100/s per pinned thread |
| CPU migrations | `perf stat -e cpu-migrations` | Thread moved away from pinned core → pinning broken |
| Involuntary wait time | `pidstat -w -p <pid> 1` | Thread is being preempted → need realtime priority |
| Cache misses (L1/LLC) | `perf stat -e L1-dcache-load-misses,LLC-load-misses` | Data structure not fitting in cache; optimize layout |
| Branch mispredictions | `perf stat -e branch-misses` | Random decision branches; use BranchOptimizer::likely/unlikely |
| Page faults | `perf stat -e page-faults` | Heap allocation; target zero on hot path after warm-up |
| Steal time (VM only) | `/proc/stat:steal` | Hypervisor overcommit; move to dedicated instance |
| Allocator pressure | `dhat` / `stats_alloc` | Number of allocations per invocation |
| Lock contention | `locks` / `perf lock` | Async Mutex on hot path is a violation |

### 10.3 Hot-Path Audit: Allocation and Performance Risks

From the `src/perf/` directory analysis (all 10 files reviewed):

| File | Risk Level | Finding |
|---|---|---|
| `compiler_optimization.rs` | 🟢 Low | Config structs only; no hot-path impact. `generate_ultra_performance_config()` is startup-only |
| `hardware_optimizations.rs` | 🟡 Medium | SIMD memory ops are safe. `CacheOptimizedRingBuffer` uses `Vec<T>` internally — allocation on `new()` only. `BranchOptimizer::unlikely()` uses `cold` function call which may interfere with inlining |
| `kernel_bypass.rs` | 🟠 Medium-High | `KernelBypassUDP` uses tokio::spawn and `Instant::now().elapsed()` on hot path; `send_packet_zero_copy` calls `anyhow!` on error (format! allocation). `start_tx_thread` uses `tokio::task::yield_now()` in a spin loop |
| `simd.rs` | 🟢 Low | Static methods only, no allocations. Tests are safe |
| `syscall_bypass.rs` | 🟠 Medium | `FastTimeProvider` caches time with 1ms refresh — good. But `SyscallBatchProcessor` stores `Vec<u8>` in `SyscallRequest::Write` which allocates on every request. `IOOptimizer::create_memory_mapped_io` runs a shell command (`uname -r`) via `std::process::Command` — startup only |
| `ultra_low_latency.rs` | 🔴 High | `LockFreeEventDispatcher::dispatch_event_ultra_fast` uses `Instant::now()` on every dispatch (violates contract §2.3). `PrefetchOptimizer` clones `EventMessage` on prefetch. `ZeroAllocSerializer` pool is good but unused |
| `zero_copy_io.rs` | 🟡 Medium | `SharedMemoryPool::allocate_block` has a CAS retry loop — safe but may spin under contention. `ZeroCopyBlock` uses `NonNull` so no allocation |
| `realtime_tuning.rs` | 🟡 Medium | Calls `std::process::Command` to run shell commands — startup only. `apply_all_optimizations` is async but safe. |
| `protocol_optimization.rs` | 🔴 High | Uses `format!()` on cache key lookup in `serialize_event_unchecked`. `format!` allocates a String on the hot path — directly violates §2.3. `bincode::serialize` allocates a `Vec<u8>` on each call. Enables `skip_integrity_checks` which is a correctness risk |
| `execution.rs` | 🟢 Low | This file is clean. `Prefetch::instructions` uses `#[inline(always)]`, no allocations. `InstructionProcessor::preprocess` is O(1). `ExecutionPath::is_buy` does constant-time Pubkey comparison |

### 10.4 Audit: `src/trading/core/execution.rs` — No Violations Found

The file at `src/trading/core/execution.rs` (146 lines) does **NOT** contain hot-path violations:

| Function | Analysis |
|---|---|
| `Prefetch::instructions()` | Takes `&[Instruction]`, reads first/middle/last element address for `_mm_prefetch` — no allocation, no I/O, no blocking |
| `Prefetch::pubkey()` / `Prefetch::keypair()` | Same pattern, safe |
| `MemoryOps::copy()` / `compare()` / `zero()` | SIMD wrappers, safe |
| `InstructionProcessor::preprocess()` | Branch-optimized empty check, prefetch, length warning (cold path) — safe |
| `InstructionProcessor::calculate_size()` | Iterator with prefetch; no allocation — safe |
| `ExecutionPath::is_buy()` | Constant-time Pubkey equality; no allocation — safe |
| `ExecutionPath::select()` | Branch-optimized closure dispatch — safe |

### 10.5 Audit: `src/trading/common/` — Low Risk

| File | Finding |
|---|---|
| `compute_budget_manager.rs` | Uses `DashMap` cache hit → returns `Arc<SmallVec<[Instruction; 2]>>`. On cache **miss**, `SmallVec` is stack-allocated (2 instructions, fits in register). No heap allocation on hit. Good. |
| `transaction_builder.rs` | `build_transaction` does one `Vec::with_capacity()` for instructions — needed allocation. `build_versioned_transaction` calls `acquire_builder()` from pool (reuse). `bincode::serialized_size` is the serialization check. Overall acceptable. |

### 10.6 Hot-Path Violations Found in `src/perf/`

| # | File | Line(s) | Violation | Severity |
|---|---|---|---|---|
| 1 | `ultra_low_latency.rs` | 282 | `Instant::now()` in `dispatch_event_ultra_fast` | 🔴 High — §2.3 mandates `HighPerformanceClock`, not `Instant` |
| 2 | `ultra_low_latency.rs` | 85 | `event.clone()` in `PrefetchOptimizer::prefetch_event_data` | 🟠 Medium — clones `EventMessage` which may include heap data |
| 3 | `protocol_optimization.rs` | 151 | `format!()` in `serialize_event_unchecked` cache key | 🔴 High — allocates String on hot path (§2.3) |
| 4 | `protocol_optimization.rs` | 258 | `bincode::serialize(event)` allocates `Vec<u8>` | 🟠 Medium — allocation on serialization path |
| 5 | `protocol_optimization.rs` | 278 | `serde_json::to_string(event)` allocates String | 🟠 Medium — JSON serialization on hot path |
| 6 | `kernel_bypass.rs` | 176 | `Instant::now().elapsed()` in packet descriptor | 🟠 Medium — should use fast timing |
| 7 | `kernel_bypass.rs` | 424-425 | `tokio::task::yield_now()` in TX thread spin loop | 🟡 Low — busy-wait pattern under contention |

### 10.7 Mitigation Plan

| Violation | Fix |
|---|---|
| `Instant::now()` → use `fast_timing::fast_now_nanos()` | Replace all `Instant::now()` calls in hot path with the syscall-bypass timer |
| `format!()` on cache key → use integer hash | Encode cache key as `u64` or `(u64, u64)` tuple; avoid String allocation entirely |
| `event.clone()` in prefetcher → use `Arc<EventMessage>` | Change `ArrayQueue<EventMessage>` to `ArrayQueue<Arc<EventMessage>>` |
| `bincode::serialize` → use pre-allocated buffer | Use `bincode::serialize_into` with a pooled `Vec<u8>` writer |
| JSON serialization on hot path → remove | JSON is never needed on the hot path; remove or gate behind cold path |

## 11. Degraded Mode Entry/Exit (from §7 — updated with Health Checks)

### 11.1 Enter Degraded

Enter degraded mode when **any** of:

- Any pipeline queue > 75% capacity for >5 seconds.
- End-to-end latency > 100ms for >5 seconds.
- FEC recovery failure rate > 1% for >30 seconds.
- Kernel UDP drops detected.
- **Risk control: any health check exceeds limit** (see risk-control-matrix.md, section 2.4).
- **Fee engine: prioritization fee data stale > 60s** (oracle down).

### 11.2 Behavior in Degraded

- Stop accepting new trade intents (drain Q4).
- Emit `DEGRADED` status metric.
- Log a single structured warning per degradation event.
- Continue processing events without triggering trades.
- Continue updating state from events.
- Continue fee engine refresh (so it's ready on recovery).

### 11.3 Exit Degraded

Exit degraded mode when **all** of:

- All queue depths < 25% capacity.
- End-to-end latency < 50ms for >5 seconds.
- FEC recovery failure rate < 0.1% for >30 seconds.
- All risk health checks normal for >30 seconds.

### 11.4 Stop Trading

Stop trading entirely when:

- Trade intent queue full for >500ms consecutively → recovery: drain queue, re-evaluate.

## 12. CPU Isolation Best Practices (macOS)

On macOS, `sched_setaffinity` is not available. Instead:

```rust
// Use thread_policy_set for time-constraint scheduling
use thread_policy::*;

fn set_realtime_policy() {
    unsafe {
        let period = mach_timebase_info_t::default();
        let computation = mach_timebase_info_t::default(); // max 100µs per 1ms period
        let constraint = mach_timebase_info_t::default();
        thread_policy_set(
            pthread_self(),
            THREAD_TIME_CONSTRAINT_POLICY,
            &THREAD_TIME_CONSTRAINT_POLICY {
                period: 1_000_000,           // 1ms period
                computation: 100_000,         // 100µs max per period
                constraint: 150_000,          // 150µs deadline
                preemptible: 0,
            },
            THREAD_TIME_CONSTRAINT_POLICY_COUNT,
        );
    }
}
```

## 13. Verification Checklist

Every implementation of the hot path must pass:

- [ ] **Static analysis**: No prohibited imports (`tokio::net`, `reqwest`, `hyper`, `std::fs`, `dns`) in hot-path modules.
- [ ] **Code review**: Every call on the receive thread path is audited against §2 and §9.
- [ ] **Profiling**: `perf stat` shows zero syscalls beyond `recvfrom()` and fast-timing reads on the receive thread.
- [ ] **Allocation tracking**: Zero heap allocations on hot path after warm-up (verified with `dhat`).
- [ ] **Load testing**: At 2x sustained shred rate, no queue exceeds 50% capacity.
- [ ] **No false drops**: Under normal load, queue drops are zero.
- [ ] **Latency budgets**: Each stage latency budget is measured and enforced.
- [ ] **Core pinning**: Each hot-path thread is pinned to a dedicated core.
- [ ] **Realtime scheduling**: Linux: SCHED_FIFO; macOS: THREAD_TIME_CONSTRAINT_POLICY.
- [ ] **Fast timing**: No `Instant::now()` in hot path; all timing uses `fast_timing::fast_now_nanos()`.
- [ ] **No format! or String allocation**: All cache keys use integer/enum tuple hashing.

---

*This contract is enforced by code review, profiling, allocation tracking, and automated load testing. Any code that violates the hot-path contract will be rejected.*

*Last updated: 2026-07-28 — Added CPU/Threading design (§18), hot-path audit of execution.rs and perf/*.rs, expanded FORBIDDEN list, macOS realtime policy guidance, and allocation-tracking verification.*