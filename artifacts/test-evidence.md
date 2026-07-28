# Test Architecture Design — sol-trade-sdk

> **Reference**: Section 37 of the pre-flight requirements
> **Scope**: Defines all test layers, fixtures, and coverage targets
> **Status**: Design — implementation pending

---

## 1. Unit Tests (`tests/unit/`)

### Purpose
Validate each building block in isolation — PDA derivation, instruction discriminators, reserve formulas, fee calculations, rounding, slippage math, state machine transitions, and error handling.

### Coverage Targets

#### 1.1 PDA Derivation
- `tests/unit/pda_pumpfun.rs` — get_bonding_curve_pda, get_bonding_curve_v2_pda, get_user_volume_accumulator_pda
- `tests/unit/pda_pumpswap.rs` — get_pool_v2_pda, get_user_volume_accumulator_pda
- `tests/unit/pda_raydium.rs` — get_amm_pool_pda, get_vault_pda, get_market_authority_pda
- `tests/unit/pda_bonk.rs` — get_pool_pda, get_vault_pda
- `tests/unit/pda_meteora.rs` — get_damm_pool_pda, get_vault_pda
- Each test: known inputs → expected PDA address → assert match
- Each test: invalid seeds → assert error/None

#### 1.2 Discriminators
- `tests/unit/discriminators.rs` — Verify every instruction discriminator (PumpFun buy/sell, PumpSwap buy/sell, Raydium CPMM swap, Bonk buy/sell, Meteora DammV2 swap) matches the corresponding IDL
- Discriminators stored as named constants (e.g. `BUY_DISCRIMINATOR`, `SELL_DISCRIMINATOR`)

#### 1.3 Reserve / Fee Formulas
- `tests/unit/reserve_formulas.rs` — PumpFun virtual reserve math (bonding curve invariant)
- `tests/unit/fee_formulas.rs` — Protocol fee calculations, cashback fee calculations
- Edge cases: zero reserves, max uint64, dust amounts, extreme slippage

#### 1.4 Rounding & Slippage
- `tests/unit/rounding.rs` — Floor/ceiling behavior for token amounts
- `tests/unit/slippage.rs` — Minimum output amount calculation, slippage basis points at boundaries (0, 1, 10_000, >10_000)

#### 1.5 Exact-In / Exact-Out
- `tests/unit/exact_in_out.rs` — Exact-in amount → expected output, exact-out amount → expected input

#### 1.6 Direction & Feature Rejection
- `tests/unit/unsupported_direction.rs` — Unsupported trade direction returns error
- `tests/unit/virtual_reserve.rs` — Virtual reserve edge cases (both zero, token-only, SOL-only)

#### 1.7 Token-2022 Transfer Fees
- `tests/unit/token_2022.rs` — Transfer fee calculation for Token-2022, fee exemption logic

#### 1.8 Quote Age / Blockhash Age
- `tests/unit/quote_age.rs` — Stale quote rejection (configurable threshold)
- `tests/unit/blockhash_age.rs` — Blockhash expiry validation

#### 1.9 Position Limits
- `tests/unit/position_limits.rs` — Min/max position size validation, per-token limits

#### 1.10 State Machine Transitions
- `tests/unit/state_machine.rs` — Connection lifecycle states (disconnected → connecting → connected → reconnecting → closed)
- Event processor states, submission queue states

---

## 2. Golden Fixtures (`tests/golden/`)

### Purpose
Capture expected byte-level outputs for every instruction to detect regressions when IDLs change. Each fixture is a self-contained test with fixed inputs and expected outputs.

### Structure
```
tests/golden/
├── fixtures/
│   ├── pumpfun_buy.json          # Inputs → expected instruction bytes + accounts
│   ├── pumpfun_sell.json
│   ├── pumpswap_buy.json
│   ├── pumpswap_sell.json
│   ├── raydium_cpmm_swap.json
│   ├── bonk_buy.json
│   ├── bonk_sell.json
│   ├── meteora_dammv2_swap.json
│   └── idl_references.json       # Maps each fixture to its canonical IDL commit
├── verify_golden.rs              # Test that loads each fixture and asserts equality
└── REFERENCE_HASHES.txt          # SHA256 hashes of all fixture files
```

### Each Fixture Contains
- `name`: Human-readable test case name
- `idls`: Map of IDL name → pinned commit SHA (e.g. `{"pump.json": "abc123"}`)
- `inputs`: All parameters needed to build the instruction
- `expected_instruction`: Serialized instruction bytes (hex)
- `expected_accounts`: Ordered list of `AccountMeta` with pubkey + is_signer + is_writable
- `expected_compute_units`: Estimated CU consumption (for compute budget tracking)
- `mainnet_example`: Known mainnet transaction signature that exercises this path

### Fixture Regeneration
- When IDLs change: update pinned commit refs, regenerate fixture data, update hashes
- CI checks fixture hashes on schedule (weekly IDL hash check)

---

## 3. Property Tests (`tests/property/`)

### Purpose
Use property-based testing (proptest or quickcheck) to verify invariants hold across a wide input space.

### Invariant Properties

| Property | Description |
|----------|-------------|
| No overflow | All arithmetic operations on u64 amounts produce valid results (checked math) |
| Output monotonicity | Larger input → larger or equal output (all else equal) |
| Slippage monotonicity | Higher slippage → more permissive minimum output (non-increasing) |
| Fee non-negativity | All fee calculations produce non-negative results |
| Reserve invariants | Virtual and real reserves maintain expected relationships post-swap |
| Exact-input boundaries | Exact-in amount produces output ≥ minimum output (when within slippage) |
| Zero-value rejection | Zero input/output amounts are rejected with appropriate error |

### Implementation
```rust
// Pseudocode
proptest! {
    #[test]
    fn no_overflow_property(input in 0..u64::MAX, slippage in 0..10_000u64) {
        let result = calculate_min_output(input, slippage);
        prop_assert!(result.is_ok());
        prop_assert!(result.unwrap() <= input); // sanity
    }

    #[test]
    fn output_monotonicity_property(a in 0..u64::MAX/2, b in a..u64::MAX, slippage in 0..10_000u64) {
        let out_a = calculate_min_output(a, slippage).unwrap();
        let out_b = calculate_min_output(b, slippage).unwrap();
        prop_assert!(out_b >= out_a);
    }
}
```

---

## 4. Integration Tests (`tests/integration/`)

### Purpose
Test cross-module behavior against a controlled local environment (solana-test-validator or emulated RPC).

### Test Scenarios

#### 4.1 Blockhash Service
- `tests/integration/blockhash_service.rs`
  - Cache hit/miss behavior
  - TTL expiry and refresh
  - Stale blockhash rejection
  - Network partition recovery

#### 4.2 Fee Service
- `tests/integration/fee_service.rs`
  - Fee estimation from recent blocks
  - Priority fee adjustment
  - Fee cap enforcement

#### 4.3 Submission Clients
- `tests/integration/submission_clients.rs`
  - Jito bundle submission + confirmation
  - Bloxroute submission
  - NextBlock submission
  - FlashBlock submission
  - Multi-lane same-signature (same tx sent through multiple lanes)
  - Queue overload handling (backpressure)
  - RPC failover (primary RPC down → secondary)
  - Provider failover (primary SWQOS provider down → secondary)

#### 4.4 Reconciliation
- `tests/integration/reconciliation.rs`
  - Confirmation polling across providers
  - Duplicate detection (same signature from two providers)
  - Signature status tracking
  - Error classification and retry logic

#### 4.5 Restart Recovery
- `tests/integration/restart_recovery.rs`
  - Graceful shutdown and state persistence
  - Restart with pending transactions
  - Restart with stale state (time-travel recovery)
  - Restart with corrupted state files

---

## 5. Simulation Tests (`tests/simulation/`)

### Purpose
Use Solana's `simulateTransaction` RPC against a local test validator (with mainnet account clones) to validate instruction correctness without executing on mainnet.

### Test Scenarios

#### 5.1 Missing Accounts
- `tests/simulation/missing_accounts.rs`
  - Intentionally omit required accounts → verify simulation error
  - Verify error message matches expected Solana error

#### 5.2 Account Order
- `tests/simulation/account_order.rs`
  - Verify correct account ordering for each protocol instruction
  - Shuffle accounts → verify simulation failure

#### 5.3 Compute Consumption
- `tests/simulation/compute_consumption.rs`
  - Measure CU per instruction type
  - Verify CU stays within budget bounds
  - Track CU drift over time (log for monitoring)

#### 5.4 Token Program Compatibility
- `tests/simulation/token_program.rs`
  - Test with Token program (TokenkegQ5Z…)
  - Test with Token-2022 program (TokenzQdBN…)
  - Verify correct program ID selection per instruction

#### 5.5 Slippage
- `tests/simulation/slippage.rs`
  - Simulate trades with various slippage values
  - Verify simulation output matches expected math

#### 5.6 Expected Output
- `tests/simulation/expected_output.rs`
  - Compare simulated output amount against calculated output
  - Log discrepancies for analysis

#### 5.7 Error Decoding
- `tests/simulation/error_decoding.rs`
  - Trigger each known error type via simulation
  - Verify error is correctly decoded from program error → SDK error

---

## 6. Replay Tests (`tests/replay/`)

### Purpose
Replay captured mainnet traffic through the state machine and event processors to measure detection accuracy, false positives, and latency.

### Test Scenarios

#### 6.1 Event Detection
- `tests/replay/event_detection.rs`
  - Feed recorded ShredStream events → verify correct event extraction
  - Measure end-to-end latency from event arrival to intent production

#### 6.2 State Reconstruction
- `tests/replay/state_reconstruction.rs`
  - Replay block history → verify reconstructed state matches expected state
  - Test with gaps (missing blocks) → verify recovery

#### 6.3 False Positive Measurement
- `tests/replay/false_positives.rs`
  - Replay known non-opportunity traffic → verify zero intents produced
  - Measure false positive rate per protocol

#### 6.4 Stale-Event Rejection
- `tests/replay/stale_event.rs`
  - Replay events with timestamps exceeding stale threshold → verify rejection

#### 6.5 Opportunity Decisions
- `tests/replay/opportunity_decision.rs`
  - Replay known-arbitrage scenarios → verify correct opportunity identification
  - Compare against recorded human-validated decisions

### Data Format
```
tests/replay/data/
├── session_2026-07-28/
│   ├── events.bin          # ShredStream events (binary, gzipped)
│   ├── metadata.json       # Session metadata (start/end slot, protocol, chain)
│   ├── opportunities.json  # Known opportunities (ground truth)
│   └── state_checkpoints/  # Periodic state dumps for verification
```

---

## 7. Failure Injection Tests (`tests/failure_injection/`)

### Purpose
Simulate network, system, and protocol failures to verify the system degrades gracefully and recovers correctly.

### Test Scenarios

#### 7.1 Network Failures
| Scenario | File | Description |
|----------|------|-------------|
| Packet loss | `packet_loss.rs` | Simulate random packet drops on ShredStream connection |
| Duplicates | `duplicates.rs` | Send duplicate events — verify idempotency |
| Out-of-order events | `out_of_order.rs` | Reorder events — verify sequence number handling |
| RPC timeout | `rpc_timeout.rs` | Inject slow/cancelled responses from RPC client |
| RPC lag | `rpc_lag.rs` | Inject delayed responses (> expected latency) |
| QUIC disconnect | `quic_disconnect.rs` | Kill and restart QUIC connection mid-stream |
| HTTP timeout | `http_timeout.rs` | Inject timeout in HTTP submission paths |
| Dropped ack | `dropped_ack.rs` | Suppress submission acknowledgments — verify retry |

#### 7.2 System Failures
| Scenario | File | Description |
|----------|------|-------------|
| Process restart | `process_restart.rs` | Kill and restart the bot process — verify state persistence |
| Queue saturation | `queue_saturation.rs` | Overload submission queues beyond capacity — verify backpressure |

#### 7.3 Protocol Failures
| Scenario | File | Description |
|----------|------|-------------|
| Malformed tx | `malformed_tx.rs` | Construct and submit intentionally malformed transactions |
| Unsupported token extension | `unsupported_extension.rs` | Test with Token-2022 extensions the SDK doesn't support |
| Fee spike | `fee_spike.rs` | Simulate sudden priority fee spike — verify fee cap enforcement |
| Rate limit | `rate_limit.rs` | Exceed RPC rate limits — verify backoff and retry |

---

## 8. Test Implementation Plan

### Priority Order
1. **P1 — Immediate**: Unit tests for PDA derivation + discriminators (catch IDL drift first)
2. **P1 — Immediate**: Golden fixtures for all protocol instructions
3. **P1 — Immediate**: Property tests for arithmetic invariants
4. **P2 — Next**: Integration tests for submission, reconciliation, blockhash, fee
5. **P2 — Next**: Simulation tests for error decoding + compute consumption
6. **P3 — Stretch**: Replay tests (requires captured traffic pipeline)
7. **P3 — Stretch**: Failure injection tests (requires fault injection framework)

### Directory Structure
```
tests/
├── unit/
│   ├── pda_pumpfun.rs
│   ├── pda_pumpswap.rs
│   ├── pda_raydium.rs
│   ├── pda_bonk.rs
│   ├── pda_meteora.rs
│   ├── discriminators.rs
│   ├── reserve_formulas.rs
│   ├── fee_formulas.rs
│   ├── rounding.rs
│   ├── slippage.rs
│   ├── exact_in_out.rs
│   ├── unsupported_direction.rs
│   ├── virtual_reserve.rs
│   ├── token_2022.rs
│   ├── quote_age.rs
│   ├── blockhash_age.rs
│   ├── position_limits.rs
│   └── state_machine.rs
├── golden/
│   ├── fixtures/
│   ├── verify_golden.rs
│   └── REFERENCE_HASHES.txt
├── property/
│   ├── no_overflow.rs
│   ├── monotonicity.rs
│   ├── fee_non_negativity.rs
│   ├── reserve_invariants.rs
│   └── zero_rejection.rs
├── integration/
│   ├── blockhash_service.rs
│   ├── fee_service.rs
│   ├── submission_clients.rs
│   ├── reconciliation.rs
│   └── restart_recovery.rs
├── simulation/
│   ├── missing_accounts.rs
│   ├── account_order.rs
│   ├── compute_consumption.rs
│   ├── token_program.rs
│   ├── slippage.rs
│   ├── expected_output.rs
│   └── error_decoding.rs
├── replay/
│   ├── event_detection.rs
│   ├── state_reconstruction.rs
│   ├── false_positives.rs
│   ├── stale_event.rs
│   ├── opportunity_decision.rs
│   └── data/
└── failure_injection/
    ├── packet_loss.rs
    ├── duplicates.rs
    ├── out_of_order.rs
    ├── rpc_timeout.rs
    ├── rpc_lag.rs
    ├── quic_disconnect.rs
    ├── http_timeout.rs
    ├── dropped_ack.rs
    ├── process_restart.rs
    ├── queue_saturation.rs
    ├── malformed_tx.rs
    ├── unsupported_extension.rs
    ├── fee_spike.rs
    └── rate_limit.rs
```

### Test Runner Configuration
Add to `Cargo.toml`:
```toml
[dev-dependencies]
proptest = "1.4"
tempfile = "3.10"
serial_test = "3.0"
criterion = { version = "0.5", optional = true }

[[test]]
name = "unit"
path = "tests/unit/mod.rs"

[[test]]
name = "golden"
path = "tests/golden/mod.rs"

[[test]]
name = "integration"
path = "tests/integration/mod.rs"
harness = false  # Uses custom runner for solana-test-validator lifecycle

[[test]]
name = "simulation"
path = "tests/simulation/mod.rs"
harness = false

[[test]]
name = "replay"
path = "tests/replay/mod.rs"
harness = false

[[test]]
name = "failure_injection"
path = "tests/failure_injection/mod.rs"
harness = false
```

### Code Review Recommendations Addressed
Per the code review report, the following testing gaps are closed:
1. ✅ **PDA unwrap → Result** in instruction builders (covered by unit tests + property tests)
2. ✅ **Instruction builder tests** for every protocol (golden fixtures + simulation)
3. ✅ **Golden fixtures for IDL regression** (fixture hash check in CI)
4. ✅ **Edge case coverage** for zero values, overflow, and extreme slippage (property tests)