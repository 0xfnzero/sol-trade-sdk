# Security Review — sol-trade-sdk

**Date:** 2026-07-28  
**Scope:** `src/`, `examples/`, `Cargo.toml`, git history  
**Reviewer:** Hermes Agent (automated audit)

---

## A. Credential Scan

### A.1 Env-var-based credential loading

All 14 example binaries load the private key via the library function:

```rust
sol_trade_sdk::common::keypair::load_keypair_from_env("PRIVATE_KEY")?
```

**`src/common/keypair.rs`** — well-designed: parses base58 or 64-byte JSON array, returns `Result<Keypair>` (never panics). Tests properly reject placeholders and wrong-length inputs. **No live secrets** embedded in source.

### A.2 Pattern scan results

| Pattern | Occurrences | Risk |
|---------|------------|------|
| `PRIVATE_KEY` | 14 example `load_keypair_from_env()` calls | **Low** — env-var loading, no hardcoded keys |
| `API_KEY` | 9 files, mostly `astralane_quic.rs` (error code enum `UNKNOWN_API_KEY`) | **Low** — used as error code names, not literals |
| `AUTH_TOKEN` | 12 occurrences in `common.rs` HTTP header usage, example `GRPC_AUTH_TOKEN` env reads | **Low** — passed via env vars at runtime |
| `GRPC_AUTH_TOKEN` | 5 example mains read from `std::env::var()` | **Low** — dynamic loading |
| `JITO_` | `JITO_TIP_ACCOUNTS` in constants + jito.rs | **Low** — hardcoded tip accounts (public keys) |
| `SHREDSTREAM` | 0 matches | **N/A** |
| `SECRET_KEY` | 0 matches | **N/A** |
| `SEED_PHRASE` | 0 matches (only PDA seeds like `"bonding-curve"`, `"pool"` etc.) | **N/A** |
| `RPC_URL` | ~30 matches, all env-var reads | **Low** |
| base58 private key regex `[5-9][1-9A-HJ-NP-Za-km-z]{87,88}` | 0 matches | **N/A** |

### A.3 Hardcoded credential-like constants — **FINDING (Medium)**

**`src/swqos/temporal.rs`** contains hardcoded partial API key fingerprint matching:

```rust
const SPECIAL_API_KEY_PREFIX: &str = "298b5025";
const SPECIAL_API_KEY_SUFFIX: &str = "a055323";
const SPECIAL_API_KEY_HASH: &str = "e7be933c8058aebcb4d08a6120fb4dfd2ead568d42527a3fc2b60a703f25e48d";
```

These are used at runtime (line 68-76) to detect a specific "community" API key and route to a different tip account. While obfuscated via prefix/suffix split + SHA-256 hash, the hash itself is a fixed string in the binary — a determined attacker can reverse this to identify the key pattern. **Recommendation:** Remove the hash-based community-key detection or gate it behind a feature flag, and use a config-based approach instead.

### A.4 `.env` files and shell history

- **No `.env` files** found in the repository
- **No `.pem`, `.key`, `id_*`, or `credentials*` files** found
- Example README files document `export PRIVATE_KEY=your_base58_private_key` as user-run steps — acceptable

### A.5 Git log credential search

- `git log -p --all -- .env '*.json' '*.toml'` returned IDL JSON files and `Cargo.toml` additions only
- **No credentials or secrets found in git history**

**Credential Scan Verdict:** No live secrets exposed. The `temporal.rs` partial-key / hash approach is the only concern — minor information disclosure risk.

---

## B. TLS/QUIC Security Audit

### B.1 Summary of SkipServerVerification implementations

**FIVE independent SkipServerVerification implementations exist across the codebase.** All unconditionally accept any server certificate via `ServerCertVerified::assertion()`.

| File | Implementation | Method |
|------|---------------|--------|
| `src/swqos/astralane_quic.rs` | Local `SkipServerVerification` struct (lines 315-360) | `.dangerous().with_custom_certificate_verifier(Arc::new(SkipServerVerification))` |
| `src/swqos/node1_quic.rs` | Local `SkipServerVerification` struct (lines 261-306) | `.dangerous().with_custom_certificate_verifier(Arc::new(SkipServerVerification))` |
| `src/swqos/soyas.rs` | Local `SkipServerVerification` struct (lines 30-73) | `.dangerous().with_custom_certificate_verifier(SkipServerVerification::new())` |
| `src/swqos/glaive_quic.rs` | External from `solana_tls_utils::SkipServerVerification` | `.dangerous().with_custom_certificate_verifier(SkipServerVerification::new())` |
| `src/swqos/solami.rs` | External from `solana_tls_utils::SkipServerVerification` | `.dangerous().with_custom_certificate_verifier(SkipServerVerification::new())` |
| `src/swqos/speedlanding.rs` | External from `solana_tls_utils::SkipServerVerification` | `.dangerous().with_custom_certificate_verifier(SkipServerVerification::new())` |

**All six** call `rustls::ClientConfig::builder().dangerous()` to set up custom certificate verification, and **all** unconditionally accept any server certificate.

### B.2 Critical finding — `ServerCertVerified::assertion()` (P0)

Every `verify_server_cert` implementation follows the same pattern:

```rust
fn verify_server_cert(
    &self,
    _end_entity: &CertificateDer<'_>,
    _intermediates: &[CertificateDer<'_>],
    _server_name: &rustls::pki_types::ServerName<'_>,
    _ocsp_response: &[u8],
    _now: rustls::pki_types::UnixTime,
) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
    Ok(rustls::client::danger::ServerCertVerified::assertion())
}
```

All parameters are IGNORED. This means:
- No chain-of-trust validation
- No hostname verification
- No certificate expiry checking
- No revocation checking
- **Any MITM proxy or attacker with UDP access to the path can impersonate the QUIC server**

The code comments acknowledge this: `/// Skip server certificate verification (Astralane server may use self-signed cert).`

### B.3 Risk assessment

This is an **accepted architectural risk** for QUIC-based SWQOS providers:

| Provider | Protocol | TLS Verification | Risk |
|----------|----------|-----------------|------|
| Astralane (QUIC) | QUIC over UDP/7000-9000 | **None — `ServerCertVerified::assertion()`** | P0 — any MITM |
| Node1 (QUIC) | QUIC over UDP/16666 | **None — `ServerCertVerified::assertion()`** | P0 — any MITM |
| Soyas (QUIC) | QUIC | **None — `ServerCertVerified::assertion()`** | P0 — any MITM |
| Glaive (QUIC) | QUIC over UDP/4000 | **None — via `solana_tls_utils`** | P0 — any MITM |
| Solami (QUIC) | QUIC | **None — via `solana_tls_utils`** | P0 — any MITM |
| Speedlanding (QUIC) | QUIC | **None — via `solana_tls_utils`** | P0 — any MITM |
| Non-QUIC SWQOS (HTTP) | HTTPS via `reqwest` | **Full WebPKI** (default reqwest behavior) | Low |

**Important context:** These providers deliberately use self-signed certs and authenticate via client-side mTLS certificates (API key as CN for Astralane, Ed25519 dummy X.509 for the others). The server identity is NOT verified by PKI — trust is established by:
1. Hardcoded IP addresses (`astralane_quic.rs` lines 57-81)
2. DNS resolution
3. Post-QUIC application-layer authentication frames (Glaive: UUID auth frame; Node1: 16-byte UUID exchange)

**Recommendation:** If the SWQOS providers ever deploy WebPKI-validated certificates, remove `SkipServerVerification` and use standard TLS verification. For now, document this as **accepted-risk: trust-on-first-use via hardcoded IPs and application-layer auth**.

### B.4 SWQOS `common.rs` TLS configuration

`src/swqos/common.rs` configures HTTP clients (reqwest) only — no custom TLS overrides. HTTP clients use system root CA store by default. **No issue.**

### B.5 `node1_quic.rs` — also uses `with_no_client_auth()`

Unlike the other QUIC clients that use mTLS client certs, `node1_quic.rs` (line 87) uses `.with_no_client_auth()`. Combined with `SkipServerVerification`, the Node1 QUIC channel has **no certificate verification on either side** — pure application-layer UUID auth is the only protection.

---

## C. Panic / Unwrap / Expect Audit

### C.1 Aggregate counts (all `src/`)

| Pattern | Count |
|---------|-------|
| `.unwrap()` | **305** |
| `.expect()` | **14** |
| `panic!()` | **1** (in `client/mod.rs` — WSOL ATA failure, startup-only) |
| `unreachable!()` | **2** (in `hardware_optimizations.rs` — compile-time CPU arch) |
| `todo!()` | **0** |
| `unimplemented!()` | **0** |
| **Total** | **322** |

### C.2 Classification by category

#### Compile-time invariant (Safe — 0 risk)
- `.unwrap()` on `std::sync::OnceLock`, `LazyLock`, `OnceCell`, `AtomicUsize` ordering
- `unreachable!()` in `hardware_optimizations.rs` — matches CPU arch at compile time
- `.expect()` in `constants/swqos.rs` — `u16::try_from()` on compile-time-known values
- `.expect()` in `gas_fee_strategy.rs` — format string construction
- **~50 occurrences** — acceptable

#### Startup-only / config-time (Low risk)
- `.unwrap()` on `default_http_client_builder().build()` in SWQOS client constructors (nextblock.rs, etc.)
- `.expect("dedicated sender runtime")` in `async_executor.rs` line 294 — tokio runtime creation
- `.expect("SolanaTrade instance not initialized")` in `client/mod.rs` line 1315 — singleton check
- `panic!("WSOL ATA creation failed")` in `client/mod.rs` line 1161
- `.unwrap()` on `QuicClientConfig::try_from(crypto)` — TLS config conversion
- **~40 occurrences** — acceptable; failure here means total system failure anyway

#### Hot-path: SWQOS serialization (P1, but invariant)
- `.unwrap()` on `bincode::serialize(self)` in `common.rs` line 82 (`to_base64_string` — formatting trait)
- `.unwrap()` on `u16::from_be_bytes(header[0..2].try_into().unwrap())` in `node1_quic.rs` line 149 — response header parsing
- `.unwrap()` on `serde_json::to_value` in `common.rs` — error serialization path
- **~15 occurrences** — the `to_base64_string` unwrap could panic if bincode serialization fails (should be a Result)

#### Hot-path: Instruction building / token math (P1 — external input)
- **`src/instruction/pumpswap.rs`** — 36 `.unwrap()` calls, including:
  - `params.input_amount.unwrap()` (lines 400, 412, 415) — **external input** from trade params
  - `params.slippage_basis_points.unwrap_or(...)` — safe due to `unwrap_or`
  - Instructions builder test code unwrap — test-only
- **`src/instruction/pumpfun.rs`** — 53 `.unwrap()` calls, many on compute budget instruction builders, PDA derivation
- **`src/instruction/bonk.rs`** — 18 `.unwrap()` calls
- **`src/instruction/raydium_cpmm.rs`** — 13 `.unwrap()` calls
- **`src/utils/calc/*`** — mathematical invariants; `.unwrap()` or division by zero risk

**P0 Finding: `params.input_amount.unwrap()` in instruction builders (pumpswap.rs)**

The `sell_base_input_internal_with_fees` and `buy_quote_input_internal_with_fees` functions call `.unwrap()` on `params.input_amount` which comes from user-supplied `TradeParams`. If the caller supplies the wrong enum variant where input_amount is None, this **panics at runtime on the hot path**, crashing the trading thread.

Example (line 400-417):
```rust
(params.input_amount.unwrap(), output_amount)
```

This should use `.context()` + `?` or `.ok_or_else()` instead.

#### Hot-path: Tip account selection (P1 — fallback)
- Every SWQOS `get_tip_account()` method calls `.choose(&mut rand::rng()).or_else(|| ..).unwrap()` — **safe only if tip account array is non-empty**. If the array is accidentally empty, this panics. Low probability, but consequences are a trading thread crash.

#### Protocol parsing: Response headers (P1)
- `node1_quic.rs` line 149: `u16::from_be_bytes(header[0..2].try_into().unwrap())` — parsing 6-byte response header. If the server sends malformed data, this **panics** rather than returning an error.

### C.3 Critical P0/P1 Panic Paths

| File | Line | Pattern | Severity | Notes |
|------|------|---------|----------|-------|
| `src/instruction/pumpswap.rs` | ~400,412,415 | `params.input_amount.unwrap()` | **P0** | External input on hot path — crashes trading thread |
| `src/instruction/pumpswap.rs` | ~400-417 | Multiple `.unwrap()` in sell/buy flow | **P0** | Can crash on malformed params |
| `src/instruction/pumpfun.rs` | Various | Compute budget ix building `.unwrap()` | P1 | Invariant but should use `?` |
| `src/swqos/common.rs` | 82 | `bincode::serialize(self).unwrap()` | P1 | Serialization failure on every tx send |
| `src/swqos/node1_quic.rs` | 149 | `.try_into().unwrap()` on server response | **P1** | Protocol parsing — malformed response panics |
| `src/swqos/node1_quic.rs` | 187 | `.unwrap_or_default()` on signature list | P1 | Empty tx panics on signature access |
| `src/client/mod.rs` | 1161 | `panic!(...)` | P1 | Startup-only, but hard crash on WSOL failure |

### C.4 Summary

**305 unwrap() + 14 expect() + 1 panic!() = 322 potential panic sites.** The vast majority (~270) are compile-time invariants (PDA derivation, compute budget building, constants) that cannot fail under normal conditions. The **P0 risk** is concentrated in ~10 call sites in instruction builders and SWQOS protocol parsing where external input or server response reaches `.unwrap()`.

---

## D. Production Feature Gates Audit

### D.1 Current feature configuration

```toml
[features]
default = []
perf-trace = []  # Performance tracing — should be disabled in production
```

Only two features exist:
- **`default`** — empty feature set (no optional components gated)
- **`perf-trace`** — enables performance tracing via `#[cfg(feature = "perf-trace")]` in `client/mod.rs` and `executor.rs`

### D.2 Dev-insecure TLS features

**No `dev-insecure-tls` feature exists.** The `SkipServerVerification` TLS bypass is **unconditionally compiled** into every binary — there is no feature gate protecting it.

### D.3 Finding: No separation between dev and production TLS (P1)

The `.dangerous().with_custom_certificate_verifier(SkipServerVerification::new())` pattern is used in all six QUIC transports regardless of build profile. There is no `#[cfg(feature = "...")]` gate, no compile-time switch, and no runtime flag to enable/disable TLS verification.

**Risk:** If a QUIC provider were to deploy WebPKI certificates, the code would still ignore them because `SkipServerVerification` is the only path. There's no way to "turn on" TLS verification without a code change.

### D.4 Finding: `solana-tls-utils` is a non-optional dependency

```toml
solana-tls-utils = "3.1.12"
```

This is in `[dependencies]` (not `[dev-dependencies]`), and `solana_tls_utils::SkipServerVerification` is imported unconditionally by `glaive_quic.rs`, `solami.rs`, and `speedlanding.rs`. The library cannot be compiled without including the skip-verification code.

### D.5 Recommendation: Explicit production/dev feature separation

1. **Add a `dev-insecure-tls` feature** that gates all `SkipServerVerification` usage:
   ```toml
   [features]
   default = []
   perf-trace = []
   dev-insecure-tls = []
   ```

2. **Gate QUIC client TLS configuration** behind the feature (or invert: gate proper TLS behind a `production-tls` feature):
   ```rust
   #[cfg(not(feature = "dev-insecure-tls"))]
   let crypto = rustls::ClientConfig::builder()
       .with_platform_verifier()
       ...
   #[cfg(feature = "dev-insecure-tls")]
   let crypto = rustls::ClientConfig::builder()
       .dangerous()
       .with_custom_certificate_verifier(...)
   ```

3. **Make `solana-tls-utils` optional** behind `dep:solana-tls-utils` feature so production builds don't pull it.

---

## E. Cross-Cutting Recommendations

### Priority P0 (Fix Immediately)
1. Replace `params.input_amount.unwrap()` with proper error propagation in instruction builders (pumpswap.rs, pumpfun.rs)
2. Replace `header[0..2].try_into().unwrap()` with `?` in node1_quic.rs response parsing

### Priority P1 (Fix Soon)
3. Gate all `SkipServerVerification` behind a `dev-insecure-tls` feature flag
4. Document accepted TLS risk for QUIC providers with certificate pinning or application-layer auth as complement
5. Audit remaining ~20 hot-path `.unwrap()` calls for potential crash on malformed input
6. Remove or gate the Temporal hardcoded API key hash

### Priority P2 (Improve)
7. Change `default_http_client_builder()` doc from "call `.build().unwrap()`" to actually handle the error
8. Make `solana-tls-utils` an optional dependency
9. Consider `#[track_caller]` and `expect("invariant: ...")` for compile-time unwraps to improve debug UX

---

## F. Risk Matrix Summary

| Area | Severity | Count | Verdict |
|------|----------|-------|---------|
| Hardcoded credentials | Medium | 1 (temporal.rs hash) | Acceptable with documentation |
| TLS verification bypass | **Critical (P0)** | 6 QUIC clients | Accepted risk (self-signed + app-layer auth); needs feature gate |
| Panic on external input | **P0** | ~10 sites | **Must fix** — trading thread crashes |
| Invariant unwrap (safe) | Low | ~270 sites | Acceptable |
| No production/dev feature separation | P1 | 1 (features only 2 entries) | Should add `dev-insecure-tls` gate |
| `perf-trace` feature mixing | Low | 1 feature | Currently no overlap with security |

---

*End of security review. Automated analysis performed by Hermes Agent.*