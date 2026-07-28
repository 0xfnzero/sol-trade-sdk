# Risk Controls Matrix & Fee Engine Design

> Design document defining all risk controls and the fee engine for `sol-trade-sdk`. Based on pre-flight section 33 (Risk Controls) and section 26 (Fee Engine). These controls are **hard limits** — enforced at the strategy and execution boundary, not advisory.

---

## 1. Architecture Overview

```
                    ┌────────────────────┐
                    │   Risk Governor    │  ← Singleton, holds all hard limits
                    │   (Arc<RwLock<>>)  │     Evaluates every trade intent
                    └────────┬───────────┘
                             │
        ┌────────────────────┼────────────────────┐
        │                    │                    │
   ┌────▼─────┐       ┌─────▼──────┐       ┌─────▼─────┐
   │Per-Trade  │       │  Global    │       │   Health  │
   │Controls   │       │  Controls  │       │  Controls │
   └───────────┘       └────────────┘       └───────────┘
        │                    │                    │
        └────────────────────┼────────────────────┘
                             │
                    ┌────────▼─────────┐
                    │  Fee Engine      │  ← Dynamic fee calculation
                    │  (per-strategy)  │     + ceiling enforcement
                    └──────────────────┘
```

---

## 2. Hard Limits — Risk Governor

### 2.1 Per-Trade Controls

These limits are evaluated **before** building the transaction. If any check fails, the trade intent is **REJECTED** (not deferred, not queued).

| # | Control | Type | Default | Rationale |
|---|---|---|---|---|
| RT1 | `max_sol_per_trade` | `u64` (lamports) | 1.0 SOL | Cap single-trade SOL spend |
| RT2 | `max_token_amount_per_trade` | `u64` (smallest unit) | 1_000_000_000_000 | Prevent position-override bugs |
| RT3 | `max_slippage_basis_points` | `u64` (bps) | 10,000 (100%) | Absolute max; individual strategies set tighter |
| RT4 | `max_quote_age_us` | `u64` (micros) | 500_000 (500ms) | Reject stale quotes; source-slot gap proxy |
| RT5 | `max_source_slot_age` | `u64` (slots) | 3 | Event slot must be within N of current |
| RT6 | `min_expected_output` | `u64` (smallest unit) | 1 | Minimum output after slippage; prevent dust |
| RT7 | `min_net_profit_lamports` | `i64` (lamports, can be negative) | 0 | Expected profit minus total fees must exceed this |
| RT8 | `max_tx_size_bytes` | `usize` | 1232 | Solana packet limit; enforced at build time |

**Enforcement point:** `RiskGovernor::check_trade_intent(&self, intent: &ValidatedTradeIntent) -> Result<()>` — called by strategy engine before passing intent to execution.

### 2.2 Global Position Controls

| # | Control | Type | Default | Rationale |
|---|---|---|---|---|
| RG1 | `max_open_exposure_sol` | `u64` (lamports) | 100.0 SOL | Total open position value (pending buys + held tokens at cost basis) |
| RG2 | `max_exposure_per_mint` | `HashMap<Pubkey, u64>` | 10.0 SOL per mint | Prevent over-concentration |
| RG3 | `max_exposure_per_protocol` | `HashMap<DexType, u64>` | 50.0 SOL per protocol | Diversification across venues |
| RG4 | `max_tx_per_minute` | `u64` | 60 | Rate-limit to prevent gas waste |
| RG5 | `max_consecutive_failures` | `u64` | 10 | Circuit-break on repeated failures |
| RG6 | `max_failures_per_interval` | `(u64, Duration)` | (20, 300s) | Rolling-window failure count |
| RG7 | `max_open_orders` | `usize` | 50 | Concurrent unconfirmed trades |

**Enforcement point:** `RiskGovernor::check_global_limits(&self, context: &MarketContext) -> Result<()>` — checked before each trade. State maintained in an in-memory `RiskState` struct with atomic counters.

### 2.3 Fee Controls

| # | Control | Type | Default | Rationale |
|---|---|---|---|---|
| RF1 | `max_priority_fee_lamports` | `u64` (lamports) | 0.01 SOL (10M lamports) | Absolute cap on compute unit price × unit limit |
| RF2 | `max_relay_tip_lamports` | `u64` (lamports) | 0.01 SOL | Per-SWQOS-lane tip cap |
| RF3 | `max_total_tx_cost_lamports` | `u64` (lamports) | 0.02 SOL | Priority fee + relay tip + base fee capped together |
| RF4 | `max_daily_loss_lamports` | `u64` (lamports) | 10.0 SOL | Rolling 24h realized loss (sells below cost basis) |
| RF5 | `max_consecutive_loss` | `u64` | 5 | Stop trading after N loss-making trades in a row |

**Enforcement point:** `FeeEngine::cap_fees(...)` — called during transaction building, before signing. If total cost exceeds limits, reduce fees or reject.

### 2.4 Health Controls

| # | Control | Type | Default | Rationale |
|---|---|---|---|---|
| RH1 | `max_rpc_lag_ms` | `u64` (ms) | 5,000 | RPC recent blockhash / balance query timeout |
| RH2 | `max_packet_loss_pct` | `f64` | 0.1% | UDP drops from kernel as fraction of total packets |
| RH3 | `max_queue_delay_us` | `u64` (micros) | 10,000 | Time from event enqueue to dequeue in pipeline |
| RH4 | `max_blockhash_age_slots` | `u64` | 5 | Blockhash cached from blockhash service must be within N slots of tip |
| RH5 | `max_reconciliation_backlog` | `usize` | 1,000 | Pending post-trade reconciliation (signature checks) |

**Enforcement point:** `HealthMonitor` — a background thread that samples each health metric every 500ms and feeds a `DegradedMode` flag. If any health check exceeds its limit, the risk governor transitions to degraded mode (stops accepting new trade intents).

### 2.5 Switches (Kill Switches)

| # | Control | Type | Default | Update mechanism |
|---|---|---|---|---|
| RS1 | `global_kill` | `bool` | `false` | Set `true` to stop ALL trading immediately |
| RS2 | `buy_only` | `bool` | `false` | Only allow buys; reject sells |
| RS3 | `sell_only` | `bool` | `false` | Only allow sells; reject buys |
| RS4 | `per_protocol_disable` | `HashSet<DexType>` | empty | Disable specific protocols (e.g., `{Bonk}` if unstable) |
| RS5 | `per_provider_disable` | `HashSet<SwqosType>` | empty | Disable specific SWQOS lanes |
| RS6 | `per_token_denylist` | `HashSet<Pubkey>` | empty | Never trade these mints (scam tokens) |
| RS7 | `per_token_allowlist` | `Option<HashSet<Pubkey>>` | `None` | When `Some`, ONLY trade mints in this set |

**Enforcement point:** `RiskGovernor::check_switches(&self, intent: &TradeIntent) -> Result<()>` — checked first, before any other control. Kills are instant; the switch state is reloaded atomically via `ArcSwap`.

---

## 3. Risk State Management

### 3.1 In-Memory Risk State

```rust
pub struct RiskState {
    // Global counters
    pub open_exposure_sol: AtomicU64,         // Lamports in open positions
    pub exposure_per_mint: DashMap<Pubkey, u64>,
    pub tx_count: AtomicU64,                   // Rolling 60s tx count
    pub failure_count: AtomicU64,              // Rolling interval
    pub consecutive_losses: AtomicU64,
    pub daily_loss: AtomicU64,                 // Rolling 24h
    
    // Per-minute rate limiting
    pub tx_minute_bucket: TokenBucket,         // Refills at rate per minute
    
    // Per-trade state
    pub open_orders: AtomicUsize,              // Concurrent unconfirmed trades
    pub reconciliation_backlog: AtomicUsize,   // Pending signature checks
}
```

### 3.2 Persistence

Risk state MUST be persisted to disk on each update (async, off the hot path):

- Write-ahead log for position state updates (every trade open/close)
- Snapshot to `risk_state.json` every 60 seconds
- On restart, replay the snapshot; stale entries older than 24h are purged

### 3.3 Thread Safety

- `RiskGovernor` is behind `Arc<RwLock<RiskState>>` for global state
- Per-trade controls are stateless (just parameter checks) — no lock needed
- Switches use `ArcSwap<RiskSwitches>` for lock-free reads
- Health state uses `AtomicBool` for the degraded flag

---

## 4. Degraded Mode & Automatic Recovery

| Condition | Action | Recovery |
|---|---|---|
| Any health check exceeds limit | Set `degraded = true`, stop new intents | All health checks normal for 30s → `degraded = false` |
| `max_consecutive_failures` reached | Set `degraded = true`, log alert | Manual reset or 5min cooldown |
| `global_kill = true` | Immediate stop | Manual `global_kill = false` |
| Queue backlog > threshold | Slow new intents (rate limit) | Backlog drops below 50% |
| Packet loss > 0.1% | Degraded — warn only | Drops below 0.05% for 60s |

---

## 5. Fee Engine Design (Section 26)

### 5.1 Overview

The Fee Engine dynamically computes transaction fees (compute unit price + limit + relay tip) per (protocol, trade-type, SWQOS-lane) tuple. It balances:
- **Landing speed** (higher fee = faster inclusion)
- **Cost efficiency** (don't overpay for low-value trades)
- **Absolute ceiling** (never exceed hard limits)

```
              ┌──────────────────┐
              │  FeeEngine       │
              │  (per-instance)  │
              └────────┬─────────┘
                       │
        ┌──────────────┼──────────────┐
        │              │              │
   ┌────▼────┐   ┌────▼────┐   ┌─────▼─────┐
   │CU Tracker│   │PriorFee │   │ Threshold  │
   │per route │   │ Oracle  │   │ Calculator │
   └─────────┘   └─────────┘   └───────────┘
```

### 5.2 Per-Route Compute Unit Tracking

**Goal:** Determine the correct `cu_limit` for each (protocol, trade-type) combination.

**Method:**

1. **Simulate** representative transactions for each combination (run `simulateTransaction` via RPC with `replace_recent_blockhash=true`).
2. **Record** `units_consumed` from the simulation response.
3. **Collect** samples over time into a circular buffer (last 100 simulations per route).
4. **Set limit** = p99 of recorded units + 20% headroom.

| Route | Simulated Samples | p50 CU | p95 CU | p99 CU | Proposed Limit |
|---|---|---|---|---|---|
| PumpFun buy | 100 | 12,000 | 15,000 | 18,000 | 22,000 |
| PumpFun sell | 100 | 15,000 | 20,000 | 25,000 | 30,000 |
| PumpSwap buy | 100 | 25,000 | 35,000 | 42,000 | 51,000 |
| PumpSwap sell | 100 | 30,000 | 40,000 | 48,000 | 58,000 |
| Raydium CPMM swap | 100 | 30,000 | 40,000 | 50,000 | 60,000 |
| Meteora swap | 100 | 35,000 | 50,000 | 60,000 | 72,000 |
| Bonk buy | 100 | 18,000 | 25,000 | 30,000 | 36,000 |

The `proposed_limit` is set at `p99 * 1.2` and clamped to `[MIN_CU, MAX_CU]` where `MIN_CU = 10,000` and `MAX_CU = 200,000`.

**Refresh cadence:** Re-simulate every 60 minutes or if any simulation returns a value > current limit.

**State:** Persisted to `cu_limits.json` — loaded at startup, updated in-place.

### 5.3 Recent Prioritization Fee Query

**Goal:** Determine `cu_price` (price per compute unit, in microlamports) that will land the transaction promptly.

**Method:**

1. Query `getRecentPrioritizationFees` via RPC (background thread, every 10 seconds).
2. Cache the last 20 samples per slot (sliding window).
3. Compute:

```rust
fn compute_recommended_cu_price() -> u64 {
    let samples = PRIOR_FEE_CACHE.latest(20);
    if samples.is_empty() { return DEFAULT_CU_PRICE; }  // e.g. 100_000
    
    // Use p70 of recent fees — aggressive enough to land quickly,
    // cheap enough to avoid overpaying
    let p70 = percentile(&samples, 0.70);
    
    // Clamp to [MIN_CU_PRICE, MAX_CU_PRICE]
    p70.clamp(MIN_CU_PRICE, MAX_CU_PRICE)
}
```

| Constant | Value |
|---|---|
| `MIN_CU_PRICE` | 10,000 microlamports |
| `MAX_CU_PRICE` | 10,000,000 microlamports (0.01 SOL max for priority) |
| DEFAULT_CU_PRICE | 100,000 microlamports |
| `PRIOR_FEE_PERCENTILE` | p70 |

**Edge case:** When `getRecentPrioritizationFees` returns empty (new epoch, no recent txs), fall back to `DEFAULT_CU_PRICE`.

### 5.4 Dynamic Fee with Absolute Ceiling

The total fee for a transaction is:

```text
total_fee = base_fee(5000) 
          + priority_fee(cu_limit * cu_price / 1_000_000)     // in lamports
          + relay_tip                                          // in lamports
```

Where:
- `cu_limit` = from per-route tracking (Section 5.2)
- `cu_price` = from prioritization fee oracle (Section 5.3)
- `relay_tip` = configurable per SWQOS lane (part of `GasFeeStrategy`)
- `base_fee` = 5000 lamports (current Solana base fee for a 2-signature tx)

**Absolute ceiling:**

```rust
fn cap_total_fees(&self, expected_profit_lamports: i64, fees: TxFee) -> Result<TxFee> {
    // Hard ceiling from Risk Governor (RF3)
    if fees.total() > self.rf3_max_total_tx_cost {
        return Err(anyhow!("total fee {} exceeds hard ceiling {}", fees.total(), self.rf3_max_total_tx_cost));
    }
    
    // Proportional cap: fees must not exceed min(profit * 0.5, hard_ceiling)
    let max_by_profit = (expected_profit_lamports as u64).saturating_mul(50) / 100;
    let effective_cap = max_by_profit.min(self.rf3_max_total_tx_cost);
    
    if fees.total() > effective_cap {
        // Scale down proportionally
        return Ok(fees.scale_to(effective_cap));
    }
    
    Ok(fees)
}
```

### 5.5 Net Profit Threshold

Before any trade execution:

```rust
fn check_profit_threshold(&self, intent: &ValidatedTradeIntent, fees: TxFee) -> Result<()> {
    let expected_gross_profit = intent.expected_output
        .saturating_sub(intent.cost_basis);
    
    let net_profit = (expected_gross_profit as i64)
        - (fees.total() as i64)
        - (intent.expected_slippage_cost as i64);
    
    if net_profit < self.rf7_min_net_profit_lamports {
        return Err(anyhow!("net profit {} below threshold {}", 
            net_profit, self.rf7_min_net_profit_lamports));
    }
    
    Ok(())
}
```

### 5.6 Cap Total Fees Against Expected Profit

If the total fee exceeds a configurable fraction of expected profit, the trade is rejected or fees are reduced:

```rust
pub struct FeeCapPolicy {
    /// Max fee-to-profit ratio (e.g., 0.30 = 30%)
    pub max_fee_to_profit_ratio: f64,
    /// When true, reduce fees down to the ratio cap instead of rejecting
    pub reduce_fees_on_overflow: bool,
    /// Minimum cu_price even after reduction (never go below this)
    pub min_cu_price_after_reduction: u64,
}
```

Default: `max_fee_to_profit_ratio = 0.30`, `reduce_fees_on_overflow = true`, `min_cu_price_after_reduction = 10_000`.

### 5.7 Fee Engine API

```rust
pub struct FeeEngine {
    // Per-route CU tracking
    cu_limits: Arc<DashMap<(DexType, TradeType), CuLimitRecord>>,
    // Prioritization fee cache
    prior_fee_cache: Arc<RwLock<PriorFeeCache>>,
    // Configuration
    config: FeeEngineConfig,
}

impl FeeEngine {
    /// Called at strategy evaluation time — before trade build
    pub fn compute_fees(
        &self,
        dex_type: DexType,
        trade_type: TradeType,
        swqos_type: SwqosType,
        strategy_type: GasFeeStrategyType,
        expected_profit_lamports: i64,
    ) -> Result<TxFee>;
    
    /// Update CU limits from simulation results
    pub fn record_simulated_cu(&self, dex_type: DexType, trade_type: TradeType, cu_consumed: u32);
    
    /// Refresh prioritization fees from RPC (called by background thread)
    pub async fn refresh_prior_fees(&self, rpc_client: &RpcClient) -> Result<()>;
}
```

### 5.8 Integration with GasFeeStrategy

The Fee Engine outputs feed into the existing `GasFeeStrategy`:

```rust
// FeeEngine produces:
let tx_fee = fee_engine.compute_fees(
    dex_type, trade_type, swqos_type, gas_fee_strategy_type, expected_profit
)?;

// This is then used as the actual fee values for the GasFeeStrategy
gas_fee_strategy.set_normal_fee_strategy(
    swqos_type,
    tx_fee.cu_limit,       // dynamically computed
    tx_fee.cu_price,       // from prioritization oracle
    tx_fee.relay_tip,      // from lane config, capped
    tx_fee.relay_tip,      // sell tip (same when using normal)
);
```

For the **high-low fee strategy** (dual lane), the Fee Engine produces two sets of fees:

```rust
let (high_price, low_price) = fee_engine.compute_dual_lane_fees(...);
gas_fee_strategy.set_high_low_fee_strategy(
    swqos_type,
    trade_type,
    high_price.cu_limit,
    low_price.cu_price,      // Low cu_price for high-tip lane
    high_price.cu_price,     // High cu_price for low-tip lane
    low_price.relay_tip,     // Low tip for high-cu_price lane
    high_price.relay_tip,    // High tip for low-cu_price lane
);
```

---

## 6. Observable Controls (Metrics)

Every control check produces a metric:

| Metric | Labels | Type | Description |
|---|---|---|---|
| `risk_check_total` | `{control: "RT1".."RS7", result: "pass"|"reject"}` | Counter | Total checks, per control |
| `risk_current_exposure` | — | Gauge | Current open exposure in SOL |
| `risk_current_degraded` | — | Gauge (0/1) | Whether degraded mode is active |
| `fee_engine_cu_limit` | `{protocol, trade_type}` | Gauge | Current CU limit per route |
| `fee_engine_cu_price` | — | Gauge | Current recommended cu_price |
| `fee_engine_oracle_age_ms` | — | Gauge | Age of prioritization fee data |
| `trade_loss_streak` | — | Gauge | Current consecutive loss count |

---

## 7. Configuration File Design

Controls are loaded from `risk_config.yaml` at startup:

```yaml
risk_controls:
  per_trade:
    max_sol_per_trade: 1_000_000_000        # 1 SOL in lamports
    max_slippage_basis_points: 1000         # 10%
    max_quote_age_us: 500_000               # 500ms
    max_source_slot_age: 3
    min_net_profit_lamports: 0
  global:
    max_open_exposure_sol: 100_000_000_000  # 100 SOL
    max_exposure_per_mint: 10_000_000_000   # 10 SOL per mint
    max_tx_per_minute: 60
    max_consecutive_failures: 5
  fee:
    max_priority_fee_lamports: 10_000_000   # 0.01 SOL
    max_relay_tip_lamports: 10_000_000      # 0.01 SOL
    max_total_tx_cost_lamports: 20_000_000  # 0.02 SOL
    max_daily_loss_lamports: 10_000_000_000 # 10 SOL
  health:
    max_rpc_lag_ms: 5000
    max_packet_loss_pct: 0.1
    max_blockhash_age_slots: 5

fee_engine:
  cu_limit_headroom: 1.2                    # 20% margin over p99
  cu_limit_refresh_interval_s: 3600
  prior_fee_percentile: 0.70
  prior_fee_refresh_interval_s: 10
  max_fee_to_profit_ratio: 0.30
  reduce_fees_on_overflow: true
  min_cu_price: 10_000
  max_cu_price: 10_000_000
  default_cu_price: 100_000

switches:
  global_kill: false
  buy_only: false
  sell_only: false
  per_protocol_disable: []
  per_provider_disable: []
  per_token_denylist: []
  # per_token_allowlist: ~   # null = allow all
```

---

## 8. Initialization Sequence

```text
Startup:
1. Load risk_config.yaml → RiskGovernorConfig
2. Load cu_limits.json → FeeEngine (or compute defaults)
3. Initialize RiskState (zeroed counts)
4. Start PriorFeeOracle background thread (every 10s)
5. Start HealthMonitor background thread (every 500ms)
6. Start CU limit refresh timer (every 60min)
7. Verify all controls loaded successfully → log confirmation

Hot path (per event):
1. [Strategy] Build trade intent
2. [Risk] Check switches (RS1-RS7)
3. [Risk] Check per-trade limits (RT1-RT8)
4. [FeeEngine] Compute fees + check profit threshold
5. [Risk] Check global limits (RG1-RG7)
6. [Risk] Apply fee controls (RF1-RF5)
7. [Execution] Build + sign + submit
8. [Reconciliation] On result: update RiskState counters
```

---

*This matrix is a design specification. Implementation MUST enforce every control at the specified enforcement point. No control should be bypassable from user-facing APIs.*