# sol-trade-sdk — Build Implementation Plan (Phases 1–4)

> **For Hermes:** Execute phases in order. Each phase has discrete commits. Do not skip phases. Do not merge phases. Do not modify code before the reproducibility baseline is locked.

**Goal:** Transform `sol-trade-sdk` from an upstream SDK into a controlled, auditable, measurable, recoverable trading application — starting with the foundational P0/P1 gates.

**Architecture:** Single Rust workspace (20 crates), targeting Ubuntu VPS. All build infrastructure and security fixes before protocol or application code changes.

**Tech Stack:** Rust 1.95.0 (nightly for fmt, stable for build), cargo-audit, cargo-deny, GitHub Actions.

---

## Phase 1 — Reproducibility & Toolchain Baseline

### Task 1.1: Create `rust-toolchain.toml`

**Objective:** Pin Rust version so every build is reproducible.

**Files:**
- Create: `rust-toolchain.toml`

Content:
```toml
[toolchain]
channel = "1.82.0"
components = ["rustfmt", "clippy"]
targets = ["x86_64-unknown-linux-gnu"]
```

Note: Use `1.82.0` which is the current Rust stable as of mid-2026 that's compatible with Solana SDK dependency resolution. Verify with `rustc --version` after setting.

**Verification:** `cargo check` still passes after pinning.

### Task 1.2: Create `source-manifest.md`

**Objective:** Record the exact pinned dependencies and their origin commits per pre-flight §4.1.

**Files:**
- Create: `artifacts/source-manifest.md`

Content: Copy the source lock table from pre-flight.md §4.1 with current verified commits (8e92f8f for `sol-trade-sdk`, plus all upstream references). Add resolution commands for each.

### Task 1.3: Verify `Cargo.lock` is committed

**Objective:** Ensure deterministic builds.

**Files:**
- Verify: `Cargo.lock` exists and is committed

**Verification:** `git ls-files Cargo.lock` should return the path.

---

## Phase 2 — P0/P1 Security Fixes

### Task 2.1: Add `dev-insecure-tls` feature gate to all 6 SWQoS QUIC providers

**Objective:** Make `ServerCertVerified::assertion()` only compile behind an explicit non-default feature flag.

**Reference:** Pre-flight §11, Agent 3 findings (6 TLS bypasses: astralane_quic, node1_quic, soyas, glaive_quic, solami, speedlanding).

**Files:**
- Create or Modify: `Cargo.toml` — add `[features]` section if missing:
```toml
[features]
default = []
production = []
dev-insecure-tls = []
```
- Modify: `src/swqos/astralane_quic.rs` — wrap cert verifier in `#[cfg(feature = "dev-insecure-tls")]`
- Modify: `src/swqos/node1_quic.rs`
- Modify: `src/swqos/soyas.rs`
- Modify: `src/swqos/glaive_quic.rs`
- Modify: `src/swqos/solami.rs`
- Modify: `src/swqos/speedlanding.rs`

Pattern for each:
```rust
#[cfg(not(feature = "dev-insecure-tls"))]
compile_error!("QUIC transport without dev-insecure-tls requires a production TLS verifier. See docs/operations.md");

#[cfg(feature = "dev-insecure-tls")]
{
    // existing ServerCertVerified::assertion() code
}
```

For HTTP-based fallback transports, ensure they use normal WebPKI verification (most already do).

**Verification:** `cargo check --features dev-insecure-tls` succeeds. `cargo check` fails with compile_error on QUIC providers.

### Task 2.2: Fix hot-path panic `params.input_amount.unwrap()` in pumpswap.rs

**Objective:** Convert hot-path panicking unwrap to a recoverable typed error.

**Reference:** A3 finding P0-3.

**Files:**
- Modify: `src/trading/instruction/pumpswap.rs`

The pattern:
```rust
// OLD: let input_amount = params.input_amount.unwrap();
// NEW: let input_amount = params.input_amount.ok_or(TradingError::MissingField("input_amount"))?;
```

Ensure the function returns `Result<_, TradingError>` (or the existing error type) instead of panicking.

**Verification:** `cargo test --workspace` still passes.

### Task 2.3: Fix `bincode::serialize(self).unwrap()` in common.rs

**Objective:** Convert serialization panic to recoverable error.

**Reference:** A3 finding P1-9.

**Files:**
- Modify: `src/common/common.rs`

```rust
// OLD: let data = bincode::serialize(self).unwrap();
// NEW: let data = bincode::serialize(self).map_err(|e| CommonError::Serialization(e.to_string()))?;
```

**Verification:** `cargo test --workspace` passes.

### Task 2.4: Fix `header[0..2].try_into().unwrap()` in node1_quic.rs

**Objective:** Convert response header parsing panic to recoverable error.

**Reference:** A3 finding P1-10.

**Files:**
- Modify: `src/swqos/node1_quic.rs`

```rust
// OLD: let length_bytes: [u8; 2] = header[0..2].try_into().unwrap();
// NEW: let length_bytes: [u8; 2] = header.get(0..2)
//     .ok_or(SwqosError::InsufficientHeader)?
//     .try_into()
//     .map_err(|_| SwqosError::InvalidHeaderLength)?;
```

**Verification:** `cargo test --workspace` passes.

---

## Phase 3 — Protocol Correctness Fixes (P0/P1)

### Task 3.1: Fix Pump.fun `FEE_BASIS_POINTS` from 95 to 100

**Objective:** Correct protocol fee constant to match on-chain behavior.

**Reference:** Pre-flight §14, A4 finding.

**Files:**
- Modify: `src/constants/trade.rs` (or wherever the constant is defined)

```rust
// OLD: pub const FEE_BASIS_POINTS: u64 = 95;
// NEW: pub const FEE_BASIS_POINTS: u64 = 100;
```

**Files to search:**
```bash
grep -rn "FEE_BASIS_POINTS" src/
```

**Verification:** Add a unit test asserting the constant matches 100 (mainnet verified). Also check `cargo test`.

### Task 3.2: Fix PumpSwap `LP_FEE_BASIS_POINTS` static fallback

**Objective:** Ensure the dynamic fee path is always reached (the static 25 bps fallback is only for pre-deployment state).

**Reference:** A4 finding P1-4.

**Files:**
- Modify: The PumpSwap fee resolution function to:
  1. Always query on-chain pool state first
  2. Use static 25 only as a startup/initialization default
  3. Log a warning when using the static fallback

**Verification:** Unit test that verifies `get_fee_basis_points()` returns the cached value when pool state is available.

### Task 3.3: Fix Raydium AMM V4 hardcoded fee to use on-chain `AmmInfo.fees`

**Reference:** A5 finding P2-11.

**Files:**
- Modify: Raydium AMM V4 swap builder to read fee from `AmmInfo.fees.swap_fee` instead of constant.

**Verification:** Unit test with known on-chain AmmInfo fixture.

---

## Phase 4 — CI & Tooling Gates

### Task 4.1: Verify `ci.yml` works (already created by Agent 10)

**Objective:** Ensure the 6-job CI pipeline (fmt, check, clippy, test, audit, deny) passes on the current codebase.

**Note:** `cargo clippy --all-features` will fail on the existing codebase (warnings exist). Gate: verify the CI file structure is correct. Suppress or fix warnings in a separate commit.

### Task 4.2: Create `docs/architecture.md`

**Objective:** Document the high-level architecture per pre-flight §48.

**Files:**
- Create: `docs/architecture.md`

Covers: crate hierarchy, data flow, hot path, protocol matrix overview, submission providers, state engine, observability.

### Task 4.3: Create `docs/configuration.md`

**Objective:** Document all configuration groups per pre-flight §42.

**Files:**
- Create: `docs/configuration.md`

Covers: network, RPC, shredstream, protocols, submission, wallet, blockhash, fees, risk, strategy, observability, storage, recovery, runtime config schemas.

---

## Implementation Order

```
Phase 1: Reproducibility ─────────────────────── Task 1.1 → 1.2 → 1.3
                                                      │
Phase 2: Security P0/P1 ──────────────────────── Task 2.1 → 2.2 → 2.3 → 2.4
                                                      │
Phase 3: Protocol Correctness P0/P1 ──────────── Task 3.1 → 3.2 → 3.3
                                                      │
Phase 4: CI & Docs ───────────────────────────── Task 4.1 → 4.2 → 4.3
```

## Verification

After Phase 4 completion:
```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --all-targets --features dev-insecure-tls -- -D warnings
cargo test --workspace
cargo audit
cargo deny check
```