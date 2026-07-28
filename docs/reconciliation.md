# Ambiguous Submission Reconciliation

> **Design document**: Reconciliation protocol for ambiguous submission responses, timeout handling, balance verification, and rebuild decision
> **Related**: `transaction-lifecycle.md` (Submit flow), `state-model.md` (AMBIGUOUS → RECONCILING → LANDED/FAILED/ROLLED_BACK)
> **Pre-flight sections**: 29, 31, 32

---

## 1. When Reconciliation Is Needed

### 1.1 Ambiguous Response Definition

A submission response is **ambiguous** when the outcome cannot be determined from the provider's response:

| Scenario | Provider Response | Classification |
|----------|------------------|----------------|
| HTTP 200 with valid result | `{"result": "<sig>"}` | **Acknowledged** — not ambiguous |
| HTTP 200 with result, but `getSignatureStatuses` returns `null` after timeout | 200 + no on-chain status | **Ambiguous** — provider may have accepted but tx never forwarded |
| HTTP 500 with no body / connection reset | No usable response | **Ambiguous** — tx may or may not have reached the validator |
| gRPC stream disconnected mid-request | Stream error | **Ambiguous** — same as above |
| HTTP 503 / provider unavailable | Service unavailable | **Not ambiguous** — provider was unreachable; tx definitely not sent |
| Request timeout (no response within timeout window) | No response | **Ambiguous** — tx may have been sent or not |
| Provider returns error with non-zero code (e.g., `ExceededSlippage` 6004) | `{"error": {...}}` | **Not ambiguous** — tx landed but failed. `is_landed_error()` returns `true` |

### 1.2 When to Trigger Reconciliation

Reconciliation begins when the `TradeIntent` transitions to `AMBIGUOUS` — meaning ALL lanes for this intent returned ambiguous results AND the confirmation timeout has elapsed:

```
for every lane in intent.lanes:
    if lane.result == Acknowledged or lane.result == Landed:
        → No reconciliation needed
    elif lane.result == Ambiguous:
        → count += 1

if count == len(intent.lanes) AND confirmation_timed_out:
    → Transition intent.state = AMBIGUOUS
    → Start reconciliation
```

---

## 2. Reconciliation Protocol (7-Step Process)

```
┌──────────────────────────────────────┐
│  TradeIntent.state == SUBMITTED      │
│  (or ACKNOWLEDGED)                   │
│                                      │
│  Confirmation timeout elapsed        │
│  (default: 15s, see poll_any_        │
│   transaction_confirmation timeout)  │
└──────────────┬───────────────────────┘
               │
    ┌──────────▼──────────┐
    │ Step 1: Mark        │
    │ intent as AMBIGUOUS │
    │ Record timeout      │
    │ duration in history │
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │ Step 2: Query       │
    │ signature statuses  │
    │ for ALL lane sigs   │
    │ via RPC             │
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │ Step 3: Check       │
    │ block height vs     │
    │ last_valid_block_   │
    │ height              │
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │ Step 4: Check       │
    │ token / SOL         │
    │ balances (pre vs    │
    │ post)               │
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │ Step 5: If sig      │
    │ has status, fetch   │
    │ full transaction    │
    │ via getTransaction  │
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │ Step 6: If landed,  │
    │ reconstruct         │
    │ position impact     │
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │ Step 7: Decision    │
    │                     │
    │ Landed?  → LANDED   │
    │ Not landed,         │
    │ opp valid → rebuild │
    │ Not landed,         │
    │ opp stale → ROLLED  │
    │   _BACK             │
    └─────────────────────┘
```

### 2.1 Step 1 — Mark AMBIGUOUS

```rust
impl Reconciler {
    pub fn enter_ambiguous(&self, intent_id: IntentId) -> Result<()> {
        let mut intent = self.intent_manager.get_mut(&intent_id)
            .ok_or(ReconciliationError::IntentNotFound)?;

        // Record the timeout duration
        let elapsed = Instant::now().duration_since(intent.state_updated_at);

        intent.transition(
            IntentState::AMBIGUOUS,
            TransitionMetadata {
                timestamp: Instant::now(),
                intent_id,
                signature: None,
                slot: None,
                blockhash: intent.blockhash,
                fees_lamports: None,
                provider: None,
                evidence: TransitionEvidence::Timeout {
                    duration: elapsed,
                },
            },
        )?;

        Ok(())
    }
}
```

### 2.2 Step 2 — Query Signature Status

Query ALL lane signatures via `getSignatureStatuses`:

```rust
impl Reconciler {
    async fn query_signature_statuses(
        &self,
        intent: &TradeIntent,
        rpc: &SolanaRpcClient,
    ) -> ReconciliationResult {
        let signatures: Vec<Signature> = intent.lane_signatures
            .iter()
            .map(|ls| ls.signature)
            .collect();

        let statuses = rpc.get_signature_statuses(&signatures).await?;

        for (i, maybe_status) in statuses.value.iter().enumerate() {
            if let Some(status) = maybe_status {
                // Signature has an on-chain status
                let sig = &signatures[i];
                if status.err.is_none() {
                    return ReconciliationResult::Landed {
                        signature: *sig,
                        slot: status.slot,
                        confirmation_status: status.confirmation_status,
                    };
                } else {
                    return ReconciliationResult::LandedButFailed {
                        signature: *sig,
                        slot: status.slot,
                        error: status.err.clone(),
                    };
                }
            }
        }

        // No signature has any on-chain status
        ReconciliationResult::NotFound
    }
}
```

### 2.3 Step 3 — Check Block Height vs Last Valid

```rust
impl Reconciler {
    async fn check_blockhash_validity(
        &self,
        intent: &TradeIntent,
        rpc: &SolanaRpcClient,
    ) -> Result<BlockhashValidity> {
        // Get current block height
        let current_slot = rpc.get_slot().await?;

        let blockhash_entry = intent.blockhash
            .and_then(|_| self.blockhash_cache.peek_blockhash());

        match blockhash_entry {
            Some(entry) => {
                let remaining = entry.last_valid_block_height.saturating_sub(current_slot);
                Ok(BlockhashValidity {
                    current_slot,
                    last_valid: entry.last_valid_block_height,
                    remaining_slots: remaining,
                    still_valid: remaining > 0,
                })
            }
            None => {
                // No cached blockhash entry — can't determine validity
                Ok(BlockhashValidity {
                    current_slot,
                    last_valid: 0,
                    remaining_slots: 0,
                    still_valid: false,
                })
            }
        }
    }
}

struct BlockhashValidity {
    current_slot: u64,
    last_valid: u64,
    remaining_slots: u64,
    still_valid: bool,
}
```

If `still_valid == false`, the transaction's blockhash has expired:
- For recent-blockhash transactions: the transaction **cannot** land now even if submitted — the validator will reject it with `BlockhashNotFound`
- The reconciliation can conclude **not landed** (assuming no other blockhash was used)

### 2.4 Step 4 — Check Token/SOL Balances

Compare pre-trade and post-trade balances to determine if the trade executed:

```rust
impl Reconciler {
    async fn check_balances(
        &self,
        intent: &TradeIntent,
        rpc: &SolanaRpcClient,
    ) -> Result<BalanceCheckResult> {
        // Read current balances for all relevant accounts
        let input_mint = intent.mint;
        let output_mint = /* resolve from protocol params */;
        let payer = /* resolve payer */;

        let input_balance = get_token_balance(rpc, &input_mint, &payer).await?;
        let output_balance = get_token_balance(rpc, &output_mint, &payer).await?;
        let sol_balance = rpc.get_balance(&payer).await?;

        // Compare with expected pre-trade values
        // (stored in intent history at VALIDATED state)
        let pre = intent.history.iter()
            .find(|t| t.to_state == IntentState::VALIDATED)
            .and_then(|t| t.pre_balances.as_ref())
            .ok_or(ReconciliationError::NoPreBalances)?;

        let input_delta = pre.input_balance.saturating_sub(input_balance);
        let output_delta = output_balance.saturating_sub(pre.output_balance);
        let sol_delta = pre.sol_balance.saturating_sub(sol_balance);

        Ok(BalanceCheckResult {
            input_delta,
            output_delta,
            sol_delta,
            trade_executed: input_delta >= intent.input_amount * 9/10, // 90% threshold
        })
    }
}

struct BalanceCheckResult {
    /// Change in input token balance
    input_delta: u64,
    /// Change in output token balance
    output_delta: u64,
    /// Change in SOL balance (fees + tips)
    sol_delta: u64,
    /// Whether the trade likely executed (input decrease >= 90% of expected)
    trade_executed: bool,
}
```

### 2.5 Step 5 — Fetch Transaction If Available

If a signature has an on-chain status (even if errored), fetch the full transaction to examine execution details:

```rust
impl Reconciler {
    async fn fetch_transaction_if_available(
        &self,
        signature: &Signature,
        rpc: &SolanaRpcClient,
    ) -> Result<Option<TransactionDetail>> {
        // Use commitment: confirmed (not finalized — we want fastest possible response)
        let config = RpcTransactionConfig {
            encoding: Some(UiTransactionEncoding::JsonParsed),
            max_supported_transaction_version: Some(0),
            commitment: Some(CommitmentConfig::confirmed()),
        };

        match rpc.get_transaction_with_config(signature, config).await {
            Ok(tx_response) => {
                let meta = tx_response.transaction.meta
                    .ok_or(ReconciliationError::NoTransactionMeta)?;

                Ok(Some(TransactionDetail {
                    slot: tx_response.slot,
                    block_time: tx_response.block_time,
                    err: meta.err,
                    cu_consumed: meta.compute_units_consumed,
                    fees: meta.fee,
                    log_messages: meta.log_messages.into(),
                    pre_balances: meta.pre_balances,
                    post_balances: meta.post_balances,
                    pre_token_balances: meta.pre_token_balances,
                    post_token_balances: meta.post_token_balances,
                }))
            }
            Err(_) => {
                // Transaction not yet available — keep waiting or return None
                Ok(None)
            }
        }
    }
}

struct TransactionDetail {
    slot: u64,
    block_time: Option<i64>,
    err: Option<TransactionError>,
    cu_consumed: Option<u64>,
    fees: u64,
    log_messages: Vec<String>,
    pre_balances: Vec<u64>,
    post_balances: Vec<u64>,
    pre_token_balances: Vec<TransactionTokenBalance>,
    post_token_balances: Vec<TransactionTokenBalance>,
}
```

### 2.6 Step 6 — Reconstruct Position Impact

If the transaction landed (even with an execution error), reconstruct the actual position impact:

```rust
impl Reconciler {
    fn reconstruct_position_impact(
        &self,
        intent: &TradeIntent,
        tx_detail: &TransactionDetail,
    ) -> PositionImpact {
        let pre_token = tx_detail.pre_token_balances.as_slice();
        let post_token = tx_detail.post_token_balances.as_slice();

        // Find the intent's input/output token in pre/post balances
        let input_amount = find_delta(
            &intent.mint,
            intent.input_token_program,
            pre_token,
            post_token,
        );

        let output_mint = determine_output_mint(&intent.protocol_params);
        let output_amount = find_delta(
            &output_mint,
            intent.output_token_program,
            pre_token,
            post_token,
        );

        let sol_fees = tx_detail.pre_balances[0]
            .saturating_sub(tx_detail.post_balances[0]);

        PositionImpact {
            input_token_delta: input_amount,
            output_token_delta: output_amount,
            fees_lamports: tx_detail.fees,
            total_sol_delta: sol_fees,
            cu_consumed: tx_detail.cu_consumed,
            execution_error: tx_detail.err.clone(),
        }
    }
}

struct PositionImpact {
    input_token_delta: u64,
    output_token_delta: u64,
    fees_lamports: u64,
    total_sol_delta: u64,
    cu_consumed: Option<u64>,
    execution_error: Option<TransactionError>,
}
```

### 2.7 Step 7 — Decision

```rust
/// The final decision from reconciliation
pub enum ReconciliationDecision {
    /// Transaction landed successfully → transition to LANDED
    Landed {
        signature: Signature,
        slot: u64,
        impact: PositionImpact,
    },
    /// Transaction landed but execution failed → transition to FAILED
    Failed {
        signature: Signature,
        slot: u64,
        error: TransactionError,
        impact: PositionImpact,
    },
    /// Transaction did NOT land, and the original opportunity is still valid
    /// → rebuild and re-submit (create new intent)
    Rebuild {
        reason: String,
    },
    /// Transaction did NOT land, and the original opportunity is no longer valid
    /// → transition to ROLLED_BACK
    RolledBack {
        reason: String,
    },
}

impl Reconciler {
    fn decide(
        &self,
        signature_result: ReconciliationResult,
        blockhash_validity: BlockhashValidity,
        balance_result: BalanceCheckResult,
        tx_detail: Option<TransactionDetail>,
        intent: &TradeIntent,
    ) -> ReconciliationDecision {
        // Decision tree
        match signature_result {
            ReconciliationResult::Landed { signature, slot, .. } => {
                // Transaction definitely landed
                let impact = tx_detail
                    .map(|d| self.reconstruct_position_impact(intent, &d))
                    .unwrap_or_default();
                ReconciliationDecision::Landed { signature, slot, impact }
            }
            ReconciliationResult::LandedButFailed { signature, slot, error } => {
                // Transaction landed but execution failed (slippage, etc.)
                let impact = tx_detail
                    .map(|d| self.reconstruct_position_impact(intent, &d))
                    .unwrap_or_default();
                ReconciliationDecision::Failed {
                    signature,
                    slot,
                    error,
                    impact,
                }
            }
            ReconciliationResult::NotFound => {
                // No on-chain status for any signature
                if balance_result.trade_executed {
                    // Balances changed — trade likely landed but status expired
                    // (past MAX_RECENT_BLOCKHASHES window)
                    ReconciliationDecision::Landed {
                        signature: intent.lane_signatures[0].signature,
                        slot: 0, // unknown
                        impact: PositionImpact {
                            input_token_delta: balance_result.input_delta,
                            output_token_delta: balance_result.output_delta,
                            fees_lamports: balance_result.sol_delta,
                            ..Default::default()
                        },
                    }
                } else if blockhash_validity.still_valid && self.opportunity_still_valid(intent) {
                    // Blockhash still valid, opportunity not stale → rebuild
                    ReconciliationDecision::Rebuild {
                        reason: format!(
                            "No on-chain status, blockhash valid for {} more slots",
                            blockhash_validity.remaining_slots
                        ),
                    }
                } else {
                    // Opportunity window closed
                    ReconciliationDecision::RolledBack {
                        reason: if !blockhash_validity.still_valid {
                            "Blockhash expired".into()
                        } else {
                            "Opportunity stale".into()
                        },
                    }
                }
            }
        }
    }

    /// Check if the original trade opportunity is still viable
    fn opportunity_still_valid(&self, intent: &TradeIntent) -> bool {
        // Check price hasn't moved beyond slippage tolerance
        // Check pool reserves haven't changed substantially
        // Check current time vs created_at + max_intent_duration
        // Check kill switch not active
        Instant::now() < intent.expires_at
            // Additional protocol-specific checks...
    }
}
```

---

## 3. Rebuild Logic

If reconciliation decides `Rebuild`, a **new** `TradeIntent` is created with fresh parameters:

```rust
impl Reconciler {
    pub async fn rebuild(&self, original: &TradeIntent) -> Result<TradeIntent> {
        // 1. Create a fresh TradeIntent (the rebuild is a NEW intent, not a retry)
        let fresh_intent = TradeIntent {
            id: derive_intent_id(/* new parameters */),
            created_at: Instant::now(),
            expires_at: Instant::now() + REBUILD_TTL,
            state: IntentState::DETECTED,
            // Clone immutable trade parameters
            protocol: original.protocol,
            direction: original.direction,
            mint: original.mint,
            input_amount: original.input_amount,
            slippage_basis_points: original.slippage_basis_points,
            protocol_params: original.protocol_params.clone(),
            // ... rest
        };

        // 2. Record in intent manager
        self.intent_manager.register(fresh_intent.clone())?;

        // 3. Return for the caller to process through the normal pipeline
        Ok(fresh_intent)
    }
}

/// TTL for rebuilt intents (shorter than original — urgency decay)
const REBUILD_TTL: Duration = Duration::from_millis(500);
```

**Important**: A rebuild creates an entirely new intent with a new ID. It is NOT a retry of the original intent. The original intent remains in `ROLLED_BACK` (if not landed) or `LANDED`/`FAILED`.

---

## 4. Timeout Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `submit_timeout` | 5s | Max wall-clock time to wait for all lane submit responses (`FAST_SUBMIT_RESULT_TIMEOUT`) |
| `confirmation_timeout` | 15s | Max time to wait for on-chain confirmation (`poll_any_transaction_confirmation` timeout) |
| `reconciliation_interval` | 100ms | Poll interval during reconciliation steps |
| `reconciliation_max_duration` | 30s | Max total wall-clock time for reconciliation |
| `pending_balance_check_delay` | 500ms | Delay before balance check (allows RPC to catch up) |

---

## 5. Existing Code Integration

### 5.1 Current Poll Logic

The existing `swqos/common.rs::poll_any_transaction_confirmation` function already has the foundation:

- Polls signatures via `getSignatureStatuses`
- Checks for `Confirmed` / `Finalized` status
- Falls back to `getTransaction` after 10 polls for detailed error extraction
- Timeout after 15 seconds

**Proposed extension**: Instead of returning only the successful signature, the poll function should also detect the ambiguous case and return structured information:

```rust
pub enum ConfirmationOutcome {
    /// At least one signature confirmed
    Confirmed {
        signature: Signature,
        slot: u64,
        status: TransactionConfirmationStatus,
    },
    /// Signature has status but execution failed
    LandedWithError {
        signature: Signature,
        error: TransactionError,
        slot: u64,
    },
    /// No signature has any status — transaction never reached chain
    NotFound,
    /// Some signatures have status but none confirmed (mixed results)
    Partial {
        statuses: Vec<(Signature, Option<SignatureStatus>)>,
    },
    /// Timed out before any signature got a non-null status
    TimedOut,
}
```

### 5.2 Current Error Classification

The existing `is_landed_error` function in `async_executor.rs` already classifies errors:

```rust
fn is_landed_error(error: &anyhow::Error) -> bool {
    // TradeError with code 500 + "timed out" → not landed
    // Any other TradeError → landed (e.g., ExceededSlippage = 6004)
    // Error message contains "timed out" → not landed
    // Default: false (conservative)
}
```

**Extension needed**: Add more granular classification:

```rust
pub enum SubmissionDisposition {
    /// Provider acknowledged submission
    Acknowledged,
    /// Transaction landed on-chain (confirmed or finalized)
    Landed { slot: u64 },
    /// Transaction landed on-chain but execution failed
    FailedOnChain {
        error: TransactionError,
        slot: u64,
    },
    /// Provider unreachable — definitely not submitted
    ProviderUnavailable { reason: String },
    /// Ambiguous — unknown whether tx reached the network
    Ambiguous { detail: String },
}
```

---

## 6. Edge Cases

| Scenario | Reconciliation Behavior |
|----------|------------------------|
| Provider returns 200 but `getSignatureStatuses` returns `null` for all signatures | Check balances. If unchanged and blockhash expired → `ROLLED_BACK`. If unchanged and blockhash still valid → `Rebuild`. |
| Provider returns 200 but tx confirmed 5 minutes later (after reconciliation finished) | Late confirmation: if intent already `ROLLED_BACK`, ignore (the new intent will handle it; nonce protects double-spend). If intent already `FAILED`, update to `LANDED` with a note. |
| `getTransaction` returns meta with `err = null` but balances are incorrect | Transaction executed partially or fee-only (nonce advance consumed fee but swap failed silently). `LANDED` with note. |
| RPC node is behind / stale | Use dedicated `commitment: confirmed` for reconciliation RPC calls. If RPC is unreachable, delay reconciliation and retry. |
| All signatures have status but none confirmed (e.g., nonce-advance failed on all lanes) | `PARTIAL` — some lanes' nonce checks failed. Original trade never executed. Rebuild if opportunity valid. |
| Balance check shows input deducted but no output received (rug/honeypot) | `FAILED` — trade executed but tokens lost. Record as loss for position tracking. |
| Durable nonce transaction: nonce consumed but swap instructions failed | Nonce IS advanced even on execution failure (validator advances nonce before executing instructions). Trade failed, but nonce is consumed. Rebuild needs fresh `fetch_nonce_info()`. |
| Reconciliation times out (30s limit exceeded) | Force-terminate to `FAILED` with reason "reconciliation timed out". The intent stays `FAILED` — no automatic rebuild. |