# Latency Baseline Design

> **Design document** — defines the methodology, benchmarks, and measurement framework for establishing a performance baseline for `sol-trade-sdk`. This is a specification, not a measured results report. Results are collected by running these benchmarks on a target machine.

---

## 1. Hardware Specification (Recording Template)

Every baseline run MUST record the following machine characteristics:

| Property | Where to find |
|---|---|
| CPU model + microarchitecture | `sysctl -n machdep.cpu.brand_string` (macOS) / `lscpu` (Linux) |
| CPU base / max frequency | `sysctl -n hw.cpufrequency` / `lscpu | grep MHz` |
| Core count (physical / logical) | `sysctl -n hw.physicalcpu hw.logicalcpu` |
| L1d / L1i / L2 / L3 cache sizes | `sysctl -n hw.l1dcachesize hw.l1icachesize hw.l2cachesize hw.l3cachesize` |
| RAM size + type + speed | `system_profiler SPHardwareDataType` (macOS) / `dmidecode -t memory` (Linux) |
| NUMA topology | `lscpu | grep NUMA` (Linux) |
| Kernel version | `uname -a` |
| macOS version | `sw_vers -productVersion` |
| Rust compiler version | `rustc --version && rustc --print cfg` |
| Build profile | `--release` flags, LTO setting, target-cpu, codegen-units, panic=abort |
| CPU governor | Linux: `cat /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor` |
| Mitigations | `sysctl -a | grep cpu` or `lscpu | grep Flags` for meltdown/spectre |

### Recommended Build Flags (Design)

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
debug = false
debug-assertions = false
strip = true
```

```bash
RUSTFLAGS="-C target-cpu=native -C link-arg=-fuse-ld=lld"
```

---

## 2. Existing Benchmarks: `test_latency.sh`

### What It Covers

The existing shell script at `test_latency.sh` runs a **PumpFun buy end-to-end** integration test. It:

1. Creates a temporary Rust binary (`examples/pumpfun_buy_test`)
2. Generates a fresh keypair (no balance)
3. Configures 4 SWQOS lanes (Jito, Bloxroute, NextBlock, FlashBlock)
4. Fetches `get_latest_blockhash()` via RPC (synchronous wait)
5. Constructs `TradeBuyParams` with placeholder (zeroed) `PumpFunParams`
6. Calls `client.buy()` and measures end-to-end wall time
7. Reports SDK-level step timings (from SDK logging)
8. Reports serializer pool + transaction builder pool stats

### Metrics It Collects

| Metric | Scope | How |
|---|---|---|
| SDK log step times | Build + submit | Console prints with step labels |
| Build pool utilization | After run | `get_pool_stats()` |
| Serializer pool utilization | After run | `get_serializer_stats()` |
| Submit success/failure | End-to-end | `client.buy()` return value |
| Transaction signature | After submit | `Ok((_, sig))` |

### Limitations (Design Gaps)

| Gap | Impact |
|---|---|
| No microbenchmarks — only end-to-end | Cannot isolate per-stage latency |
| No warm-up iterations | Cold cache results inflate first-run latency |
| No percentile reporting (p50/p95/p99) | Single-run mean hides tail latency |
| No timing before SDK entry | UDP recv → event decode gap not measured |
| No event-source→submission breakdown | Pipeline stages not individually instrumented |
| `PumpFunParams` use default (zeroed) addresses | Not representative of real reserved-reserve data |
| Only PumpFun buy — no sell, no Raydium/Meteora/Bonk | Protocol variance not tested |
| No GC / allocator-pressure measurement | Allocation patterns invisible |
| No CPU counters (cycles, cache misses, branch misses) | perf stat not collected |
| No syscall trace | Hidden blocking calls not detected |
| Blockhash fetch is inside the measured window | RPC latency pollutes the SDK measurement |

---

## 3. Microbenchmark Suite (Design per Section 38)

Each benchmark targets a single pipeline stage. All benchmarks MUST:

- Run on a **dedicated CPU core** (pinned via `sched_setaffinity` or thread_set_self on macOS)
- **Warm up**: 10,000 iterations discarded before measurement
- **Measure**: 100,000 sampled iterations in batches of 1,000
- **Report**: p50, p95, p99, max (absolute nanoseconds), sample count
- **Use fast timing**: `fast_timing::fast_now_nanos()` to avoid `Instant::now()` syscall overhead
- **Record CPU counters**: `perf stat -e cycles,instructions,cache-misses,branch-misses,context-switches` on Linux

### 3.1 Event Decode

```text
Input:  arbitrary shred data or raw instruction bytes (size varies per protocol)
Action: decode instruction discriminator + parse event fields
Output: decoded event struct
```

| Parameter | Value |
|---|---|
| Warm-up iterations | 10,000 |
| Sampled iterations | 100,000 |
| Protocols | PumpFun (create + buy), PumpSwap (swap), Bonk (buyExactIn), Raydium (swapV2), Meteora (swap) |
| Data source | Pre-recorded instruction bytes from mainnet |
| Reporting | ns per decode, decode throughput (events/s) |

### 3.2 Protocol Parse

```text
Input:  decoded event struct
Action: extract protocol-specific parameters (reserves, fee rates, mints, vault addresses)
Output: ProtocolParams variant
```

| Parameter | Value |
|---|---|
| Warm-up | 10,000 |
| Sampled | 100,000 |
| Per protocol | Separate benchmark for PumpFunParams::from_trade, PumpSwapParams::from_trade_with_fee_basis_points, etc. |

### 3.3 PDA Derivation

```text
Input:  program_id + seeds
Action: call find_program_address (or cached version)
Output: Pubkey + bump seed
```

| Parameter | Value |
|---|---|
| Warm-up | 10,000 |
| Sampled | 100,000 |
| PDA types | PumpFun bonding curve, PumpSwap pool V2, user volume accumulator, creator vault, fee sharing config |
| Cache state | Warm (pre-computed, 100K entries), Cold (first compute) — report both |

### 3.4 ATA Derivation

```text
Input:  wallet_address, mint, token_program_id
Action: get_associated_token_address_with_program_id_fast (cached path)
Output: Pubkey
```

| Parameter | Value |
|---|---|
| Warm-up | 10,000 |
| Sampled | 100,000 |
| Variants | Standard ATA, seed-optimized ATA, both with cache hit and cache miss |
| Token programs | TOKEN_PROGRAM, TOKEN_PROGRAM_2022 |

### 3.5 Quote Calculation

```text
Input:  pool reserves (virtual_sol, virtual_token, real_sol, real_token), input amount, fee rate
Action: compute expected output amount with constant-product formula + fee deduction
Output: output amount (u64), minimum output after slippage
```

| Parameter | Value |
|---|---|
| Warm-up | 10,000 |
| Sampled | 100,000 |
| Protocols | PumpFun (virtual curve), PumpSwap, Raydium CPMM, Raydium AMM V4, Meteora |
| Edge cases | Zero reserves, overflow boundary, max input, min output |

### 3.6 Instruction Construction

```text
Input:  protocol-specific parameters + trading parameters
Action: build the Vec<Instruction> for the trade (including ATA instructions, transfer, swap/swap-base-in)
Output: Vec<Instruction>
```

| Parameter | Value |
|---|---|
| Warm-up | 10,000 |
| Sampled | 100,000 |
| Cache state | Instruction cache hot vs cold |
| Instruction cache size | Measure hit rate vs miss penalty |

### 3.7 Transaction Serialization

```text
Input:  VersionedMessage (compiled instructions + address table lookups)
Action: message.serialize() + maybe bincode::serialized_size
Output: Vec<u8>
```

| Parameter | Value |
|---|---|
| Warm-up | 10,000 |
| Sampled | 100,000 |
| With/without ALT | Measure size reduction benefit separately |
| Size classes | Small (2-3 ix), Medium (4-6 ix), Large (7+ ix with ALTs) |

### 3.8 Signing

```text
Input:  message bytes (serialized), Keypair
Action: ed25519_sign (try_sign_message)
Output: Signature
```

| Parameter | Value |
|---|---|
| Warm-up | 10,000 |
| Sampled | 100,000 |
| Note | Keypair stored as Arc<Keypair>; measure the Arc deref + signing |

### 3.9 End-to-End: Event → Signed

```text
Input:  Raw event bytes (from ShredStream / gRPC)
Pipeline: decode → parse → deduplicate → staleness check → state update → strategy → quote → build instructions → serialize → sign
Output: VersionedTransaction (signed)
```

| Parameter | Value |
|---|---|
| Warm-up | 1,000 |
| Sampled | 10,000 |
| Protocols | One run each for PumpFun, PumpSwap, Bonk, Raydium, Meteora |
| Reporting | Per-stage breakdown (instrumented), end-to-end total |

### 3.10 End-to-End: Event → Submit

```text
Input:  Raw event bytes
Pipeline: full pipeline + submit to SWQOS lane
Output: Submitted (signature returned, no confirmation wait)
```

| Parameter | Value |
|---|---|
| Warm-up | 100 |
| Sampled | 1,000 |
| Note | Requires live SWQOS config; mark as integration-only |
| Reporting | Event-to-submit time, submit-to-response time (SWQOS round-trip) |

---

## 4. Warm-Up Policy

| Phase | Duration | Purpose |
|---|---|---|
| **JIT warm** | 1,000 iterations | Trigger any JIT compilation in the Rust runtime |
| **Cache warm** | 10,000 iterations | Fill L1/L2/L3 cache, branch predictor tables |
| **Allocator warm** | 10,000 iterations | Stabilize jemalloc/mimalloc thread cache |
| **Measurement** | 100,000 iterations | Record timing samples |

The warm-up iterations MUST be executed on the same CPU core and thread as the measurement phase. The process MUST run for at least two seconds of warm-up regardless of iteration count (add a minimum wall-time guard).

---

## 5. Sample Count and Run Protocol

| Aspect | Rule |
|---|---|
| Minimum samples | 100,000 per benchmark |
| Batch size | 1,000 samples per `fast_now_nanos()` call pair |
| Runs per benchmark | 3 consecutive runs, report median of the 3 (to account for OS jitter) |
| Outlier rejection | Discard runs where max > 10x p99 (OS throttling, thermal) |
| Thread isolation | No other user-space threads running on the measurement CPU |
| Priority | `SCHED_FIFO` priority 80 on Linux; `thread_policy_set` THREAD_TIME_CONSTRAINT_POLICY on macOS |

---

## 6. Reporting Format (per Benchmark)

```text
## 3.1 Event Decode — PumpFun

| Stat | ns | cycles | instructions | L1-misses | LLC-misses |
|---|---|---|---|---|---|
| p50 | 142 | 425 | 1,024 | 0 | 0 |
| p95 | 198 | 594 | 1,136 | 2 | 0 |
| p99 | 245 | 735 | 1,211 | 8 | 1 |
| max | 1,024 | 3,072 | 2,048 | 24 | 4 |
| samples | 100,000 | — | — | — | — |
| throughput | 7,042,254 events/s | — | — | — | — |
```

Each benchmark reports the above table. The `cycles`, `instructions`, `L1-misses`, and `LLC-misses` columns are collected via `perf stat -e cycles,instructions,l1d-reloads,ll-cache-misses` on Linux only (omitted on macOS).

---

## 7. CPU Performance Counter Collection (Linux Only)

```bash
# Example perf command for a bench binary
perf stat \
  -e cycles,instructions,cache-misses,branch-misses,context-switches,cpu-migrations,page-faults,L1-dcache-loads,L1-dcache-load-misses,LLC-loads,LLC-load-misses \
  ./target/release/bench-binary --bench-name "event_decode_pumpfun"
```

Collected counters are mapped to each benchmark via `perf stat --per-thread` running on the pinned thread.

---

## 8. Allocation Tracking

Each benchmark MUST also report:

```text
Total allocations: 42
Total allocated bytes: 12,864
Allocations per invocation: 0.042
Avg allocated bytes per invocation: 12.86
```

Use `#[global_allocator]` with `dhat` or `statistical` profiling in bench mode, or run with `MALLOC_TRACE=1`. The target is **zero allocations on the hot path** after warm-up.

---

## 9. Latency Budget Allocation (Design Target)

| Stage | Budget (ns) | Current (to be measured) |
|---|---|---|
| UDP recv → enqueue | 500 | TBD |
| Shred decode → slot assembly | 5,000 | TBD |
| FEC recovery | 3,000 | TBD |
| Transaction decode | 1,000 | TBD |
| Program ID filter | 200 | TBD |
| Instruction decode | 300 | TBD |
| Deduplication check | 100 | TBD |
| State update | 500 | TBD |
| Strategy evaluation | 1,000 | TBD |
| Quote calculation | 500 | TBD |
| Instruction construction | 500 | TBD |
| Transaction build | 800 | TBD |
| Signing | 5,000 | TBD |
| SWQOS submit | 15,000 | TBD |
| **Total event→submitted** | **33,500** | **TBD** |

Target: **< 50 µs** from event decode → signed transaction.
Target: **< 200 µs** end-to-end from UDP event → SWQOS submit.

---

## 10. Benchmark Implementation Guidelines

```rust
// Pseudo-API for each benchmark
pub struct BenchmarkResult {
    pub name: &'static str,
    pub warm_up_iterations: u64,
    pub sampled_iterations: u64,
    pub p50_ns: f64,
    pub p95_ns: f64,
    pub p99_ns: f64,
    pub max_ns: u64,
    pub allocations: u64,
    pub allocated_bytes: u64,
}

pub trait LatencyBenchmark {
    fn name(&self) -> &'static str;
    fn setup(&mut self);                            // Prepare input data
    fn warm_up(&mut self, iterations: u64);         // Discarded warm-up loop
    fn run_iteration(&mut self);                    // Single measured iteration (no alloc inside)
    fn run(&mut self) -> BenchmarkResult;           // Orchestrate warm + measure + report
}
```

Each benchmark lives in a dedicated `benches/` directory with a `#[divan]` or `#[criterion]` harness, using a custom `LatencyRecorder` that wraps `fast_timing::fast_now_nanos()`.

---

## 11. Allocation-Free Guarantee

After warm-up, the hot-path stages (decode → sign) MUST produce **zero heap allocations** per invocation. Any allocation detected during measurement is a regression and MUST be flagged.

**Enforcement:** run benchmarks with `#[global_allocator]` that counts allocations and panics on allocation during measured iterations.

---

*This document defines what to measure and how. Results are collected by running the benchmark suite on the target machine and recording the tables defined above.*