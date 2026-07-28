# Golden Fixture Tests — sol-trade-sdk

> **Status:** Created — first pass  
> **Audit reference:** Pre-flight §37.2, §4.1  
> **DEX protocols covered:** 6

## Protocol Program ID Fixtures

| Protocol | File | Tests |
|---|---|---|
| PumpFun | `tests/unit/pumpfun.rs` | 6 — fee constants, total fee calc, virtual reserves |
| PumpSwap | `tests/unit/pumpswap.rs` | 3 — LP fee, protocol fee, creator fee defaults |
| Raydium AMM V4 | `tests/unit/raydium_amm_v4.rs` | 1 — program ID |
| Raydium CPMM | `tests/unit/raydium_cpmm.rs` | 1 — program ID |
| Meteora DAMM V2 | `tests/unit/meteora_damm_v2.rs` | 1 — program ID |
| Bonk DEX | `tests/unit/bonk.rs` | 2 — program ID, PDA authority |

## CI Status

| Workflow | Status | Notes |
|---|---|---|
| `ci.yml` (Agent 10) | ✅ Created | 6 jobs: fmt, check, clippy, test, audit, deny |
| `release.yml` (updated) | ✅ CI-gated | Now requires `ci-pass` before release |
| `cargo fmt --check` | ✅ Passes | After fix in commit |
| `cargo test` | ⏳ Running | Cold Solana dependency build (~5-10 min) |

## Remaining Phase 4 Items

- [ ] Wait for `cargo test` to complete
- [ ] Verify test results (PASS/FAIL)
- [ ] Add CI scheduled jobs (weekly audit, upstream drift, IDL hash)
- [ ] Commit and push