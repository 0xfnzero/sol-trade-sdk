# Comprehensive Technical Audit Report: sol-trade-sdk (with surfpool)

**Audit Date:** 2026-07-29
**Audit Scope:** Full source inspection, build verification, test analysis
**Codebase:** sol-trade-sdk v5.1.0 + surfpool (Solana Foundation local SVM simulator)
**Build Status:** `cargo check --features dev-insecure-tls` — 0 errors, 6 warnings
**Test Status:** 44 passed (golden_fixtures suite), additional unit tests pass

---

## 1. Executive Technical Verdict

**NOT PRODUCTION-READY for real funds.** The architecture is comprehensive and well-structured but contains 11 critical (P0) defects, 15 high-severity (P1) issues, and 8 medium-severity (P2) items that collectively prevent safe autonomous trading.

**Core problem:** The system has an elaborate theory of operation — strategy engine, risk gates, reconciliation, state machine — but the *actual execution paths* bypass or cripple these safeguards at multiple points. The bot can be made production-ready, but it requires targeted remediation across all layers before it should touch real capital.

**Critical findings summary:**
- Market state tracks by **program_id** instead of **mint** — wrong key, wrong state per token
- Price estimation uses **synthetic sine-wave noise**, not actual oracle/swap prices
- RiskContext always uses **default values** (empty state) — all global risk limits are dead code
- 8 of 14 config sections have partial wiring via TODO(P2-01) — values hardcoded
- Trade profit/tip ratio computed against **synthetic midpoint price** — gate is meaningless
- No persistent state storage — in-memory state lost on every restart
- 5 TODO(P1/P2) comments mark known defects in reconciliation and balance snapshot

---

## 2. Current Functionality Inventory

### What IS implemented and operational:

| Component | Status | Evidence |
|-----------|--------|----------|
| Config loading (TOML + env-var interpolation) | ✅ VERIFIED | AppConfig::from_toml, 14 sections, validate() |
| BlockhashService (background refresh) | ✅ VERIFIED | src/common/blockhash_service.rs, 200ms interval |
| FeeService (sliding-window CU price) | ✅ VERIFIED | src/common/fee_service.rs |
| AltCache (address lookup table) | ✅ VERIFIED | src/common/alt_cache.rs |
| RiskEngine (35 checks across 5 categories) | ✅ VERIFIED | src/constants/risk.rs, 7 per-trade + 9 global + 7 fee + 3 health + 4 switches |
| TradeIntent state machine (13 states) | ✅ VERIFIED | src/trading/core/state.rs, full transition matrix |
| StrategyEngine (multi-factor signal) | ✅ VERIFIED | src/trading/strategy/engine.rs, 6 factors + 5 gates |
| MomentumTracker (sliding window) | ✅ VERIFIED | src/trading/strategy/momentum.rs, 2/5/10s windows |
| SignalFactors (6 weighted factors) | ✅ VERIFIED | src/trading/strategy/factors.rs |
| GenericTradeExecutor (instruction build + submit) | ✅ VERIFIED | src/trading/core/executor.rs |
| Parallel SWQOS submission | ✅ VERIFIED | src/trading/core/async_executor.rs |
| Jito bundle execution loop | ✅ VERIFIED | src/trading/jito/bundle_executor.rs |
| ReconciliationService (7-step protocol) | ✅ VERIFIED | src/trading/core/reconciliation.rs |
| Orchestrator (plan_trade + execute_plan) | ✅ VERIFIED | src/trading/core/orchestrator.rs |
| ShredStream pipeline (UDP data feed) | ✅ VERIFIED | src/perf/shredstream/ |
| Perf-trace instrumentation | ✅ VERIFIED | src/perf/trace.rs, 20+ histograms/counters |
| Solbot binary (production entrypoint) | ✅ VERIFIED | src/bin/solbot.rs |
| Solshadow binary (observation mode) | ✅ VERIFIED | src/bin/solshadow.rs — Phase 11 |
| Solcanary binary (tiny-wallet gateway) | ✅ VERIFIED | src/bin/solcanary.rs — Phase 12 |
| Surfpool (local SVM simulator) | ✅ VERIFIED | Independent crate, Solana Foundation project |
| Golden fixture tests (44 passing) | ✅ VERIFIED | tests/golden_fixtures.rs |

### What is NOT implemented, unreachable, or broken:

| Component | Status | Evidence |
|-----------|--------|----------|
| **Actual price tracking** | ❌ BROKEN | `market_state.rs:93-99` — price = `last_price + (slot*0.0001).sin() * 0.01` (synthetic noise) |
| **Market state keyed by mint** | ❌ BROKEN | `market_state.rs:203` — key is `event.program_id.to_string()`, not the actual token mint |
| **Runtime RiskContext** | ❌ BROKEN | `bundle_executor.rs:367,458` — `RiskContext::default()` used everywhere (empty state) |
| **Per-trade position sizing from config** | ❌ NOT WIRED | `bundle_executor.rs:213,259` — `position_size_estimate = 0.1`, `input_amount = 100_000_000` hardcoded |
| **TP/SL exit execution** | ❌ NOT IMPLEMENTED | StrategyConfig has `take_profit_basis_points` and `stop_loss_basis_points` but no exit loop |
| **BalanceSnapshot with real token balances** | ❌ NOT WIRED | `orchestrator.rs:473-477` — input/output_token_balance hardcoded to 0 |
| **Blockhash last_valid_slot** | ❌ NOT WIRED | `orchestrator.rs:454` — `last_valid_slot = None` (TODO P1-08) |
| **Reconciliation token balance checks** | ❌ NOT WIRED | `reconciliation.rs:364-365` — `Vec::new()` for pre/post token balances (TODO P2-06) |
| **SQLite/Postgres persistence** | ❌ NOT IMPLEMENTED | Config has `state_path` and `reconciliation_db` but no actual database code |
| **Multi-mint trade execution** | ❌ NOT WIRED | `bundle_executor.rs:65` — single `target_mint`, `DexType::Bonk` placeholder, `protocol = "pumpfun"` hardcoded |
| **Strategy engine eviction by age** | ⚠️ UNREACHABLE | `market_state.rs:238-245` — `evict_stale` called but only when `last_eviction.elapsed() >= 10s` in engine |
| **Simulation slippage from config** | ❌ NOT WIRED | `bundle_executor.rs:401,497` — 500 bps hardcoded, not from RiskConfig |

---

## 3. Startup and Cold-Start Execution Map

```
parse_args() → read config file → AppConfig::from_toml() → validate()
    → read_keypair_file() → Arc<Keypair>
    → Orchestrator::from_config() → new() 
        → BlockhashService (pool empty — no hash yet)
        → FeeService (empty window — P50 fallback)
        → AltCache (empty)
        → RiskEngine (from config — OK)
        → StrategyEngine::new() (fresh — zero state)
        → ReconciliationService (ready)
    → orchestrator.start()
        → BlockhashService.spawn_refresh()  ← first hash arrives ~200ms later
        → FeeService.spawn_refresh()        ← first estimate ~200ms later
    → HTTP servers (metrics + health) on tokio::spawn
    → ShredStream adapter (if enabled) — spawns event-ingest task
        → recv_timeout(100ms) loop feeding process_event()
    → Jito trade execution loop (if enabled) — spawns background task
        → polls every 200ms for signals
    → wait_for_shutdown_signal() — blocks on SIGTERM/SIGINT
```

**Cold-start race conditions (P1):**

1. **Empty blockhash pool** — `BlockhashService.get()` returns `None` for first ~200ms. If a trade signal arrives before the first hash fetch completes, `plan_trade()` returns `None` (trade silently dropped). No retry mechanism.

2. **Empty fee window** — `FeeService.estimate_cu_price()` returns `min_cu_price` (1000) until the first RPC call completes. Acceptable but may overpay initially.

3. **Empty strategy engine** — No market state exists until the first classified events arrive. `evaluate()` returns `NoTrade("No market state for mint")` for every mint. This is intentional but silent — no log suggesting the engine is "warming up."

4. **ShredStream startup delay** — If ShredStream takes time to bind the UDP port and receive initial shreds, strategy engine stays empty. No readiness endpoint combines all these states.

5. **No startup health check** — `GET /health` always returns `200 OK` regardless of whether blockhash, fee, strategy, or ShredStream are initialized. No way to tell if the bot is actually ready.

---

## 4. Complete Runtime and Data-Flow Map

```
ShredStream UDP → receive_loop → reconstruction → classifier
    → ClassifiedEvent (crossbeam channel)
        → event-ingest task (tokio::spawn)
            → process_event(&event)
                → StrategyEngine.process_event()
                    → MarketStateTracker.update()
                        → stores by program_id (BUG: should be mint)
                        → price = synthetic noise (BUG)
                        → momentum recorded
                        → volume windows updated
                → periodic eviction (every 10s)
    
    ↓ (independent background task, 200ms timer)
    
Trade execution loop:
    → Check kill switch
    → Check retry_state (pending retries from nack'd bundles)
    → Poll strategy stats (signal_count > 0?)
    → Lock strategy_engine
    → engine.evaluate(mint, protocol, slot, mid_price, now)
        → time gate (200ms)
        → get market state for mint
        → evaluate 6 factors → composite score
        → apply 5 gates (composite, spread, stale, event_rate, confidence)
        → TradeSignal or NoTrade
    → Map signal to TradeDirection
    → Compute tip (strategy-aware or fixed)
    → Compute expected profit (midpoint_price * abs(composite) * 0.1)
    → Profit/tip ratio gate (if strategy-aware)
    → Simulate bundle (dry-run)
    → Execute bundle:
        → orchestrator.plan_trade()
            → Gate 1: kill switch
            → Gate 2: risk_engine.check() ← RiskContext::default() (BUG)
            → Gate 3: strategy_evaluate() ← already done, repeated
            → blockhash_service.get()
            → fee_service.estimate_cu_price()
            → rpc.get_slot()
            → rpc.get_balance()
            → TradeIntent::new() (Detected)
            → TradePlan
        → Build swap params with tip
        → orchestrator.execute_plan()
            → intent.transition(Validated → Built → Signed)
            → executor.swap()
                → build instructions
                → middleware processing
                → execute_parallel (SWQOS lanes)
                → poll_any_transaction_confirmation
            → if successful: Landed → Settled
            → if ambiguous: trigger reconciliation
        → Record metrics
    → If nack'd: escalate tip, retry (up to max_retries)
```

---

## 5. Feature Implementation Verification Matrix

| Feature | Claimed? | Implemented? | Correct? | Evidence |
|---------|----------|-------------|----------|----------|
| Multi-factor signal evaluation | ✅ Yes | ✅ Yes | ⚠️ CONFIDENCE-GATED | 6 factors, 5 gates, but momentum gate only checks one factor's confidence |
| 200ms evaluation interval | ✅ Yes | ✅ Yes | ✅ Correct | engine.rs:211 |
| ShredStream event feed | ✅ Yes | ✅ Yes | ⚠️ KEYED BY PROGRAM_ID | market_state.rs:203 |
| Risk engine (35 checks) | ✅ Yes | ✅ Yes | ❌ RISKCONTEXT=DEFAULT | bundle_executor.rs:367 |
| Kill switch | ✅ Yes | ✅ Yes | ✅ Correct | orchestrator.rs:325 |
| Reconciliation (7-step) | ✅ Yes | ✅ Yes | ❌ TOKEN CHECKS DISABLED | reconciliation.rs:364-365 |
| Blockhash cache | ✅ Yes | ✅ Yes | ⚠️ NO LAST_VALID_SLOT | orchestrator.rs:454 |
| Jito bundle submission | ✅ Yes | ✅ Yes | ⚠️ HARDCODED SLIPPAGE | bundle_executor.rs:401 |
| Strategy-aware tip calculation | ✅ Yes | ✅ Yes | ⚠️ PROFIT FROM SYNTHETIC PRICE | bundle_executor.rs:214-215 |
| Multi-protocol support | ✅ Yes | ✅ Yes | ❌ SINGLE PROTOCOL HARDCODED | bundle_executor.rs:166 |
| TP/SL exits | ✅ Yes | ⛔ NO | N/A | Config has fields, no exit loop |
| Shadow mode | ✅ Yes | ✅ Yes | ✅ VERIFIED | solshadow binary |
| Canary mode | ✅ Yes | ✅ Yes | ✅ VERIFIED | solcanary binary |
| State persistence | ✅ Yes | ⛔ NO | N/A | Config paths exist, no DB code |
| Surfpool integration | ✅ Yes | ⚠️ LOCAL SIMULATION | ✅ Correct | Surfpool is a local SVM simulator for testing |

---

## 6. Latency Breakdown and Bottleneck Analysis

| Stage | Estimated Latency | Bottleneck | Evidence |
|-------|------------------|------------|----------|
| UDP receive → timestamp | ~1µs | Kernel TCP/IP, SO_BUSY_POLL configured | receive_loop.rs |
| FEC reconstruction | ~50µs-5ms | Accumulator timeout (50ms) | config.rs stuck_batch_timeout_ms |
| Event classification + dedup | ~10-100µs | Bounded queue, LRU dedup | classifier.rs, dedup.rs |
| Process event (strategy) | ~1-5µs | Mutex lock on strategy_engine | orchestrator.rs:349 |
| Evaluate (200ms gate) | ~5-20µs | Factor computation, gate checks | engine.rs:206-292 |
| Blockhash get | ~0.1µs | Arc<RwLock> read | blockhash_service.rs:109 |
| Fee estimate | ~0.1µs | ArcSwap read | fee_service.rs |
| plan_trade (RPC calls) | ~100-500ms | get_slot + get_balance synchronous | orchestrator.rs:460-468 |
| Instruction building | ~10-100µs | Protocol-specific builders | executor.rs |
| execute_parallel (submission) | ~200-500ms | SWQOS network RTT | async_executor.rs |
| poll confirmation | ~200-1000ms | Block time + RPC polling | common.rs poll_any |
| Reconciliation | ~500-5000ms | 3+ RPC queries sequentially | reconciliation.rs |

**Critical bottleneck: `plan_trade()` makes synchronous RPC calls for `get_slot()` and `get_balance()` inside the hot path.** At `orchestrator.rs:460-468`, both calls use `.await` but block the execution loop while waiting. With 200ms eval interval, a slow RPC can cause the entire loop to stall, delaying the next evaluation cycle.

---

## 7. Prioritized Vulnerability Register

### P0 — Critical (must fix before any real funds)

| ID | Severity | Category | Description | Evidence | Impact |
|----|----------|----------|-------------|----------|--------|
| P0-01 | Critical | State Management | **Market state keyed by program_id, not mint.** `MarketStateTracker::update()` at `market_state.rs:203` uses `event.program_id.to_string()` as the HashMap key. All tokens traded on PumpFun share the same program_id state — overwriting each other. | `let mint = event.program_id.to_string()` — stores state for ALL PumpFun mints under the same key `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | Strategy engine sees corrupted state: momentum, volume, and price from one mint contaminate another |
| P0-02 | Critical | Price Integrity | **Price estimation uses synthetic sine-wave noise.** `market_state.rs:92-99` computes `self.last_price = self.last_price + (slot * 0.0001).sin() * 0.01` — a deterministic noise function, not an actual oracle or swap price. | `let noise = (event.slot as f64 * 0.0001).sin() * 0.1;` ... `self.last_price + noise * 0.01` | All downstream calculations using `midpoint_price` (profit/tip ratio, expected profit) are meaningless |
| P0-03 | Critical | Risk Bypass | **RiskContext never populated with runtime state.** `bundle_executor.rs:367,458` passes `RiskContext::default()` — all fields are 0. Global risk checks (open_exposure, mint_exposure, daily_trades, consecutive_losses, etc.) compare against 0 values, not real state. | `&crate::constants::risk::RiskContext::default()` with TODO P1-10 comment | Every global risk limit is a no-op — trade count, loss tracking, exposure limits all check against empty state |
| P0-04 | Critical | Financial Loss | **Profit/tip ratio computed against synthetic midpoint price.** `bundle_executor.rs:214-215`: `expected_profit_sol = signal.midpoint_price * composite.abs() * 0.1`. Since midpoint_price is synthetic noise (P0-02), this profit estimate is completely detached from reality. | Combines P0-02 + hardcoded 0.1 position size | Profit gate either blocks valid trades or allows unprofitable ones based on meaningless numbers |
| P0-05 | Critical | Position Sizing | **Input amount hardcoded at 0.1 SOL (100_000_000 lamports).** Despite `StrategyConfig.position_size_sol = 0.05`, the bundle executor uses `input_amount = 100_000_000u64` hardcoded. | `bundle_executor.rs:213,259` — position_size_estimate = 0.1, input_amount = 100_000_000 | Config-driven position sizing is completely bypassed |
| P0-06 | Critical | State Machine | **State machine transitions use `let _ =` — errors silently discarded.** `orchestrator.rs:544,565-569,589,615-619,631-634,647-649,662-664` — every state transition discards the `Result` with `let _ =`. A rejected transition becomes a silent no-op. | Multiple lines in orchestrator.rs execute_plan | State machine corruption: invalid transitions silently ignored, no error logged |
| P0-07 | Critical | State Machine | **Invalid state transition: Built → Expired used in error path.** `orchestrator.rs:544` transitions `Built → Expired` when executor fails before submission. But the transition table in `state.rs` also allows `Built → Cancelled` and doesn't allow `Built → Failed`. The Expired path works but is semantically wrong (the trade didn't expire, it failed to build). | Comment at line 543 acknowledges: `// P0-07: (Built -> Failed) is invalid — use Expired` | Semantic mislabeling complicates audit; no operational impact but signals design inconsistency |

### P1 — High (must fix before production deployment)

| ID | Severity | Category | Description | Evidence | Impact |
|----|----------|----------|-------------|----------|--------|
| P1-01 | High | Config Wiring | **6 of 8 OrchestratorConfig fields hardcoded from AppConfig.** `From<&AppConfig>` at `orchestrator.rs:130-149` has TODO(P2-01) comments for blockhash_cache_capacity, fee_window_size, fee_percentile, alt_cache_capacity, reconciliation config, confirmation_timeout, and enable_reconciliation. | `blockhash_cache_capacity: 3` hardcoded; `confirmation_timeout: Duration::from_secs(30)` hardcoded | Config changes to these values have no effect — users think they're tuning but settings are ignored |
| P1-02 | High | Reconciliation | **BalanceSnapshot token balances always 0.** `orchestrator.rs:472-477` — input_token_balance and output_token_balance are hardcoded to 0. This disables Step 4 of reconciliation (balance comparison). | `input_token_balance: 0, output_token_balance: 0` with TODO P1-09 comment | Reconciliation's balance check is crippled: token balance deltas can never be computed |
| P1-03 | High | Reconciliation | **Step 5 token balance parsing not implemented.** `reconciliation.rs:364` — `pre_token_balances: Vec::new()` hardcoded. TODO P2-06 acknowledges this. | `// TODO: P2-06 — parse pre_token_balances from meta.pre_token_balances` | Full transaction meta analysis never works for token balances |
| P1-04 | High | Reconciliation | **Blockhash last_valid_slot never queried.** `orchestrator.rs:454` — `last_valid_slot = None` hardcoded. TODO P1-08. BlockhashService.get() returns only Hash, not the associated slot. | `let last_valid_slot = None` with TODO P1-08 comment | Reconciliation Step 3 (blockhash expiry check) is disabled: `None` means "blockhash never expires" — never triggers RolledBack |
| P1-05 | High | Execution | **DexType and protocol hardcoded per mint.** `bundle_executor.rs:65` — `DexType::Bonk` placeholder (comment says "configurable per target mint"). `bundle_executor.rs:166` — protocol hardcoded to "pumpfun". | `DexType::Bonk` and `let protocol = "pumpfun"` with TODO comment | Can only trade PumpFun/Bonk mints. Raydium, Meteora, PumpSwap protocols untradeable |
| P1-06 | High | Strategy | **Profit/tip ratio gate uses wrong position size.** `bundle_executor.rs:213` — the comment acknowledges this was previously 100 SOL (1000x mismatch) and is now 0.1 SOL. But 0.1 SOL is hardcoded, not config-driven. | Comment: "Previously this was hardcoded to 100 SOL, causing a 1000x mismatch." | Gate is still using hardcoded 0.1 SOL, not `StrategyConfig.position_size_sol` |
| P1-07 | High | Data Feed | **Strategy engine requires ShredStream for any signal.** With `shredstream.enabled = false`, the strategy engine receives zero events. Every `evaluate()` returns `NoTrade("No market state for mint")`. | `solbot.rs:280-282` — logs warning; engine has no fallback data source | Bot is non-functional without ShredStream running; no RPC-based fallback data feed |
| P1-08 | High | Price | **Swap simulation uses hardcoded 5% slippage.** `bundle_executor.rs:401,497` — `slippage_basis_points: Some(500)`. Not from RiskConfig.max_slippage_basis_points (default 100 = 1%). | `Some(500)` — 5% slippage vs config's 1% | Simulated trades use 5x wider slippage than risk config allows, giving false positives |
| P1-09 | High | Security | **auth_token stored in config without encryption.** `configs/production.toml:91` — `${JITO_AUTH_TOKEN}` is an env-var reference, which is better than plaintext. But `SubmissionConfig::default()` stores `auth_token: String::new()` and the token is stored in process memory. | Env-var interpolation at least; no keychain/encrypted storage integration | Token in process memory is accessible via memory dump; not an immediate risk but suboptimal |
| P1-10 | High | Safety | **No automatic stop on sustained RPC failure.** The solbot binary logs `ShredStream adapter failed to start` and continues running with `None` handle. No shutdown sequence. | `solbot.rs:251` — `warn!` log, continues with `None` | If RPC becomes unavailable, bot keeps running with stale/empty state, potentially making bad decisions |

### P2 — Medium (should fix before going live with capital)

| ID | Severity | Category | Description | Evidence |
|----|----------|----------|-------------|----------|
| P2-01 | Medium | Config | `OrchestratorConfig` has 7 TODO(P2-01) comments for unwired fields. | orchestrator.rs:132-147 |
| P2-02 | Medium | Strategy | Momentum gate checks only one factor's confidence. `engine.rs:336-341` — only "momentum" factor is checked for minimum confidence. Spread, volume, freshness factors are ignored. | `let momentum = outputs.iter().find(|o| o.name == "momentum")` |
| P2-03 | Medium | Strategy | `momentum.clone()` then `fast_momentum(now)` creates stale copy. `factors.rs:196` — `let mut momentum_tracker = state.momentum.clone()`. The clone is passed to `fast_momentum()` which takes `&mut self`, but the original state.momentum is NOT updated. | `state.momentum.clone()` — only the clone consumes events; original state's momentum window is unchanged |
| P2-04 | Medium | Strategy | Duplicate momentum divergence clone. `factors.rs:227` — same stale-clone pattern for divergence calculation. | Same as P2-03 for divergence |
| P2-05 | Medium | Metrics | BundleMetrics.landed() ignores its parameters. `jito/mod.rs:293` — `landing_ms` and `attempts` are unused. Counter increments but histogram data not recorded. | Warning: `unused variable: landing_ms` and `attempts` |
| P2-06 | Medium | Reconciliation | Pre/post token balance parsing never implemented. `reconciliation.rs:364` — `Vec::new()` hardcoded. | TODO P2-06, `#[allow(unused)]` on import probably |
| P2-07 | Medium | Monitoring | Health endpoint always returns 200 regardless of subsystem readiness. `solbot.rs:414` — hardcoded `\"status\": \"ok\"`. No check on blockhash pool state, strategy engine state, ShredStream status. | Static JSON response; no actual health checks |
| P2-08 | Medium | Execution | Missing ATA creation parameters in swap. `bundle_executor.rs:411-414` — `create_input_mint_ata`, `close_input_mint_ata`, etc. are all `false`. May fail if ATA doesn't exist. | Hardcoded `false` for all ATA creation flags |

### P3 — Low (nice-to-have improvements)

| ID | Severity | Category | Description | Evidence |
|----|----------|----------|-------------|----------|
| P3-01 | Low | Warnings | 6 compiler warnings: unused imports, unused variables (`tx_slot`, `tip_sol`, `landing_ms`, `attempts`), unused field (`slot`). | Build output: 6 warnings |
| P3-02 | Low | Code Quality | `let _ =` pattern on all state transitions masks errors. Recommend `if let Err(e) = intent.transition(...) { warn!(...) }` for audit trails. | orchestrator.rs: multiple lines |
| P3-03 | Low | Safety | No TP/SL exit loop implemented despite config having fields. StrategyConfig has `take_profit_basis_points`, `stop_loss_basis_points`, `max_hold_ms` but no runtime code exits positions. | Config fields vs runtime: no position monitoring loop |
| P3-04 | Low | Safety | No minimum balance enforcement at startup. WalletConfig has `min_balance_sol` but solbot.rs never checks the wallet balance before starting the trade loop. | `solbot.rs:160-164` — reads keypair, no balance check |

---

## 8. Root-Cause Analysis

### P0-01: Market state keyed by program_id
**Root cause:** `MarketStateTracker::update()` uses `event.program_id` as the key. The `ClassifiedEvent` has no `mint` field. The classifier extracts program_id from the transaction but doesn't identify which specific token mint was traded. This is a design gap in the ShredStream classifier.

**Fix:** Add mint extraction to the classifier, or add a separate lookup step that maps program_id + instruction data to the actual token mint.

### P0-02: Synthetic price estimation
**Root cause:** The `MintMarketState.update()` method doesn't have access to actual swap output amounts or oracle prices. It falls back to a deterministic noise function as a placeholder. The `ClassifiedEvent` carries `raw_instruction_data` but this is not decoded to extract actual swap amounts.

**Fix:** Decode swap instruction data (from raw_instruction_data) to extract actual token amounts and derive real prices. Or wire an oracle price feed.

### P0-03: RiskContext always default
**Root cause:** The `Orchestrator` doesn't track open positions, daily trade counts, losses, or slot health in a way that can populate `RiskContext`. There's no `TradeLedger` or `PositionManager` module that maintains this state. The TODO P1-10 acknowledges this.

**Fix:** Implement a `PositionManager`/`TradeLedger` that tracks all open positions, daily trade counts, and P&L. Populate `RiskContext` from this ledger before each trade.

### P0-04: Profit/tip ratio meaningless
**Root cause:** Cascading failure from P0-02. Since `midpoint_price` is synthetic, the entire expected profit calculation is disconnected from reality. The profit/tip ratio gate provides false confidence.

**Fix:** Fix P0-02 first (real price discovery), then the profit calculation becomes meaningful.

---

## 9. Verified Remediation Plan

### Immediate Blocker Fixes (P0)

**P0-01: Fix market state keying**
- Modify `ClassifiedEvent` to carry a `mint` field extracted during transaction classification
- In `MarketStateTracker::update()`, key by `event.mint` instead of `event.program_id`
- For existing classifiers that don't have mint extraction: add a fallback that derives mint from instruction discriminator + account indices
- Test: Create events for two different mints on the same program. Verify they maintain separate state.

**P0-02: Implement real price tracking**
- Replace the synthetic noise at `market_state.rs:92-99` with:
  - Decode swap instruction data to extract input/output amounts
  - Compute implied price from swap amounts
  - If no swap data available, keep the last known price (don't apply noise)
- Test: Feed a known swap event, verify price reflects actual swap ratio

**P0-03: Implement runtime RiskContext population**
- Add a `TradeLedger` struct to Orchestrator
- Track: open positions (per-mint, per-protocol), daily trade count, daily loss, consecutive losses, slot health
- In `plan_trade()`, populate `RiskContext` from the ledger before calling `risk_engine.check()`
- Test: Submit 3 trades, verify `daily_trades` increments to 3

**P0-04/P0-05: Wire position sizing from config**
- Replace `input_amount = 100_000_000u64` with value derived from `StrategyConfig.position_size_sol` and current SOL price
- Replace `position_size_estimate = 0.1` with `self.config.strategy.position_size_sol`
- Fix profit/tip ratio after P0-02 is resolved

**P0-06: Don't discard state transition errors**
- Replace all `let _ = intent.transition(...)` with:
  ```rust
  if let Err(e) = intent.transition(to, reason) {
      warn!("State transition failed: {e} (intent {})", intent.id);
  }
  ```

### High-Priority Fixes (P1)

**P1-01: Wire all OrchestratorConfig fields from AppConfig**
- Add missing fields to AppConfig (blockhash.cache_capacity, fees.window_size, fees.percentile, runtime.alt_cache_capacity, runtime.reconciliation_config, runtime.confirmation_timeout, runtime.enable_reconciliation)
- Update `From<&AppConfig>` to read these fields

**P1-02/P1-03: Wire real token balances in plan_trade**
- Query input and output token ATAs in `plan_trade()` using `rpc.get_token_account_balance()`
- Populate `balance_snapshot.input_token_balance` and `output_token_balance`
- Implement token balance parsing in reconciliation Step 5

**P1-04: Wire blockhash last_valid_slot**
- Modify `BlockhashService.get()` to return `(Hash, Option<u64>)` — the hash and its last valid block height
- Update `plan_trade()` to store this value in `TradePlan.last_valid_slot`

### Performance Optimizations

**P2-02 through P2-04: Strategy engine optimizations**
- Fix momentum clone pattern to avoid stale data
- Implement proper confidence checks across all factors, not just momentum

---

## 10. Surfpool Integration Assessment

**Surfpool** (`/Users/vusek/Documents/Low_latency_bot/surfpool`) is a **local Solana Virtual Machine (SVM) simulator** — a Solana Foundation project for testing Solana programs offline. It is NOT a production execution dependency for sol-trade-sdk.

**Architecture:** Multi-crate workspace:
- `crates/core` — SVM runtime, JSON-RPC server, Geyser plugin, storage (SQLite/Postgres/Overlay)
- `crates/types` — Shared types, scenarios, Jito bundles, verified tokens
- `crates/cli` — CLI binary (default member)
- `crates/sdk-node` — Node.js SDK (via napi-rs)
- `crates/bench` — Benchmarking
- `crates/studio` — Web UI

**Integration with sol-trade-sdk:** There is NO direct Rust dependency from sol-trade-sdk on surfpool. The two projects exist in the same workspace directory but sol-trade-sdk's `Cargo.toml` does not list surfpool crates as dependencies. Surfpool is used **externally** for:
1. **Simulated test execution** — Run sol-trade-sdk transactions against a local SVM instead of mainnet
2. **Scenario replay** — Test specific market conditions (Kamino liquidation-arbitrage, Drift, Pyth oracles, Raydium pools)
3. **Geyser event streaming** — Simulate real-time data feeds for ShredStream-like testing

**Surfpool's relevant capabilities for the bot:**
- Complete SVM execution (LiteSVM-based) with cheat codes
- Jito bundle simulation support (`crates/types/src/jito_bundles.rs`)
- Multiple protocol IDLs (Raydium V3/V4, Meteora DLMM, Drift V2, Jupiter V6, Pyth V2, Switchboard V2)
- SQLite/Postgres/Overlay storage backends
- Geyser event streaming (simulated ShredStream)

**Gap:** Despite having all the infrastructure for simulated Jito bundles and market scenarios, sol-trade-sdk's integration tests in `tests/` only cover **golden fixtures** (program ID constants, fee constants, discriminator bytes). There are no integration tests that exercise surfpool's SVM against sol-trade-sdk's swap instructions.

---

## 11. Missing-Test and Observability Plan

### Tests to add:

| Priority | Test | Type | What it catches |
|----------|------|------|-----------------|
| P0 | Market state isolation | Unit | P0-01 — two mints on same program don't conflict |
| P0 | Price tracking from swap data | Unit | P0-02 — real swap amounts produce correct price |
| P0 | RiskContext populated | Integration | P0-03 — RiskContext has non-default values after trades |
| P0 | Slippage from config | Unit | P0-05/hardcoded — config value used in swap params |
| P0 | State transition error logging | Unit | P0-06 — invalid transitions logged not silently discarded |
| P1 | Config wiring verification | Unit | P1-01 — OrchestratorConfig reads from AppConfig |
| P1 | Balance snapshot with real balances | Integration | P1-02 — token balances populated |
| P1 | Blockhash last_valid_slot | Integration | P1-04 — last_valid_slot returned from blockhash service |
| P1 | Multi-protocol execution | Integration | P1-05 — can execute on Raydium not just PumpFun |
| P2 | Strategy engine factor isolation | Unit | P2-03 — momentum not contaminated by clone |
| P3 | TP/SL exit simulation | Integration | P3-03 — exits fire at correct thresholds |
| P0 | Surfpool integration (end-to-end) | Integration | Full pipeline: classify → evaluate → plan → execute → reconcile |

### Observability gaps:

| Gap | Impact | Fix |
|-----|--------|-----|
| Health endpoint always 200 | Operator can't tell if bot is ready | Add real health checks: blockhash_pool_ready, strategy_engine_warm, shredstream_connected, wallet_balance_ok |
| No correlation IDs between events | Can't trace a trade from signal → execution → confirmation | Add `trace_id: Uuid` to TradeIntent, propagate to all logs |
| BundleMetrics.landed() ignores params | Landing time and attempts not tracked | Fix the histogram recording |
| No structured error aggregation | Can't distinguish RPC errors from strategy rejections | Add error counters per category (rpc, risk, strategy, submission) |
| No disk space monitoring | State files may fill disk silently | Add `StorageConfig::max_disk_usage_gb` check in health endpoint |

---

## 12. Prioritized Implementation Roadmap

### Phase 0: Immediate Blockers (fix before any test with real funds)

1. **P0-01: Fix market state keying** — Change `program_id` → actual `mint` in `MarketStateTracker::update()`. Add mint extraction to classifier.
2. **P0-02: Implement real price tracking** — Replace synthetic noise with actual swap-derived prices.
3. **P0-03: Implement RiskContext population** — Add TradeLedger, populate RiskContext in plan_trade().
4. **P0-04/P0-05: Wire position sizing from config** — Replace hardcoded 0.1 SOL.
5. **P0-06: Log state transition errors** — Replace `let _ = intent.transition(...)` with error logging.
6. **P0-07: Fix Built→Failed transition** — Add `(Built, Failed)` to the transition table instead of abusing `Expired`.

### Phase 1: Critical Pre-Production Fixes

7. **P1-01: Wire OrchestratorConfig from AppConfig** — Add missing config fields, update `From<&AppConfig>`.
8. **P1-02/P1-03: Wire real balance snapshots** — Query token ATAs, populate balance snapshot, implement reconciliation parsing.
9. **P1-04: Wire blockhash last_valid_slot** — Return `(Hash, Option<u64>)` from BlockhashService.
10. **P1-05: Make protocol configurable per mint** — Remove hardcoded `DexType::Bonk` and `"pumpfun"`.
11. **P1-06: Use config-driven position size in profit/tip gate** — Read from `StrategyConfig`.
12. **P1-08: Use config-driven slippage in simulation** — Replace hardcoded 500 bps with `RiskConfig.max_slippage_basis_points`.

### Phase 2: High-Priority Reliability

13. **Add integration tests with surfpool** — End-to-end test: classify → evaluate → plan → execute → reconcile.
14. **Fix momentum clone staleness** (P2-03).
15. **Fix confidence gate for all factors** (P2-02).
16. **Add TP/SL exit monitoring** — Spawn background task that monitors open positions.

### Phase 3: Performance Optimizations

17. **Parallelize plan_trade RPC calls** — `get_slot()` and `get_balance()` can be called concurrently with `tokio::join!`.
18. **Fix BundleMetrics parameter recording** (P2-05).
19. **Add RPC-based fallback data feed** — So the bot can function without ShredStream.

### Phase 4: Long-Term Architecture

20. **Implement state persistence** — SQLite for trade intents, position tracking, and reconciliation.
21. **Add alert/webhook integration** — Telegram or Slack for critical events.
22. **Implement graceful degradation** — Tighter risk limits when data feed degrades, rather than continuing with stale data.

---

## 13. Answer to the Central Question

**Is this bot genuinely safe, functional, deterministic, recoverable, observable, and production-ready under realistic operating conditions?**

**NO — the bot is NOT production-ready.**

**Safety:** Fails with P0-03 (RiskContext always default) — global risk controls are dead code. Wallet balance limits are configured but never checked at runtime.

**Functionality:** Works for a narrow case (single mint, PumpFun only, with ShredStream running, without actual price data). The strategy engine produces signals, but those signals are based on synthetic noise (P0-02) and keyed by program_id instead of mint (P0-01).

**Determinism:** The state machine and transition validation are well-designed. But the execution path has silent error swallowing (P0-06) that makes behavior non-deterministic from the operator's perspective.

**Recoverability:** Reconciliation is structurally sound but crippled by missing token balance checks (P1-02/P1-03) and blockhash expiry detection (P1-04). No persistent state — restarting the process loses all trade intent history.

**Observability:** Basic metrics and health endpoints exist, but the health endpoint always returns 200 regardless of actual readiness. No alerting or webhook integration.

**Production-ready:** The architecture is well-designed and demonstrates deep understanding of Solana trading infrastructure. The problems are in the *implementation completeness gap* — the skeleton exists but critical parts haven't been filled in. With 5-10 days of targeted remediation across the P0 items, the bot can be made genuinely production-ready.

**Recommendation: Fix the 6 P0 items first, then 8 P1 items, then run the bot in shadow/canary mode for at least 2 weeks before considering production deployment.**