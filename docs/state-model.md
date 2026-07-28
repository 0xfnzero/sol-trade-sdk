# TradeIntent State Machine

> **Design document**: TradeIntent struct, State Machine (all states and transitions), Transition Recording
> **Related**: `transaction-lifecycle.md` (Blockhash/Build/Submit), `reconciliation.md` (Ambiguous handling)
> **Pre-flight sections**: 25, 30, 32

---

## 1. TradeIntent Struct

The `TradeIntent` is the core domain object representing a single trading operation throughout its lifecycle — from detection on the gRPC stream through settlement confirmation.

### 1.1 Struct Definition

```rust
/// Unique identifier for a trade intent — 32 bytes derived from the source event
pub type IntentId = [u8; 32];

/// Protocol/DEX that will execute the trade
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    PumpFun,
    PumpSwap,
    RaydiumAmmV4,
    RaydiumCpmm,
    MeteoraDammV2,
    Bonk,
    Jupiter,
}

/// Trade direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeDirection {
    Buy,
    Sell,
    CreateAndBuy,
}

/// The complete trade intent
#[derive(Debug, Clone)]
pub struct TradeIntent {
    // ── Identity ──
    /// Unique intent ID (SHA-256 hash of source_slot + source_signature + mint)
    pub id: IntentId,

    /// Which protocol executes this trade
    pub protocol: Protocol,

    /// Trade direction
    pub direction: TradeDirection,

    // ── Source ──
    /// Slot where the opportunity was detected (gRPC event slot)
    pub source_slot: u64,

    /// Signature of the source transaction that triggered the detection
    pub source_signature: Signature,

    /// Mint of the token being traded
    pub mint: Pubkey,

    /// Timestamps
    pub created_at: Instant,
    pub expires_at: Instant,

    // ── State ──
    /// Current state in the lifecycle
    pub state: IntentState,

    /// Wall-clock timestamp of last state transition
    pub state_updated_at: Instant,

    // ── Trade Parameters ──
    /// Input amount (in lamports or token decimals)
    pub input_amount: u64,

    /// Slippage tolerance in basis points
    pub slippage_basis_points: u64,

    /// Token program for input
    pub input_token_program: Option<Pubkey>,

    /// Token program for output
    pub output_token_program: Option<Pubkey>,

    // ── Account Configuration ──
    /// Whether to create input mint ATA
    pub create_input_ata: bool,
    /// Whether to close input mint ATA after trade
    pub close_input_ata: bool,
    /// Whether to create output mint ATA
    pub create_output_ata: bool,
    /// Whether to close output mint ATA
    pub close_output_ata: bool,
    /// Fixed output amount for exact-out semantics
    pub fixed_output_amount: Option<u64>,
    /// Whether to use exact SOL amount for buy
    pub use_exact_sol_amount: Option<bool>,

    // ── Protocol-specific parameters ──
    pub protocol_params: DexParamEnum,

    // ── Transaction State (populated as the intent progresses) ──
    // --- Built / Signed ---
    /// Blockhash used for all lane transactions in this intent
    pub blockhash: Option<Hash>,
    /// Blockhash source information
    pub blockhash_source: Option<RpcSource>,

    /// Signatures from each lane submission
    pub lane_signatures: Vec<LaneSignature>,

    // --- Submitted / Landing ---
    /// Which lanes were used for submission
    pub lanes: Vec<SubmissionLane>,

    // --- Settlement ---
    /// Final landing slot (from getTransaction response)
    pub landing_slot: Option<u64>,
    /// Final landing signature (the one that confirmed)
    pub landing_signature: Option<Signature>,
    /// Compute units consumed (from tx meta)
    pub cu_consumed: Option<u64>,
    /// Total fees paid (base fee + priority fee + tip, in lamports)
    pub total_fees_lamports: Option<u64>,

    /// Full transition history (ordered)
    pub history: Vec<StateTransition>,
}

/// A signature produced by one lane
#[derive(Debug, Clone)]
pub struct LaneSignature {
    pub lane: SubmissionLane,
    pub signature: Signature,
    pub submitted_at: Instant,
    pub acknowledged: bool,
    pub landed: Option<bool>,
}

/// Identifies a submission lane
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubmissionLane {
    pub swqos_type: SwqosType,
    pub strategy_type: GasFeeStrategyType,
    pub transport: &'static str,  // "http", "grpc", "quic"
}
```

### 1.2 Intent ID Derivation

The `IntentId` MUST be deterministically derived from the source event to ensure idempotency:

```rust
fn derive_intent_id(
    source_slot: u64,
    source_signature: &Signature,
    mint: &Pubkey,
    protocol: Protocol,
) -> IntentId {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(source_slot.to_le_bytes());
    hasher.update(source_signature.as_ref());
    hasher.update(mint.as_ref());
    hasher.update(protocol.to_string().as_bytes());
    let result = hasher.finalize();
    result.into()
}
```

This ensures:
- The same event always produces the same intent ID
- Duplicate detection is possible by checking `id` against an in-memory dedup set
- Crash recovery can scan for pending intents by reconstructing IDs from recent events

---

## 2. State Machine

### 2.1 States

```
                                  ┌────────────────────────────┐
                                  │        DETECTED            │
                                  │  (initial state on event)  │
                                  └────────────┬───────────────┘
                                               │
                                               │ validate()
                                               │
                                  ┌────────────▼───────────────┐
                    ┌─────────────│        VALIDATED           │
                    │             │  (opportunity is real)     │
                    │             └────────────┬───────────────┘
                    │                          │
                    │                          │ build()
                    │                          │
                    │             ┌────────────▼───────────────┐
                    │             │          BUILT             │
                    │             │  (transaction constructed) │
                    │             └────────────┬───────────────┘
                    │                          │
                    │                          │ sign()
                    │                          │
                    │             ┌────────────▼───────────────┐
                    │             │          SIGNED            │
                    │             │  (transaction signed)      │
                    │             └────────────┬───────────────┘
                    │                          │
                    │                          │ submit()
                    │                          │
                    │             ┌────────────▼───────────────┐
                    │             │        SUBMITTED           │
                    │             │  (sent to provider)        │
                    │             └────────────┬───────────────┘
                    │                          │
                    │                          ├──────────────────────┐
                    │                          │                      │
                    │             ┌────────────▼────────┐   ┌────────▼────────┐
                    │             │       LANDED        │   │   AMBIGUOUS    │
                    │             │  (on-chain confirm) │   │  (timeout/disc)│
                    │             └────────────┬────────┘   └────────┬────────┘
                    │                          │                     │
                    │                          │                     │ reconcile()
                    │                          │                     │
                    │                          │           ┌────────▼────────┐
                    │                          │           │   RECONCILING  │
                    │                          │           └────────┬────────┘
                    │                          │                     │
                    │                          │           ┌────────┼────────┐
                    │                          │           │        │        │
                    │                          │     ┌─────▼───┐ ┌──▼───┐ ┌──▼────┐
                    │                          │     │ RETURNED│ │ FAILED││LANDED │
                    │                          │     │TO LAN-  │ │       ││ (re-  │
                    │                          │     │DED/LAND │ │       ││cover) │
                    │                          │     │ED/SETTL │ │       │└───────┘
                    │                          │     │ED       │ │       │
                    │                          │     └─────────┘ └───────┘
                    │                          │
                    │             ┌────────────▼────────┐
                    │             │       SETTLED       │
                    │             │ (position verified) │
                    │             └─────────────────────┘
                    │
                    │  TERMINAL STATES:
                    │  ┌───────────┐  ┌────────┐  ┌──────────┐  ┌───────────┐
                    │  │ REJECTED │  │EXPIRED │  │  FAILED  │  │ CANCELLED │
                    │  │(pre-sub-)│  │(pre-   │  │(post-    │  │(user-     │
                    │  │          │  │submit) │  │submit)   │  │initiated) │
                    │  └───────────┘  └────────┘  └──────────┘  └───────────┘
                    │
                    └── Optional states ──────────────────────────────┘
                       ACKNOWLEDGED (lane accepted, not yet landed)
                       AMBIGUOUS    (unknown on-chain outcome)
                       RECONCILING  (in reconciliation process)
                       ROLLED_BACK  (position reverted)

```

### 2.2 Required States (Core Flow)

| State | Description | Entry Condition | Exit Action |
|-------|-------------|-----------------|-------------|
| `DETECTED` | Opportunity identified from gRPC event stream. Intent created with source event data. | Source event received (gRPC `pop()`), dedup check passed | `validate()` |
| `VALIDATED` | Opportunity validated: parameters are sane, payer has balance, opportunity hasn't passed. | Validation checks passed | `build()` |
| `BUILT` | Transaction assembled: business instructions compiled, compute budget added, tip instruction added, nonce advance added (if applicable). | `build()` succeeded, blockhash assigned | `sign()` |
| `SIGNED` | Transaction signed with payer keypair. Ready for submission. | `sign()` completed | `submit()` |
| `SUBMITTED` | Transaction sent to at least one SWQOS provider. May still be in-flight. | At least one lane returned acknowledgement | `confirm()` or `timeout()` |
| `LANDED` | Transaction confirmed on-chain (Confirmed or Finalized). | `getSignatureStatuses` returns non-null with `err = None` | `settle()` |
| `SETTLED` | Post-landing verification complete: position updated, token balances confirmed. | Balance check confirms expected state | Terminal |

### 2.3 Terminal States

| State | Description | Entry Condition |
|-------|-------------|-----------------|
| `REJECTED` | Pre-submission failure: pre-build check failed, validation failed, blockhash unavailable. | Any pre-build check failure. `RejectionReason` recorded. |
| `EXPIRED` | Intent timed out before reaching a terminal state. | `expires_at` elapsed before `LANDED` or `SETTLED`. |
| `FAILED` | Post-submission failure: transaction landed but execution failed (slippage, etc.), or all lanes failed with non-ambiguous errors. | All lanes returned definitive errors or on-chain execution failed. |
| `CANCELLED` | User-initiated cancellation. | User called `cancel()` before `SUBMITTED`. |

### 2.4 Optional States

| State | Description | Entry Condition |
|-------|-------------|-----------------|
| `ACKNOWLEDGED` | At least one lane accepted the transaction (submit response successful). Separates "provider accepted" from "on-chain landed". | `wait_for_first_submitted()` returned success. |
| `AMBIGUOUS` | All lanes returned ambiguous responses (timeout, disconnect, HTTP 5xx with no body). On-chain outcome unknown. Need reconciliation. | All `TaskResult` entries are ambiguous after timeout window. |
| `RECONCILING` | Reconciliation in progress: querying signature statuses, checking balances, fetching transactions. | `AMBIGUOUS` → reconciliation process started. |
| `ROLLED_BACK` | Reconciliation determined the transaction never landed but opportunity is lost or position was already consumed by another trade. Intent will not be rebuilt. | Reconciliation confirmed no landing; opportunity stale. |

### 2.5 Valid State Transition Matrix

```
                   TO:  DET  VAL  BLD  SIG  SUB  ACK  LND  STL  AMB  REC  REJ  EXP  FAL  CAN  ROL
FROM:             ──────────────────────────────────────────────────────────────────────────────
DETECTED               -    ✓    ✗    ✗    ✗    ✗    ✗    ✗    ✗    ✗    ✓    ✓    ✗    ✓    ✗
VALIDATED              ✗    -    ✓    ✗    ✗    ✗    ✗    ✗    ✗    ✗    ✓    ✓    ✗    ✓    ✗
BUILT                  ✗    ✗    -    ✓    ✗    ✗    ✗    ✗    ✗    ✗    ✓    ✓    ✗    ✓    ✗
SIGNED                 ✗    ✗    ✗    -    ✓    ✗    ✗    ✗    ✗    ✗    ✗    ✓    ✗    ✓    ✗
SUBMITTED              ✗    ✗    ✗    ✗    -    ✓    ✓    ✗    ✓    ✗    ✗    ✓    ✓    ✓    ✗
ACKNOWLEDGED           ✗    ✗    ✗    ✗    ✗    -    ✓    ✗    ✓    ✗    ✗    ✓    ✓    ✗    ✗
LANDED                 ✗    ✗    ✗    ✗    ✗    ✗    -    ✓    ✗    ✗    ✗    ✗    ✗    ✗    ✗
SETTLED                ✗    ✗    ✗    ✗    ✗    ✗    ✗    -    ✗    ✗    ✗    ✗    ✗    ✗    ✗
AMBIGUOUS              ✗    ✗    ✗    ✗    ✗    ✗    ✓    ✗    ✗    ✓    ✗    ✓    ✓    ✗    ✓
RECONCILING            ✗    ✗    ✗    ✗    ✗    ✗    ✓    ✓    ✗    -    ✗    ✓    ✓    ✗    ✓

Key: ✓ = allowed, ✗ = forbidden, - = self (no change)
```

### 2.6 State Transition Logic

```rust
impl TradeIntent {
    /// Attempt to transition the intent to a new state.
    /// Records the transition in `history` on success.
    pub fn transition(
        &mut self,
        new_state: IntentState,
        metadata: TransitionMetadata,
    ) -> Result<(), StateError>;

    /// Check if a transition is valid per the matrix
    fn is_valid_transition(from: IntentState, to: IntentState) -> bool;
}

/// Metadata recorded with every state transition
#[derive(Debug, Clone)]
pub struct TransitionMetadata {
    pub timestamp: Instant,
    pub intent_id: IntentId,
    pub signature: Option<Signature>,     // present for SUBMITTED, LANDED
    pub slot: Option<u64>,                 // present for DETECTED, LANDED
    pub blockhash: Option<Hash>,           // present for BUILT
    pub fees_lamports: Option<u64>,        // present for LANDED
    pub provider: Option<SwqosType>,       // present for SUBMITTED, ACKNOWLEDGED
    pub evidence: TransitionEvidence,      // what proves this transition
}

/// What evidence supports this state transition
#[derive(Debug, Clone)]
pub enum TransitionEvidence {
    /// Event-based: detected from gRPC stream
    GrpcEvent { source_slot: u64, source_signature: Signature },
    /// Validation: checks passed
    ValidationPass,
    /// Build: transaction was constructed
    BuildSuccess { tx_size: usize },
    /// Signature: transaction was signed
    Signature,
    /// Submit: provider acknowledged submission
    ProviderAck { provider: SwqosType, response: String },
    /// On-chain: getSignatureStatuses confirmed
    SignatureStatus {
        confirmation_status: TransactionConfirmationStatus,
        slot: u64,
    },
    /// On-chain: getTransaction returned meta
    TransactionMeta {
        slot: u64,
        cu_consumed: u64,
        fees: u64,
        err: Option<TransactionError>,
    },
    /// Balance check: token balances confirmed
    BalanceCheck {
        pre_balances: Vec<(Pubkey, u64)>,
        post_balances: Vec<(Pubkey, u64)>,
    },
    /// User-initiated
    UserAction { reason: String },
    /// Timeout
    Timeout { duration: Duration },
    /// Reconciliation
    Reconciliation { details: String },
}

#[derive(Debug, Clone)]
pub enum StateError {
    InvalidTransition {
        from: IntentState,
        to: IntentState,
    },
    AlreadyTerminal {
        current: IntentState,
    },
}
```

---

## 3. Position Accounting (Post-Settlement)

### 3.1 Settlement Verification

After a transaction lands (`LANDED` state), settlement verification confirms the expected position changes:

```rust
impl TradeIntent {
    /// Verify that the on-chain outcome matches the expected trade parameters.
    /// Transitions to SETTLED or FAILED depending on outcome.
    pub async fn verify_settlement(
        &mut self,
        rpc: &SolanaRpcClient,
    ) -> Result<(), SettlementError>;
}

pub enum SettlementError {
    /// Expected output amount does not match actual
    OutputMismatch { expected: u64, actual: u64, slot: u64 },
    /// Balance could not be read
    BalanceReadFailed { account: Pubkey, error: String },
    /// Transaction meta unavailable
    MetaUnavailable { signature: Signature },
}
```

### 3.2 Position Impact Reconstruction

For each landed trade, reconstruct:
1. **Input token balance change** — confirm input deducted as expected
2. **Output token balance change** — confirm output received within slippage bounds
3. **SOL balance change** — account for fees, tips, and rent reclaims
4. **Net position delta** — (output_value - input_value - fees) / input_value

### 3.3 Rolled-Back State Semantics

`ROLLED_BACK` is used when:
- The transaction was submitted, acknowledged by a provider, but reconciliation determined it never landed (all `getSignatureStatuses` returned `null`)
- The opportunity window has passed (slot > `last_valid_block_height`)
- Token balances returned to their pre-trade state
- The intent is NOT rebuilt — a new `TradeIntent` would be created if a fresh opportunity arises

---

## 4. Lifecycle Management API

```rust
/// Core intent manager — owns all active intents and manages the state machine
pub struct IntentManager {
    /// Active intents: those not yet in a terminal state
    active: HashMap<IntentId, TradeIntent>,

    /// Completed intents (terminal states) — bounded LRU cache for recent history
    completed: LruCache<IntentId, TradeIntent>,

    /// Deduplication set: IDs seen in the last N slots
    dedup: LruCache<IntentId, (u64, IntentState)>,
}

impl IntentManager {
    /// Register a new intent from a gRPC event → DETECTED
    pub fn register(&mut self, intent: TradeIntent) -> Result<(), DedupError>;

    /// Check if an intent ID was already seen (for dedup during gRPC event processing)
    pub fn is_known(&self, id: &IntentId) -> bool;

    /// Transition an intent to a new state
    pub fn transition(
        &mut self,
        id: &IntentId,
        to: IntentState,
        metadata: TransitionMetadata,
    ) -> Result<(), StateError>;

    /// Get an intent by ID
    pub fn get(&self, id: &IntentId) -> Option<&TradeIntent>;

    /// Get a mutable reference for reconciliation
    pub fn get_mut(&mut self, id: &IntentId) -> Option<&mut TradeIntent>;

    /// Prune expired intents (called periodically by background task)
    pub fn prune_expired(&mut self);

    /// Get all active intents in a given state
    pub fn active_in_state(&self, state: IntentState) -> Vec<&TradeIntent>;

    /// Metrics
    pub fn metrics(&self) -> IntentManagerMetrics;
}

pub struct IntentManagerMetrics {
    pub active_count: usize,
    pub completed_count: usize,
    pub dedup_hits: u64,
    pub expired_count: u64,
}

pub enum DedupError {
    AlreadyExists {
        id: IntentId,
        state: IntentState,
        registered_at: Instant,
    },
}
```

---

## 5. Edge Cases

| Scenario | Behavior |
|----------|----------|
| Same event received twice from gRPC stream | Dedup check rejects second registration (`AlreadyExists`) |
| Intent expires while building | Pre-build `age_check` → `REJECTED(Expired)` |
| Intent expires while in-flight (submitted to provider) | Timeout handler → `AMBIGUOUS` if no confirmation received, else remains in-flight until timeout → `AMBIGUOUS` → reconciliation |
| Transaction lands but execution fails (e.g., slippage) | `LANDED` → balance check reveals incorrect output → `FAILED` |
| Provider acknowledges but transaction never lands | `ACKNOWLEDGED` → timeout → `AMBIGUOUS` → `RECONCILING` → `FAILED` |
| User cancels before build completes | `CANCELLED` — terminal, no further action |
| Restart recovery | IntentManager empty on restart; pending intents are lost (nonce protects against double-spend for nonce transactions; blockhash expiry protects recent-blockhash transactions) |
| Multiple lanes land (rare with blockhash, impossible with nonce) | First `LANDED` state transition locks the intent; subsequent `LANDED` attempts for the same intent are ignored (already terminal) |