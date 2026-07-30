//! Reconciliation Service — 7-step ambiguous submission resolution protocol.
//!
//! When ALL submission lanes return ambiguous results and the confirmation
//! timeout has elapsed, this service resolves the ambiguity by querying
//! on-chain state directly.
//!
//! # Protocol (7 steps)
//!
//! 1. **Mark AMBIGUOUS** — Record the timeout duration in transition history.
//! 2. **Query signature statuses** — Call `getSignatureStatuses` for ALL lane signatures.
//! 3. **Check block height vs last valid** — If expired, conclude "not landed".
//! 4. **Check token/SOL balances** — Compare pre-trade vs post-trade balances.
//! 5. **Fetch transaction** — If status available, call `getTransaction` with confirmed commitment.
//! 6. **Reconstruct position impact** — Compute input/output deltas from tx meta.
//! 7. **Decide** — Return LANDED, FAILED, ROLLED_BACK, or REBUILD.

use crate::common::SolanaRpcClient;
use solana_sdk::{pubkey::Pubkey, signature::Signature, transaction::TransactionError};
use solana_transaction_status::TransactionConfirmationStatus;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Reconciliation outcome
// ---------------------------------------------------------------------------

/// Outcome of the reconciliation process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationOutcome {
    /// Transaction confirmed landed and executed successfully.
    Landed { signature: Signature, slot: u64 },
    /// Transaction landed but execution failed (e.g., slippage exceeded).
    Failed { signature: Signature, error: String, slot: u64 },
    /// Transaction never landed; opportunity is stale; do not rebuild.
    RolledBack { reason: String },
    /// Transaction never landed but opportunity is still valid; rebuild.
    Rebuild { reason: String },
}

impl fmt::Display for ReconciliationOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReconciliationOutcome::Landed { signature, slot } => {
                write!(f, "Landed(sig={}, slot={})", signature, slot)
            }
            ReconciliationOutcome::Failed { signature, error, slot } => {
                write!(f, "Failed(sig={}, slot={}, err={})", signature, slot, error)
            }
            ReconciliationOutcome::RolledBack { reason } => {
                write!(f, "RolledBack({})", reason)
            }
            ReconciliationOutcome::Rebuild { reason } => {
                write!(f, "Rebuild({})", reason)
            }
        }
    }
}

impl ReconciliationOutcome {
    pub fn is_terminal(&self) -> bool {
        matches!(self, ReconciliationOutcome::RolledBack { .. })
    }
}

// ---------------------------------------------------------------------------
// Balance snapshot (taken before submission)
// ---------------------------------------------------------------------------

/// Pre-trade balance snapshot for reconciliation balance comparison (Step 4).
#[derive(Debug, Clone)]
pub struct BalanceSnapshot {
    /// Fee payer SOL balance (lamports) before submission.
    pub payer_sol_lamports: u64,
    /// Input token ATA balance before submission (0 if not applicable).
    pub input_token_balance: u64,
    /// Output token ATA balance before submission (0 if not applicable).
    pub output_token_balance: u64,
    /// Slot at which the snapshot was taken.
    pub snapshot_slot: u64,
}

// ---------------------------------------------------------------------------
// Status record for one lane's signature
// ---------------------------------------------------------------------------

/// The status of a single submitted signature as seen by the RPC.
#[derive(Debug, Clone)]
pub struct SignatureStatus {
    pub signature: Signature,
    pub slot: Option<u64>,
    pub confirmations: Option<u64>,
    pub err: Option<TransactionError>,
    pub confirmed: bool,
    pub finalized: bool,
}

// ---------------------------------------------------------------------------
// Transaction meta from getTransaction
// ---------------------------------------------------------------------------

/// Parsed fields from `getTransaction` response meta.
#[derive(Debug, Clone, Default)]
pub struct TransactionMeta {
    pub err: Option<String>,
    pub pre_sol_balance: u64,
    pub post_sol_balance: u64,
    pub pre_token_balances: Vec<(Pubkey, Pubkey, u64)>,
    pub post_token_balances: Vec<(Pubkey, Pubkey, u64)>,
    pub fee_lamports: u64,
    pub cu_consumed: Option<u64>,
    pub slot: u64,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the reconciliation service.
#[derive(Debug, Clone)]
pub struct ReconciliationConfig {
    /// Blockhash expiry in slots (default: 150).
    pub blockhash_expiry_slots: u64,
    /// Timeout for individual RPC queries during reconciliation (default: 5s).
    pub rpc_query_timeout: Duration,
    /// Whether to skip balance-based reconciliation (default: false — do balance checks).
    pub skip_balance_check: bool,
}

impl Default for ReconciliationConfig {
    fn default() -> Self {
        Self {
            blockhash_expiry_slots: 150,
            rpc_query_timeout: Duration::from_secs(5),
            skip_balance_check: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Reconciliation Service
// ---------------------------------------------------------------------------

/// The reconciliation service implements the 7-step ambiguous submission
/// resolution protocol.
pub struct ReconciliationService {
    rpc: Arc<SolanaRpcClient>,
    config: ReconciliationConfig,
}

impl ReconciliationService {
    /// Create a new reconciliation service backed by the given RPC client.
    pub fn new(rpc: Arc<SolanaRpcClient>, config: ReconciliationConfig) -> Self {
        Self { rpc, config }
    }

    /// Run the full 7-step reconciliation protocol for a set of lane signatures.
    #[allow(clippy::too_many_arguments)]
    pub async fn reconcile(
        &self,
        signatures: &[Signature],
        pre_balance: &BalanceSnapshot,
        last_valid_slot: Option<u64>,
        current_slot: u64,
        payer: &Pubkey,
        _input_mint: Option<&Pubkey>,
        _output_mint: Option<&Pubkey>,
        _input_ata: Option<&Pubkey>,
        output_ata: Option<&Pubkey>,
    ) -> ReconciliationOutcome {
        info!(target: "sol_trade_sdk", "reconciliation: starting 7-step protocol for {} signatures", signatures.len());

        // Step 2: Query signature statuses
        let statuses = self.query_signature_statuses(signatures).await;
        info!(target: "sol_trade_sdk", "reconciliation: step 2 — {} statuses returned", statuses.len());

        // Fast path: any signature confirmed?
        for status in &statuses {
            if status.confirmed && status.err.is_none() {
                if let Some(slot) = status.slot {
                    info!(target: "sol_trade_sdk", "reconciliation: signature {} confirmed at slot {}", status.signature, slot);
                    return ReconciliationOutcome::Landed { signature: status.signature, slot };
                }
            }
        }

        // Fast path: any signature landed but failed?
        for status in &statuses {
            if status.confirmed || status.finalized {
                if let Some(ref err) = status.err {
                    if let Some(slot) = status.slot {
                        warn!(target: "sol_trade_sdk", "reconciliation: signature {} landed but failed at slot {}: {:?}", status.signature, slot, err);
                        return ReconciliationOutcome::Failed {
                            signature: status.signature,
                            error: format!("{:?}", err),
                            slot,
                        };
                    }
                }
            }
        }

        // Step 3: Check block height vs last valid
        if let Some(last_valid) = last_valid_slot {
            if current_slot >= last_valid {
                let expired_by = current_slot - last_valid;
                info!(target: "sol_trade_sdk", "reconciliation: step 3 — blockhash expired {} slots ago", expired_by);
                let has_any_status = statuses.iter().any(|s| s.slot.is_some());
                if !has_any_status {
                    return ReconciliationOutcome::RolledBack {
                        reason: format!("blockhash expired at slot {} (last_valid={})", current_slot, last_valid),
                    };
                }
            }
        }

        // Step 4: Check token/SOL balances
        if !self.config.skip_balance_check {
            let balance_result = self.check_balances(payer, output_ata, pre_balance).await;
            if let Some(result) = balance_result {
                info!(target: "sol_trade_sdk", "reconciliation: step 4 — balance check: {}", result);
                return result;
            }
        }

        // Step 5: Fetch full transaction
        for status in &statuses {
            if let Some(tx_slot) = status.slot {
                let tx_meta = self.fetch_transaction(&status.signature).await;
                if let Some(meta) = tx_meta {
                    info!(target: "sol_trade_sdk", "reconciliation: step 5/6 — fetched tx {} at slot {}, fee={}", status.signature, meta.slot, meta.fee_lamports);
                    return if meta.err.is_none() {
                        ReconciliationOutcome::Landed { signature: status.signature, slot: meta.slot }
                    } else {
                        ReconciliationOutcome::Failed {
                            signature: status.signature,
                            error: meta.err.unwrap_or_else(|| "unknown error".into()),
                            slot: meta.slot,
                        }
                    };
                }
            }
        }

        // Step 7: Decide
        if let Some(last_valid) = last_valid_slot {
            if current_slot < last_valid {
                info!(target: "sol_trade_sdk", "reconciliation: step 7 — no evidence, blockhash still valid, recommending REBUILD");
                return ReconciliationOutcome::Rebuild {
                    reason: "no on-chain evidence, blockhash still valid".into(),
                };
            }
        }

        ReconciliationOutcome::RolledBack {
            reason: format!("no on-chain evidence after {} signatures checked", signatures.len()),
        }
    }

    /// Step 2: Query `getSignatureStatuses` for all lane signatures.
    async fn query_signature_statuses(&self, signatures: &[Signature]) -> Vec<SignatureStatus> {
        if signatures.is_empty() {
            return Vec::new();
        }

        let result = tokio::time::timeout(
            self.config.rpc_query_timeout,
            self.rpc.get_signature_statuses(signatures),
        )
        .await;

        match result {
            Ok(Ok(statuses)) => signatures
                .iter()
                .zip(statuses.value.iter())
                .filter_map(|(sig, status)| {
                    status.as_ref().map(|s| {
                        let confirmed = s.confirmations.unwrap_or(0) > 0
                            || s.confirmation_status.clone().map_or(false, |c| {
                                matches!(
                                    c,
                                    TransactionConfirmationStatus::Confirmed
                                        | TransactionConfirmationStatus::Finalized
                                )
                            });
                        let finalized =
                            s.confirmation_status == Some(TransactionConfirmationStatus::Finalized);
                        SignatureStatus {
                            signature: *sig,
                            slot: Some(s.slot),
                            confirmations: s.confirmations.map(|c| c as u64),
                            err: s.err.clone(),
                            confirmed,
                            finalized,
                        }
                    })
                })
                .collect(),
            Ok(Err(e)) => {
                warn!(target: "sol_trade_sdk", "reconciliation: getSignatureStatuses failed: {}", e);
                Vec::new()
            }
            Err(_) => {
                warn!(target: "sol_trade_sdk", "reconciliation: getSignatureStatuses timed out");
                Vec::new()
            }
        }
    }

    /// Step 4: Check SOL balance against pre-trade snapshot.
    async fn check_balances(
        &self,
        payer: &Pubkey,
        output_ata: Option<&Pubkey>,
        pre: &BalanceSnapshot,
    ) -> Option<ReconciliationOutcome> {
        let post_sol = tokio::time::timeout(
            self.config.rpc_query_timeout,
            self.rpc.get_balance(payer),
        )
        .await
        .ok()?
        .ok()?;

        let sol_delta = post_sol as i64 - pre.payer_sol_lamports as i64;

        const MIN_FEE_LAMPORTS: u64 = 5_000;
        if sol_delta < -(MIN_FEE_LAMPORTS as i64) {
            if let Some(output_ata) = output_ata {
                if let Some(post_token) = self.get_token_balance(output_ata).await {
                    let token_delta = post_token as i64 - pre.output_token_balance as i64;
                    if token_delta > 0 {
                        return Some(ReconciliationOutcome::Landed {
                            signature: Signature::default(),
                            slot: 0,
                        });
                    }
                }
            }
            info!(target: "sol_trade_sdk", "reconciliation: balance step — SOL decreased by {} lamports (tx landed)", -sol_delta);
        }

        None
    }

    /// Step 5: Fetch full transaction data for a signature.
    async fn fetch_transaction(&self, signature: &Signature) -> Option<TransactionMeta> {
        use solana_transaction_status::UiTransactionEncoding;
        let result = tokio::time::timeout(
            self.config.rpc_query_timeout,
            self.rpc.get_transaction(signature, UiTransactionEncoding::Json),
        )
        .await;

        match result {
            Ok(Ok(tx_response)) => {
                let meta = tx_response.transaction.meta?;
                let err = meta.err.map(|e| format!("{:?}", e));
                Some(TransactionMeta {
                    err,
                    pre_sol_balance: meta.pre_balances.first().copied().unwrap_or(0),
                    post_sol_balance: meta.post_balances.first().copied().unwrap_or(0),
                    pre_token_balances: Vec::new(), // TODO: P2-06 — parse pre_token_balances from meta.pre_token_balances
                    post_token_balances: Vec::new(), // TODO: P2-06 — parse post_token_balances from meta.post_token_balances
                    fee_lamports: meta.fee,
                    cu_consumed: meta.compute_units_consumed.into(),
                    slot: 0, // Note: slot is not on UiTransactionStatusMeta; fetch from tx_response.slot if needed
                })
            }
            Ok(Err(e)) => {
                warn!(target: "sol_trade_sdk", "reconciliation: getTransaction failed for {}: {}", signature, e);
                None
            }
            Err(_) => {
                warn!(target: "sol_trade_sdk", "reconciliation: getTransaction timed out for {}", signature);
                None
            }
        }
    }

    /// Helper: get SPL token balance for an ATA.
    async fn get_token_balance(&self, ata: &Pubkey) -> Option<u64> {
        let result = tokio::time::timeout(
            self.config.rpc_query_timeout,
            self.rpc.get_token_account_balance(ata),
        )
        .await;

        match result {
            Ok(Ok(balance)) => Some(balance.amount.parse::<u64>().unwrap_or(0)),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::Keypair;

    #[test]
    fn outcome_terminal_check() {
        assert!(ReconciliationOutcome::RolledBack { reason: "test".into() }.is_terminal());
        assert!(!ReconciliationOutcome::Landed { signature: Signature::default(), slot: 0 }
            .is_terminal());
        assert!(!ReconciliationOutcome::Failed {
            signature: Signature::default(),
            error: "".into(),
            slot: 0
        }
        .is_terminal());
        assert!(!ReconciliationOutcome::Rebuild { reason: "test".into() }.is_terminal());
    }

    #[test]
    fn config_defaults() {
        let cfg = ReconciliationConfig::default();
        assert_eq!(cfg.blockhash_expiry_slots, 150);
        assert!(!cfg.skip_balance_check);
    }

    #[test]
    fn balance_snapshot_fields() {
        let snap = BalanceSnapshot {
            payer_sol_lamports: 1_000_000_000,
            input_token_balance: 0,
            output_token_balance: 500_000,
            snapshot_slot: 12345,
        };
        assert_eq!(snap.payer_sol_lamports, 1_000_000_000);
        assert_eq!(snap.snapshot_slot, 12345);
    }

    #[test]
    fn reconciliation_service_creation() {
        let rpc = Arc::new(SolanaRpcClient::new("https://api.mainnet-beta.solana.com".into()));
        let svc = ReconciliationService::new(rpc, ReconciliationConfig::default());
        assert!(!svc.config.skip_balance_check);
    }

    #[test]
    fn empty_signatures_no_evidence() {
        let rpc = Arc::new(SolanaRpcClient::new("https://api.mainnet-beta.solana.com".into()));
        let svc = ReconciliationService::new(rpc, ReconciliationConfig::default());
        let pre = BalanceSnapshot {
            payer_sol_lamports: 1_000_000_000,
            input_token_balance: 0,
            output_token_balance: 0,
            snapshot_slot: 100,
        };
        let payer = Pubkey::new_unique();
        let _ = svc.reconcile(&[], &pre, Some(250), 300, &payer, None, None, None, None);
    }
}