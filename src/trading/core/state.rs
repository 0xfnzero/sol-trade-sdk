//! Trade intent state machine.
//!
//! Every trade intent starts as DETECTED and progresses through VALIDATED, BUILT,
//! SIGNED, SUBMITTED, LANDED, and finally SETTLED. Any transition may instead
//! enter a terminal failure state (REJECTED, EXPIRED, FAILED, CANCELLED, AMBIGUOUS).
//!
//! Each transition records a timestamp and optional evidence/error for audit trails.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// State enum
// ---------------------------------------------------------------------------

/// All possible states of a trade intent, from detection through settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeState {
    /// Opportunity detected (pool event, price move, signal trigger)
    Detected,
    /// Pre-trade validation passed (risk gates, balance, protocol checks)
    Validated,
    /// Transaction instructions constructed
    Built,
    /// Transaction signed with execution keypair
    Signed,
    /// Submitted to at least one submission lane (RPC, Jito, SWQoS)
    Submitted,
    /// Confirmed landed on chain (signature observed in a block)
    Landed,
    /// Settled — trade outcome known (swap completed, position opened)
    Settled,

    // ── Terminal failure states ──
    /// Rejected by risk gates or strategy pre-checks
    Rejected,
    /// Expired before submission (blockhash stale, market ended)
    Expired,
    /// Submitted but failed on chain (instruction error, slippage exceeded)
    Failed,
    /// Cancelled by operator (emergency kill, manual override)
    Cancelled,
    /// Submitted but landing state is ambiguous (reconciliation needed)
    Ambiguous,
}

impl TradeState {
    /// Returns `true` if this is a terminal (non-retryable) state.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TradeState::Settled
                | TradeState::Rejected
                | TradeState::Expired
                | TradeState::Failed
                | TradeState::Cancelled
        )
    }

    /// Returns `true` if this state represents a successfully completed trade.
    pub fn is_success(self) -> bool {
        matches!(self, TradeState::Settled | TradeState::Landed)
    }

    /// Returns `true` if this state needs manual reconciliation.
    pub fn needs_reconciliation(self) -> bool {
        matches!(self, TradeState::Ambiguous)
    }
}

// ---------------------------------------------------------------------------
// Allowed transitions (compile-time enforcement via methods)
// ---------------------------------------------------------------------------

/// Validate that a transition from `from` to `to` is legal.
pub fn is_valid_transition(from: TradeState, to: TradeState) -> bool {
    use TradeState::*;
    matches!(
        (from, to),
        // Happy path
        (Detected, Validated)
            | (Validated, Built)
            | (Built, Signed)
            | (Signed, Submitted)
            | (Submitted, Landed)
            | (Landed, Settled)
        // Terminal exits from any pre-submission state
            | (Detected, Rejected)
            | (Detected, Expired)
            | (Detected, Cancelled)
            | (Validated, Rejected)
            | (Validated, Expired)
            | (Validated, Cancelled)
            | (Built, Failed)
            | (Built, Expired)
            | (Built, Cancelled)
            | (Signed, Expired)
            | (Signed, Cancelled)
        // Terminal exits from submission
            | (Submitted, Failed)
            | (Submitted, Expired)
            | (Submitted, Cancelled)
            | (Submitted, Ambiguous)
        // Recovery from ambiguity
            | (Ambiguous, Landed)
            | (Ambiguous, Failed)
        // Re-submit after ambiguous resolution
            | (Ambiguous, Signed)
    )
}

// ---------------------------------------------------------------------------
// Transition record
// ---------------------------------------------------------------------------

/// A single state transition with timestamp and optional evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransition {
    pub from: TradeState,
    pub to: TradeState,
    /// Unix timestamp (milliseconds) when the transition occurred.
    pub timestamp_ms: u64,
    /// Human-readable reason or error description.
    pub reason: String,
    /// Optional supporting evidence (error code, rejected reason, tx signature).
    pub evidence: Option<String>,
}

impl StateTransition {
    pub fn new(from: TradeState, to: TradeState, reason: impl Into<String>) -> Self {
        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        Self { from, to, timestamp_ms: now, reason: reason.into(), evidence: None }
    }

    pub fn with_evidence(mut self, evidence: impl Into<String>) -> Self {
        self.evidence = Some(evidence.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Trade Intent
// ---------------------------------------------------------------------------

/// A single trade intent with full state machine, transitions, and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeIntent {
    /// Unique identifier for this trade intent.
    pub id: Uuid,
    /// Current state.
    pub state: TradeState,
    /// Ordered list of all transitions this intent has undergone.
    pub transitions: Vec<StateTransition>,
    /// Slot when the opportunity was detected (if known).
    pub detected_slot: Option<u64>,
    /// Transaction signature (if submitted).
    pub signature: Option<String>,
    /// Protocol name (e.g., "pumpfun", "pumpswap", "raydium_cpmm").
    pub protocol: String,
    /// Mint address of the token being traded.
    pub mint: String,
    /// Trade direction.
    pub direction: TradeDirection,
    /// Input amount in smallest units.
    pub input_amount: u64,
    /// Minimum output amount (after slippage).
    pub min_output_amount: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeDirection {
    Buy,
    Sell,
}

impl TradeIntent {
    /// Create a new trade intent in the `Detected` state.
    pub fn new(
        protocol: impl Into<String>,
        mint: impl Into<String>,
        direction: TradeDirection,
        input_amount: u64,
        min_output_amount: u64,
        detected_slot: Option<u64>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            state: TradeState::Detected,
            transitions: vec![StateTransition::new(
                TradeState::Detected,
                TradeState::Detected,
                "intent created",
            )],
            detected_slot,
            signature: None,
            protocol: protocol.into(),
            mint: mint.into(),
            direction,
            input_amount,
            min_output_amount,
        }
    }

    /// Attempt a state transition. Returns `Err` if the move is invalid.
    pub fn transition(
        &mut self,
        to: TradeState,
        reason: impl Into<String>,
    ) -> anyhow::Result<&StateTransition> {
        let from = self.state;
        if !is_valid_transition(from, to) {
            anyhow::bail!("Invalid state transition: {:?} -> {:?} (intent {})", from, to, self.id);
        }
        let t = StateTransition::new(from, to, reason);
        self.state = to;
        self.transitions.push(t);
        self.transitions.last().ok_or_else(|| anyhow::anyhow!("transition list empty after push"))
    }

    /// Transition with evidence attached.
    pub fn transition_with_evidence(
        &mut self,
        to: TradeState,
        reason: impl Into<String>,
        evidence: impl Into<String>,
    ) -> anyhow::Result<&StateTransition> {
        let from = self.state;
        if !is_valid_transition(from, to) {
            anyhow::bail!("Invalid state transition: {:?} -> {:?} (intent {})", from, to, self.id);
        }
        let mut t = StateTransition::new(from, to, reason);
        t.evidence = Some(evidence.into());
        self.state = to;
        self.transitions.push(t);
        self.transitions.last().ok_or_else(|| anyhow::anyhow!("transition list empty after push"))
    }

    /// Record a successful submission signature.
    pub fn record_submission(&mut self, signature: impl Into<String>) -> anyhow::Result<()> {
        self.transition(TradeState::Submitted, "submitted to lane")?;
        self.signature = Some(signature.into());
        Ok(())
    }

    /// Time (ms) spent in the current state.
    pub fn elapsed_in_current_state_ms(&self) -> Option<u64> {
        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        self.transitions.last().map(|t| now.saturating_sub(t.timestamp_ms))
    }

    /// Total time (ms) since the intent was created.
    pub fn total_lifetime_ms(&self) -> Option<u64> {
        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        self.transitions.first().map(|t| now.saturating_sub(t.timestamp_ms))
    }

    /// Human-readable state path.
    pub fn state_path(&self) -> String {
        self.transitions
            .iter()
            .map(|t| format!("{:?}→{:?}", t.from, t.to))
            .collect::<Vec<_>>()
            .join(" → ")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use TradeState::*;

    #[test]
    fn happy_path_transitions() {
        let mut intent =
            TradeIntent::new("pumpfun", "mint123", TradeDirection::Buy, 1000, 990, Some(42));
        assert_eq!(intent.state, Detected);

        intent.transition(Validated, "risk check passed").unwrap();
        assert_eq!(intent.state, Validated);

        intent.transition(Built, "instructions built").unwrap();
        assert_eq!(intent.state, Built);

        intent.transition(Signed, "signed by execution key").unwrap();
        assert_eq!(intent.state, Signed);

        intent.record_submission("5VERv8NM1i...").unwrap();
        assert_eq!(intent.state, Submitted);
        assert_eq!(intent.signature.as_deref(), Some("5VERv8NM1i..."));

        intent.transition(Landed, "observed in slot 123456").unwrap();
        assert_eq!(intent.state, Landed);

        intent.transition(Settled, "swap confirmed").unwrap();
        assert_eq!(intent.state, Settled);
        assert!(intent.state.is_terminal());
    }

    #[test]
    fn invalid_transition_returns_error() {
        let mut intent =
            TradeIntent::new("pumpfun", "mint123", TradeDirection::Buy, 1000, 990, None);
        // Can't jump from Detected to Settled
        assert!(intent.transition(Settled, "skip ahead").is_err());
        assert_eq!(intent.state, Detected);
    }

    #[test]
    fn terminal_rejection() {
        let mut intent = TradeIntent::new("bonk", "mint456", TradeDirection::Sell, 500, 490, None);
        intent.transition(Rejected, "max_sol_per_trade exceeded").unwrap();
        assert_eq!(intent.state, Rejected);
        assert!(intent.state.is_terminal());
    }

    #[test]
    fn ambiguous_recovery() {
        let mut intent =
            TradeIntent::new("pumpswap", "mint789", TradeDirection::Buy, 2000, 1980, Some(100));
        intent.transition(Validated, "ok").unwrap();
        intent.transition(Built, "ok").unwrap();
        intent.transition(Signed, "ok").unwrap();
        intent.record_submission("sig123").unwrap();
        intent.transition(Ambiguous, "submitted but no confirmation after 10s").unwrap();
        assert!(intent.state.needs_reconciliation());

        // Recover: re-submit
        intent.transition(Signed, "re-submitting after reconciliation").unwrap();
        assert_eq!(intent.state, Signed);

        intent.record_submission("sig456").unwrap();
        intent.transition(Landed, "confirmed on re-submit").unwrap();
        intent.transition(Settled, "done").unwrap();
        assert!(intent.state.is_success());
    }

    #[test]
    fn transition_preserves_evidence() {
        let mut intent =
            TradeIntent::new("raydium_cpmm", "mintABC", TradeDirection::Buy, 3000, 2900, None);
        intent
            .transition_with_evidence(
                Rejected,
                "insufficient balance",
                "balance: 0.01 SOL, min: 0.05 SOL",
            )
            .unwrap();
        let last = intent.transitions.last().unwrap();
        assert_eq!(last.from, Detected);
        assert_eq!(last.to, Rejected);
        assert!(last.evidence.as_deref().unwrap().contains("0.01 SOL"));
    }

    #[test]
    fn state_path_readable() {
        let mut intent = TradeIntent::new("pumpfun", "x", TradeDirection::Buy, 100, 99, None);
        intent.transition(Validated, "ok").unwrap();
        intent.transition(Rejected, "bad").unwrap();
        let path = intent.state_path();
        assert!(path.contains("Detected→Detected"));
        assert!(path.contains("Detected→Validated"));
        assert!(path.contains("Validated→Rejected"));
    }

    #[test]
    fn is_valid_transition_table() {
        // Every valid happy path transition
        assert!(is_valid_transition(Detected, Validated));
        assert!(is_valid_transition(Validated, Built));
        assert!(is_valid_transition(Built, Signed));
        assert!(is_valid_transition(Signed, Submitted));
        assert!(is_valid_transition(Submitted, Landed));
        assert!(is_valid_transition(Landed, Settled));

        // Invalid: skip states forward
        assert!(!is_valid_transition(Detected, Built));
        assert!(!is_valid_transition(Detected, Submitted));
        assert!(!is_valid_transition(Validated, Submitted));
        assert!(!is_valid_transition(Submitted, Settled));

        // Invalid: cannot go backwards
        assert!(!is_valid_transition(Landed, Submitted));
        assert!(!is_valid_transition(Settled, Landed));
    }
}
