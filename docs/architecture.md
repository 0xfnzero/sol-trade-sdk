# Architecture — sol-trade-sdk

> **Status:** Design reference  
> **Audit commit:** `8e92f8f`  
> **Rust toolchain:** `1.82.0` (pinned via rust-toolchain.toml)  

## 1. Crate Hierarchy

```
sol-trade-sdk/                     # lib crate (rlib)
  ├── src/                         # Core library
  │   ├── common/                  # Shared utilities (clock, timing, logging, config)
  │   ├── constants/               # Protocol constants, program IDs, fee config
  │   ├── instruction/             # DEX instruction builders
  │   │   ├── utils/               # Per-protocol helpers (pumpfun, pumpswap, raydium, etc.)
  │   │   └── (per-protocol).rs    # Instruction construction + tests
  │   ├── trading/                 # Application-level trading logic
  │   │   ├── core/                # Execution engine, traits, params, compute budget
  │   │   └── examples/            # SDK usage examples
  │   ├── utils/                   # Calculation utilities (fees, reserves, slippage)
  │   ├── swqos/                   # 18 SWQoS transaction submission providers
  │   │   ├── common.rs            # Shared submission traits (FormatBase64VersionedTransaction)
  │   │   ├── astralane*.rs        # Astralane QUIC + HTTP providers
  │   │   ├── jito*.rs             # Jito submission + bundle providers
  │   │   ├── node1*.rs            # Node1 QUIC provider
  │   │   ├── glaive*.rs           # Glaive providers
  │   │   └── ...                  # Other providers
  │   └── perf/                    # Performance optimization modules
  │       ├── ultra_low_latency.rs # Lock-free dispatcher, CPU affinity
  │       ├── kernel_bypass.rs     # Ring buffer TX/RX
  │       ├── simd.rs              # SIMD optimizations
  │       ├── syscall_bypass.rs    # Syscall-bypass timing
  │       └── zero_copy_io.rs      # Zero-copy I/O
  ├── examples/                    # 18 example binaries (trading clients, snipers, etc.)
  ├── docs/                        # Design documentation
  ├── artifacts/                   # Audit artifacts, matrices, reports
  ├── runbooks/                    # 11 incident response runbooks
  ├── .github/workflows/           # CI/CD pipelines
  ├── deny.toml                    # Supply chain policy
  └── rust-toolchain.toml          # Pinned Rust version
```

## 2. Data Flow

```
ShredStream UDP
    │
    ▼
Receive Pipeline (CPU 0)
  ├── timestamp (TSC/monotonic)
  ├── validate (packet size, magic bytes)
  ├── deduplicate (slot, shred_index, signature)
  └── bounded SPSC queue (32K cap)
    │
    ▼
Reconstruction (CPU 1)
  ├── FEC recovery
  ├── transaction decoding
  ├── program ID filter (allowlist)
  └── bounded queue (8K cap)
    │
    ▼
State Engine (CPU 2)
  ├── event classification
  ├── local state update
  └── strategy gate
    │
    ▼
Execution (CPU 3)
  ├── trade intent validation
  ├── transaction construction (cached deps)
  ├── signing
  ├── submission (primary provider)
  └── reconciliation loop
```

**Hot-path contract:** The UDP receive must NEVER do RPC calls, disk writes, JSON serialization, strategy calculations, transaction building, signing, submission, or high-volume logging.

## 3. Supported Protocols

| Protocol | Program Type | Instructions | Status |
|---|---|---|---|
| PumpFun | Bonding curve | buy, sell | ✅ Verified (fee 100 bps) |
| PumpSwap | Constant product AMM | buy, sell | ✅ Verified (dynamic fees) |
| Raydium AMM V4 | Constant product | swap_in, swap_out | ✅ Verified (1-byte discriminators) |
| Raydium CPMM | Constant product | swap | ✅ Verified (Anchor 8-byte) |
| Meteora DAMM V2 | Dynamic AMM | swap | ✅ Verified (13+ accounts) |
| Bonk DEX | Constant product | buy, sell | ✅ Verified (15 accounts) |

**Token-2022 policy:** CPMM allows TransferFeeConfig, MetadataPointer, TokenMetadata, InterestBearingConfig, ScaledUiAmount. All others must be rejected unless explicitly whitelisted.

## 4. Submission Providers

18 SWQoS providers available. Production starts with **1 primary + 1 fallback**:

- **Default Solana RPC** (HTTP, normal WebPKI TLS) — safe baseline
- **Jito** (HTTP, bundle support) — accelerated submission

All 6 QUIC providers gated behind `dev-insecure-tls` feature (compile_error in production builds).

## 5. State Machine

```
DETECTED → VALIDATED → BUILT → SIGNED → SUBMITTED → LANDED → SETTLED
    │            │         │        │          │
    ▼            ▼         ▼        ▼          ▼
  REJECTED     EXPIRED   FAILED  CANCELLED   AMBIGUOUS → RECONCILING
```

Every transition records: timestamp, trade-intent ID, signature, slot, blockhash, fees, submission lane, error/evidence.

## 6. Dependency Caches (outside hot path)

- **BlockhashService** — background refresh, age tracking, last_valid_block_height
- **FeeService** — per-route CU distribution, prioritization fee oracle
- **Pool metadata cache** — validated pool state, vaults, token programs
- **ATA cache** — pre-created account addresses
- **ALT cache** — verified address lookup tables
- **Connection pool** — warmed HTTP/QUIC connections

## 7. Risk Controls

31 hard limits across 5 categories:

| Category | Count | Examples |
|---|---|---|
| Per-trade | 8 | max SOL, max token amount, max slippage, min output |
| Global | 8 | max exposure, max tx/min, per-mint cap |
| Fee | 7 | max priority fee, max relay tip, max daily loss |
| Health | 6 | max RPC lag, max packet loss, max blockhash age |
| Switches | 6 | global kill, buy-only, per-protocol disable |

## 8. Observability

Stage-level timestamps on every event:
```
packet_received → reconstruction_complete → transaction_decoded
→ event_classified → dedup_complete → state_updated
→ strategy_complete → intent_validated → transaction_built
→ transaction_signed → lane_submitted → lane_acknowledged
→ landing_observed → settlement_complete
```

Metrics: packets, kernel drops, FEC recoveries, queue depths, stale events, landing rate, P&L.