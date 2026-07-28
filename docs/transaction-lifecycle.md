# Transaction Lifecycle Architecture

> **Design document**: Blockhash Cache Service, Durable Nonce Policy, Transaction Builder, Multi-Lane Submission & Deduplication
> **Related**: `state-model.md`, `reconciliation.md`
> **Pre-flight sections**: 22, 23, 25, 29

---

## 1. Blockhash Cache Service

### 1.1 Purpose

The Blockhash Cache Service provides a continuously-refreshed source of recent blockhashes for transaction construction, decoupling the low-latency trading path from RPC `getLatestBlockhash` calls. It eliminates the RPC round-trip latency that would otherwise occur on every trade.

### 1.2 Struct Design

```rust
/// A single cached blockhash entry with metadata
pub struct CachedBlockhash {
    /// The blockhash value (32 bytes)
    pub blockhash: Hash,
    /// Monotonic timestamp when this entry was fetched from RPC
    pub fetched_at: Instant,
    /// The context slot reported by the RPC at fetch time
    pub context_slot: u64,
    /// The last valid block height for this blockhash (from RPC response)
    pub last_valid_block_height: u64,
    /// Which RPC source provided this blockhash
    pub source: RpcSource,
    /// Commitment level used to fetch this blockhash
    pub commitment: CommitmentLevel,
}

/// Identifies which RPC node provided a blockhash
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcSource {
    Primary,
    Fallback(u8),
}

/// Blockhash cache configuration
pub struct BlockhashCacheConfig {
    /// How often to refresh the cached blockhash (slots or milliseconds)
    pub refresh_interval: Duration,
    /// Commitment level for fetching (Processed for lowest latency, Confirmed for safety)
    pub commitment: CommitmentLevel,
    /// How far below MAX_PROCESSING_AGE (150 slots) to trigger a refresh
    pub refresh_margin_slots: u64,
    /// Primary RPC endpoint
    pub primary_rpc: String,
    /// Optional fallback RPC endpoints for failover
    pub fallback_rpcs: Vec<String>,
    /// Whether to use blockhash subscription instead of polling (requires Geyser/RPC pubsub)
    pub use_subscription: bool,
}

/// The blockhash cache runtime
pub struct BlockhashCacheService {
    /// Current cached entry (behind RwLock for concurrent readers)
    current: RwLock<Option<CachedBlockhash>>,
    /// Refresh task handle
    refresh_task: Option<JoinHandle<()>>,
    /// Shutdown signal
    shutdown: Arc<AtomicBool>,
    /// RPC clients (primary + fallbacks)
    rpc_clients: Vec<Arc<SolanaRpcClient>>,
    /// Configuration
    config: BlockhashCacheConfig,
    /// Metrics
    metrics: Arc<BlockhashMetrics>,
}
```

### 1.3 Refresh Loop

```
┌──────────────────────────────────────┐
│         Refresh Loop                 │
│  (spawned as background tokio task)  │
└──────┬───────────────────────────────┘
       │
       ▼
┌──────────────────┐     ┌──────────────────────┐
│  Sleep interval  │────►│  Fetch from Primary  │
│  or slot trigger │     │  getLatestBlockhash  │
└──────────────────┘     │  (Processed commit)  │
                         └──────────┬───────────┘
                                    │
                         ┌──────────▼───────────┐
                         │  Response valid?     │
                         │  - has context_slot  │
                         │  - has blockhash     │
                         │  - has last_valid    │
                         │    block_height      │
                         └──────┬──────┬────────┘
                                │      │
                           YES  │      │  NO
                                │      │
                         ┌──────▼      ▼────────────────┐
                         │ Update cache │ Try fallback   │
                         │  with new    │ RPC in round-  │
                         │  CachedBlock-│ robin fashion  │
                         │  hash entry  │                │
                         └──────┬──────┴────────────────┘
                                │
                         ┌──────▼──────┐
                         │ Emit metrics│
                         │  - age      │
                         │  - slot lag │
                         │  - source   │
                         └─────────────┘
```

**Refresh trigger conditions**:
- **Time-based**: Every N milliseconds (configurable, default 200ms)
- **Slot-based**: When current slot >= last_valid_block_height - refresh_margin_slots
- **Exhaustion**: When no valid blockhash exists (first fetch or after expiry)

### 1.4 Failover Between RPC Nodes

```rust
impl BlockhashCacheService {
    /// Attempt to fetch a blockhash, trying fallback RPCs on failure
    async fn fetch_blockhash(&self) -> Result<CachedBlockhash> {
        // Round-robin through all RPC clients
        let mut last_error = None;
        for (i, client) in self.rpc_clients.iter().enumerate() {
            match self.fetch_from_client(client, i).await {
                Ok(entry) => return Ok(entry),
                Err(e) => {
                    tracing::warn!("Blockhash fetch failed on RPC[{}]: {:?}", i, e);
                    last_error = Some(e);
                    continue;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("all RPCs exhausted")))
    }

    async fn fetch_from_client(
        &self,
        client: &SolanaRpcClient,
        client_idx: usize,
    ) -> Result<CachedBlockhash> {
        let (blockhash, context) = client
            .get_latest_blockhash_with_commitment(
                CommitmentConfig { commitment: self.config.commitment }
            )
            .await?;

        Ok(CachedBlockhash {
            blockhash: blockhash.0,
            fetched_at: Instant::now(),
            context_slot: context.context_slot,
            last_valid_block_height: blockhash.1,
            source: match client_idx {
                0 => RpcSource::Primary,
                n => RpcSource::Fallback((n - 1) as u8),
            },
            commitment: self.config.commitment,
        })
    }
}
```

### 1.5 Consumer API

```rust
impl BlockhashCacheService {
    /// Get a valid blockhash for transaction construction.
    /// Returns BlockhashNotFound error (not retried by caller) when cache is empty.
    pub fn get_valid_blockhash(&self) -> Result<CachedBlockhash, BlockhashError>;

    /// Get current blockhash without validity check (for diagnostics/metrics)
    pub fn peek_blockhash(&self) -> Option<CachedBlockhash>;

    /// Check if the cached blockhash is still within its validity window
    pub fn is_current(&self, entry: &CachedBlockhash) -> bool;

    /// Return age in slots since context_slot
    pub fn age_in_slots(&self, current_slot: u64) -> u64;

    /// Return remaining validity window in slots
    /// (last_valid_block_height - current_slot)
    pub fn remaining_validity(&self, current_slot: u64) -> u64;

    /// Expose metrics for monitoring/prometheus
    pub fn metrics(&self) -> &BlockhashMetrics;
}

/// Lifecycle error: returned when no blockhash is available.
/// The submitting layer MUST NOT retry — this is a pre-flight failure.
pub enum BlockhashError {
    CacheEmpty,
    AllRpcSourcesFailed { attempts: usize, last_error: String },
}
```

### 1.6 Metric Exposure

```
BlockhashCacheMetrics:
  - blockhash_age_ms: histogram
  - blockhash_slot_lag: gauge (current_slot - context_slot)
  - blockhash_remaining_slots: gauge
  - blockhash_refresh_count: counter
  - blockhash_failover_count: counter (per fallback index)
  - blockhash_stale_count: counter (get_valid_blockhash called when stale)
  - blockhash_not_found_count: counter (get_valid_blockhash called when empty)
```

### 1.7 Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Background refresh vs on-demand | Eliminates RPC round-trip on trade hot path |
| RwLock vs atomic | Entry is too large for atomic; RwLock allows concurrent reads with rare writes |
| Processed commitment by default | Lowest latency; trade safety for landing guarantees |
| BlockhashNotFound is terminal | If cache is empty, transaction cannot be safely built — retrying the same cache lookup will repeat the same failure |
| Failover per fetch, not per trade | Ensures cache stays populated even through transient RPC failures |
| No subscription fallback | Polling is simpler and more reliable; subscription would add geyser dependency |

---

## 2. Durable Nonce Policy

### 2.1 Design Principles

1. **Disabled by default in production** — nonce accounts add management overhead and an extra System Program instruction (AdvanceNonceAccount) that consumes compute budget and increases transaction size.
2. **Explicit opt-in** — only enabled when the user configures a nonce account address and the use case requires offline transaction validity (e.g., pre-signed transactions, delayed execution beyond the 150-slot blockhash window).
3. **One owner per nonce account** — each nonce account is owned by one payer keypair. Sharing a nonce account across multiple concurrent trading wallets creates contention and nonce-advance ordering failures.

### 2.2 Nonce Flow

```
┌──────────────┐     ┌──────────────────────┐
│ fetch_nonce  │────►│ Transaction Builder  │
│ _info(RPC)   │     │ inserts              │
│              │     │ AdvanceNonceAccount  │
│ Returns:     │     │ instruction at       │
│ DurableNonce │     │ position 0           │
│ Info {       │     │                      │
│   nonce_acct,│     │ Sets recent_blockhash│
│   nonce_hash │     │ = nonce_account's    │
│ }            │     │ stored durable nonce │
└──────────────┘     └──────────┬───────────┘
                                │
                     ┌──────────▼───────────┐
                     │ Signed & submitted   │
                     │                      │
                     │ Validator checks:    │
                     │  1. Nonce account    │
                     │     owned by System  │
                     │  2. State::Init      │
                     │  3. stored nonce ==  │
                     │     recent_blockhash │
                     │  4. nonce != next    │
                     │     nonce (not used) │
                     │  5. nonce authority  │
                     │     signed tx        │
                     │                      │
                     │ If valid: nonce is   │
                     │ advanced atomically  │
                     └──────────────────────┘
```

### 2.3 Nonce Advance Instruction Placement

The `AdvanceNonceAccount` instruction MUST be the **first instruction** in the transaction message. This is required by the Solana runtime: if any other instruction executes before the nonce advance, the nonce validation will fail. The runtime processes instructions in order, and the nonce check happens as part of instruction 0's execution.

```rust
// Current implementation in nonce_manager.rs:
pub fn add_nonce_instruction(
    instructions: &mut Vec<Instruction>,
    payer: &Keypair,
    durable_nonce: Option<&DurableNonceInfo>,
) -> Result<(), anyhow::Error> {
    if let Some(durable_nonce) = durable_nonce {
        let nonce_advance_ix = advance_nonce_account(
            &durable_nonce.nonce_account.unwrap(),
            &payer.pubkey(),
        );
        // Push to FRONT of instructions list
        instructions.insert(0, nonce_advance_ix);  // <-- Must be position 0
    }
    Ok(())
}
```

### 2.4 Concurrent-Use Prevention

When a durable nonce is in use:
- **One in-flight transaction per nonce value** — the nonce is consumed (advanced) as soon as a transaction using it lands. A second transaction using the same nonce value will fail with `BlockhashNotFound` because the nonce account's stored value no longer matches.
- **Multi-lane submission with nonce** — When using multiple SWQOS lanes with a durable nonce, every lane builds a transaction with the SAME nonce value. At most ONE will land. The nonce guarantees that only the first to land succeeds; all others fail at the nonce check. This is correct behavior — the deduplication is enforced by the protocol itself.
- **Restart recovery** — After a restart or crash, the caller must call `fetch_nonce_info()` again to get the fresh nonce value. The old nonce value is no longer valid because the nonce account was advanced (or the previous transaction was dropped).

### 2.5 When to Enable (and NOT Enable)

**Enable when**:
- Building pre-signed transactions that must outlive the 150-slot blockhash window
- Delayed execution (e.g., time-locked swaps, scheduled orders)
- Cross-session transactions where the same transaction bytes need validity across process restarts

**DO NOT enable when**:
- Running latency-sensitive HFT / market-making bots (the extra instruction adds ~200 CU and ~1-2μs build time)
- All trades land within 1-2 slots (recent blockhash is sufficient)
- You cannot guarantee exclusive access to the nonce account

### 2.6 Nonce Account Lifecycle Management

```
1. CREATE:  SystemProgram::create_nonce_account(...)
            → Creates account + sets initial nonce value
            → Owner = payer (nonce authority)

2. FETCH:   fetch_nonce_info(rpc, nonce_pubkey)
            → Reads account data, parses CurrentHash at offset 40
            → Returns DurableNonceInfo { nonce_account, current_nonce }

3. CONSUME: Transaction builder inserts AdvanceNonceAccount IX
            → Validator advances nonce atomically on success
            → On failure (non-execution): nonce NOT advanced (fee still collected)
            → On failure (execution): nonce IS advanced (fee + rollback accounts)

4. REFRESH: fetch_nonce_info(rpc, nonce_pubkey) — again
            → Gets the NEW current_nonce for the next transaction
```

---

## 3. Transaction Builder

### 3.1 Design: From Raw Params to TradeIntent

**Current design**: `build_transaction()` accepts raw parameters (payer, instructions, blockhash, tip, etc.) and produces a signed `VersionedTransaction`.

**New design**: The builder accepts a validated `TradeIntent` rather than raw parameters. This ensures:
1. All pre-flight checks happen in one place
2. State machine enforces that only `VALIDATED` intents reach the builder
3. Consistent error handling (all pre-flight failures map to `REJECTED`)

```rust
/// Transaction builder — accepts only validated TradeIntent
pub struct TradeTransactionBuilder {
    blockhash_cache: Arc<BlockhashCacheService>,
    config: TransactionBuilderConfig,
}

pub struct TransactionBuilderConfig {
    /// Max allowed transaction age in slots (default: 100, must be < 150)
    pub max_age_slots: u64,
    /// Max serialized transaction size in bytes (default: 1232)
    pub max_tx_size: usize,
    /// Kill switch: when true, reject all builds
    pub kill_switch: Arc<AtomicBool>,
    /// Token program whitelist (empty = allow all)
    pub allowed_token_programs: Vec<Pubkey>,
}

impl TradeTransactionBuilder {
    /// Build a signed transaction from a validated TradeIntent.
    /// Returns BUILT state on success, REJECTED with reason on failure.
    pub fn build(
        &self,
        intent: &TradeIntent,       // Must be in VALIDATED state
        lane: &SubmissionLane,      // Which lane this build is for
    ) -> Result<BuiltTransaction, RejectionReason>;
}
```

### 3.2 Pre-Build Checks

Every `build()` call performs the following checks in order. Any failure produces a `RejectionReason` that transitions the intent to `REJECTED`.

```
┌─────────────────────────────────────┐
│         TradeIntent (VALIDATED)     │
└──────────┬──────────────────────────┘
           │
    ┌──────▼──────┐
    │ 1. Age check │── Is intent still within expires_at?
    └──────┬──────┘     NO → REJECTED(Expired)
           │ YES
    ┌──────▼──────────┐
    │ 2. State version │── intent.state == VALIDATED?
    └──────┬──────────┘     NO → REJECTED(InvalidState)
           │ YES
    ┌──────▼──────────┐
    │ 3. Exposure     │── Has this intent already been submitted
    │    check        │    across any lane?
    └──────┬──────────┘     YES → REJECTED(Duplicate)
           │ NO
    ┌──────▼──────────┐
    │ 4. Blockhash    │── Valid cached blockhash available?
    │    availability │     NO → REJECTED(BlockhashNotFound)
    └──────┬──────────┘
           │ YES
    ┌──────▼──────────┐
    │ 5. Fees valid   │── Fee payer has enough lamports for
    │                 │    estimated fee + tip?
    └──────┬──────────┘     NO → REJECTED(InsufficientFunds)
           │ YES
    ┌──────▼──────────┐
    │ 6. Size check   │── Estimated serialized size ≤ max_tx_size?
    │ (pre-validate)  │     NO → REJECTED(TooLarge)
    └──────┬──────────┘
           │ YES
    ┌──────▼──────────────────────┐
    │ 7. Token program whitelist  │── All referenced token programs
    │                             │    are in allowed list?
    └──────┬──────────────────────┘     NO → REJECTED(DisallowedProgram)
           │ YES
    ┌──────▼──────────────────────┐
    │ 8. Kill switch              │── kill_switch.load() is false?
    └──────┬──────────────────────┘     YES → REJECTED(SystemHalted)
           │ ALL CHECKS PASSED
    ┌──────▼──────────────────────┐
    │ Build VersionedTransaction  │
    │  - Compile instructions     │
    │  - Sign with payer key      │
    │  - Serialize & final size   │
    │    check                    │
    └──────┬──────────────────────┘
           │
    ┌──────▼──────────────┐
    │ BuiltTransaction    │
    │  { tx, intent_id,  │
    │    lane, blockhash, │
    │    size, fee_est }  │
    └─────────────────────┘
```

### 3.3 Rejection Reasons

```rust
pub enum RejectionReason {
    /// Intent has expired (created_at + timeout elapsed)
    Expired {
        created_at: Instant,
        expires_at: Instant,
    },
    /// Intent is in wrong state for building
    InvalidState {
        expected: IntentState,
        actual: IntentState,
    },
    /// This intent has already been submitted or is in-flight
    Duplicate {
        first_submission: Instant,
    },
    /// No valid blockhash available in cache
    BlockhashNotFound,
    /// Fee payer cannot cover estimated fees
    InsufficientFunds {
        available: u64,
        required: u64,
    },
    /// Transaction would exceed packet size limit
    TooLarge {
        estimated_size: usize,
        max_size: usize,
    },
    /// Transaction references a disallowed token program
    DisallowedProgram {
        program_id: Pubkey,
    },
    /// System-level kill switch is active
    SystemHalted,
    /// Build failed during message compilation or signing
    BuildFailure {
        detail: String,
    },
}
```

### 3.4 Key Design Decision: No Automatic Slippage Expansion

When a build fails (e.g., `TooLarge`), the builder MUST NOT automatically retry with adjusted slippage or reduced instructions. This is deliberate:

- **Slippage is a user-defined risk parameter**, not an optimization variable
- **Automatic expansion would change trade semantics** silently
- **The correct response** is to surface the `RejectionReason` to the caller, who may choose to adjust parameters and create a new `TradeIntent`

---

## 4. Multi-Lane Submission & Deduplication

### 4.1 Architecture Overview

```
                        ┌─────────────────────┐
                        │   TradeIntent Id:   │
                        │   [32-byte hash]    │
                        │   State: SIGNED     │
                        └──────────┬──────────┘
                                   │
                   ┌───────────────┼───────────────┐
                   │               │               │
            ┌──────▼──────┐ ┌──────▼──────┐ ┌──────▼──────┐
            │ Lane 1      │ │ Lane 2      │ │ Lane N      │
            │ Jito/HTTP   │ │ Helius/HTTP │ │ ZeroSlot    │
            │             │ │             │ │ /gRPC       │
            │ tip: 0.003  │ │ tip: 0.001  │ │ tip: 0.001  │
            │ CU: 400k    │ │ CU: 400k    │ │ CU: 300k    │
            └──────┬──────┘ └──────┬──────┘ └──────┬──────┘
                   │               │               │
            ┌──────▼──────┐ ┌──────▼──────┐ ┌──────▼──────┐
            │ Build Tx A  │ │ Build Tx B  │ │ Build Tx C  │
            │ (same ix,   │ │ (same ix,   │ │ (same ix,   │
            │ same payer, │ │ same payer, │ │ same payer, │
            │ diff tip    │ │ diff tip    │ │ diff tip    │
            │ acct)       │ │ acct)       │ │ acct)       │
            └──────┬──────┘ └──────┬──────┘ └──────┬──────┘
                   │               │               │
            ┌──────▼──────┐ ┌──────▼──────┐ ┌──────▼──────┐
            │ Submit to   │ │ Submit to   │ │ Submit to   │
            │ Jito Block  │ │ Helius RPC │ │ ZeroSlot    │
            │ Engine      │ │             │ │ gRPC        │
            └──────┬──────┘ └──────┬──────┘ └──────┬──────┘
                   │               │               │
                   │         ┌─────▼─────┐         │
                   └─────────►  Result   ◄─────────┘
                             │ Collector │
                             │           │
                             │ - success │
                             │ - sigs[]  │
                             │ - errors  │
                             └─────┬─────┘
                                   │
                        ┌──────────▼──────────┐
                        │ Poll-any confirm    │
                        │ (one signature that │
                        │ landed via any lane)│
                        └─────────────────────┘
```

### 4.2 Lane = Transport Route, Not Separate Trade Intent

Each lane produces a **different signed transaction** because:
- Tip transfer instructions target different tip accounts per provider
- Compute budget instructions (CU limit/price) may differ per fee strategy
- Provider-specific headers or gRPC metadata differ

However, all lanes share:
- **Same TradeIntent ID** — this identifies the trade across all lanes
- **Same business instructions** — the core swap instructions are identical
- **Same payer keypair** — all transactions are signed by the same authority
- **Same blockhash/nonce** — all lanes use the same recent_blockhash or same durable nonce value

### 4.3 Deduplication by Signature

Each lane's transaction has a **different signature** because the transaction bytes differ (different tip account, different CU config). The signature is the hash of the signed transaction bytes.

**Deduplication strategy**:
```
Same trade-intent ID  ─────►  One intent, multiple lane transactions
                                  │
                           ┌──────┴──────┐
                           │             │
                     Sig A (Jito)   Sig B (Helius)
                           │             │
                           │  At most ONE lands on-chain
                           │  (both use same blockhash/nonce)
                           │             │
                           └──────┬──────┘
                                  │
                          ┌───────▼────────┐
                          │ Poll ANY of    │
                          │ the signatures │
                          │ (poll_any_     │
                          │ transaction_   │
                          │ confirmation)  │
                          └────────────────┘
```

When using a **recent blockhash**: Multiple signatures could theoretically all land on-chain because they share the same blockhash but have different bytes (different CU limits or tip accounts). However, the swap instructions are identical, so the second transaction to execute would hit `AlreadyProcessed` (status cache prevents re-execution of the same message hash) or fail with account lock contention.

When using a **durable nonce**: At most ONE lane's transaction can land because the validator atomically advances the nonce. All other lanes' transactions will fail with `BlockhashNotFound` at the nonce check stage.

### 4.4 Distinguishing Lane Acknowledgement from Landing

Two distinct outcomes per lane:

```
Lane Acknowledgement (submit response):
  - Jito HTTP returned 200 with result = signature
  - Helius POST returned success
  - ZeroSlot gRPC returned OK
  → This means: "The provider accepted the transaction"
  → Does NOT mean: "The transaction landed on-chain"

Landing (on-chain confirmation):
  - getSignatureStatuses returns Confirmed or Finalized
  - getTransaction returns valid meta with err = null
  → This means: "The transaction was included in a block and executed"
```

The `ResultCollector` in the current `async_executor.rs` handles this:
- `wait_for_first_submitted()` returns when the first lane acknowledges
- `wait_for_success()` returns when the first lane confirms on-chain

### 4.5 Reconciliation on Ambiguous Responses

When a lane returns an ambiguous response (timeout, disconnect, HTTP 500 with no body, gRPC stream reset), the lane's result is marked as `AMBIGUOUS` and the `TradeIntent` transitions to the `AMBIGUOUS` state if ALL lanes returned ambiguous results. See `reconciliation.md` for the full reconciliation protocol.

### 4.6 Result Collector: Current Design vs Proposed

**Current** (`async_executor.rs`):
```rust
struct TaskResult {
    success: bool,
    signature: Signature,
    error: Option<anyhow::Error>,
    swqos_type: SwqosType,
    strategy_type: GasFeeStrategyType,
    landed_on_chain: bool,
    submit_done_us: i64,
}
```

**Proposed extension**:
```rust
struct TaskResult {
    /// Was the submission accepted by the provider?
    submit_acknowledged: bool,
    /// Did the transaction land on-chain? (None = unknown)
    landed: Option<bool>,
    /// The transaction signature
    signature: Signature,
    /// The TradeIntent this task belongs to
    intent_id: [u8; 32],
    /// Provider error, if any
    error: Option<SubmissionError>,
    /// SWQOS provider metadata
    swqos_type: SwqosType,
    strategy_type: GasFeeStrategyType,
    /// Timestamps
    submit_done_us: i64,
}

pub enum SubmissionError {
    /// Provider rejected the transaction
    Rejected { code: u32, message: String },
    /// Provider accepted but never confirmed (timeout)
    TimedOut { timeout: Duration },
    /// Provider response was ambiguous (disconnect, 500, etc.)
    Ambiguous { detail: String },
    /// Transaction landed on-chain but execution failed
    ExecutionFailure { error: TransactionError },
}
```

---

## 5. Integration Points

### 5.1 Current Code Integration

| Component | Current Location | Change Required |
|-----------|-----------------|-----------------|
| Blockhash Cache | Not yet implemented | New module: `src/trading/core/blockhash_cache.rs` |
| Blockhash Cache Config | Not yet implemented | Extend `TradeConfig` in `src/common/types.rs` |
| Durable Nonce | `src/common/nonce_cache.rs` | Minor: add `insert(0, ...)` in `add_nonce_instruction` |
| Transaction Builder | `src/trading/common/transaction_builder.rs` | Add `build_from_intent()` method; keep `build_transaction()` as internal |
| Multi-Lane Submit | `src/trading/core/async_executor.rs` | Extend `TaskResult` with `intent_id` and `SubmissionError` |
| Result Collector | `src/trading/core/async_executor.rs` | Add `AMBIGUOUS` detection and `intent_id` tracking |
| Provider Interface | `src/swqos/mod.rs` (`SwqosClientTrait`) | No change needed; provider interface is sufficient |

### 5.2 Proposed Module Layout

```
src/trading/core/
  ├── mod.rs
  ├── execution.rs             (unchanged)
  ├── executor.rs              (unchanged)
  ├── async_executor.rs        (extend TaskResult, ResultCollector)
  ├── transaction_pool.rs      (unchanged)
  ├── traits.rs                (unchanged)
  ├── params/                  (unchanged)
  ├── blockhash_cache.rs       (NEW)
  ├── intent.rs                (NEW: TradeIntent, IntentState)
  └── reconciler.rs            (NEW: ambiguous submission handler)
```

---

## 6. Edge Cases & Failure Modes

| Scenario | Behavior |
|----------|----------|
| All RPCs down during blockhash refresh | Cache stays on last known good value until it expires; subsequent `get_valid_blockhash()` → `BlockhashNotFound` |
| Blockhash fetched but expired before trade builds | Builder rejects with `BlockhashNotFound` during pre-build check |
| Two lanes produce same signature | Impossible with current design (different tip accounts → different tx bytes → different signatures) |
| Durable nonce used concurrently across two processes | Second process's tx fails (`BlockhashNotFound` at nonce check); no double-spend |
| Kill switch activated mid-submit | Next `build()` call fails with `SystemHalted`; already-submitted lanes continue in-flight |
| Provider accepts tx but never confirms | `poll_any_transaction_confirmation` times out; intent → `AMBIGUOUS` for reconciliation |
| Provider returns HTTP 503/gRPC unavailable | Lane submits nothing; other lanes continue; if all lanes fail → intent → `FAILED` |