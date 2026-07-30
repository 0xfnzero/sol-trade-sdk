# AUDIT REMEDIATION PLAN — sol-trade-sdk

## Dependency Order

```
P0-01 (market keying) ──────┐
P0-02 (real pricing) ───────┤
P0-03 (RiskContext) ────────┤
P0-06 (state transitions) ──┤  ← no deps, all independent
P1-07 (blockhash last_valid) ┤
P1-08 (add Built→Failed) ───┤
                              │
P0-04 (profit/tip ratio) ────┼─ depends on P0-02 (needs real prices)
P0-05 (position sizing) ────┼─ depends on P0-03 (needs config values)
P1-01 (config wiring) ──────┼─ depends on P0-05 (uses config values)
P1-06 (slippage from config) ┼─ depends on P0-05 (uses config values)
                              │
P1-02/03 (balance snapshots) ─ depends on P0-06 (uses plan_trade)
P1-04 (protocol per mint) ───
P1-05 (blockhash last_valid) ─
                              │
Surfpool integration ───────── late (needs working transaction pipeline)
```

## Phase 1 — Independent P0 Fixes (no deps)

| ID | File(s) | Task | Test |
|----|---------|------|------|
| P0-01 | `market_state.rs:203` | Key by `mint` extracted from instruction data instead of `program_id` | Two distinct mints maintain separate state |
| P0-02 | `market_state.rs:92-99` | Replace synthetic sine-wave with actual swap-amount derived prices | Real swap data produces correct price; synthetic path rejected |
| P0-03 | `risk.rs:40-75`, `orchestrator.rs`, `bundle_executor.rs:367,458` | Add TradeLedger to Orchestrator, populate RiskContext before each trade | RiskContext has non-default values; rejected when limits exceeded |
| P0-06 | `orchestrator.rs:544-679` | Replace all `let _ = intent.transition(...)` with proper error propagation | Invalid transitions abort execution |
| P1-08 | `state.rs:95` | Add `(Built, Failed)` to valid transition table | Built→Failed accepted |
| P2-05 | `bundle_executor.rs:544+` | Fix missing `landed()` parameters | BundleMetrics records landing time |
| P2-07 | `solbot.rs:414` | Return actual system readiness, not hardcoded "ok" | Health endpoint reflects real subsystem state |

## Phase 2 — P0 Fixes with Dependencies

| ID | Depends on | Task |
|----|-----------|------|
| P0-04 | P0-02 | Fix profit/tip ratio to use real price and config-driven position size |
| P0-05 | P0-03 | Fix hardcoded 0.1 SOL / 100_000_000 lamports; read from AppConfig.strategy.position_size_sol |

## Phase 3 — P1 Config & Reconciliation Fixes

| ID | Task |
|----|------|
| P1-01 | Wire OrchestratorConfig fields from AppConfig (add missing fields to AppConfig) |
| P1-02/03 | Query real token ATAs in plan_trade, populate balance_snapshot |
| P1-04 | Return (Hash, Option<u64>) from BlockhashService for last_valid_slot |
| P1-05 | Make protocol configurable per mint (multi-mint execution map) |
| P1-06 | Use RiskConfig.max_slippage_basis_points instead of hardcoded 500 bps |
| P1-07 | Auto-stop on sustained RPC failure |

## Phase 4 — Surfpool Integration

| Task |
|------|
| Add surfpool-based integration tests for full transaction pipeline |
| Wallet funding, account init, buy/sell construction, submission, balance checks |

## Phase 5 — Final Deliverables

| File | Content |
|------|---------|
| `AUDIT_REMEDIATION_LOG.md` | Chronological record of all changes |
| `AUDIT_REPORT_v3.md` | Post-remediation re-audit |
| `PRODUCTION_READINESS_CHECKLIST.md` | Binary pass/fail for each readiness gate |

## Rollback Notes

- Every fix is scoped to the minimal affected files
- State machine changes are purely additive (new valid transitions)
- No broad architectural refactoring — only targeted issue fixes
- Pre-existing tests must continue to pass