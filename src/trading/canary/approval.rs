//! Manual approval gate — holds pending trades in a queue and exposes
//! HTTP endpoints for human approval or rejection.
//!
//! Each pending trade has a unique UUID, a timeout, and full context
//! (protocol, mint, direction, amount, expected profit) so the operator
//! can make an informed decision.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::trading::core::orchestrator::TradePlan;
use crate::trading::core::state::TradeDirection;

// ---------------------------------------------------------------------------
// Pending trade descriptor
// ---------------------------------------------------------------------------

/// A trade waiting for manual approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryPendingTrade {
    /// Unique approval ID (UUID v4).
    pub id: String,
    /// When the trade was proposed.
    pub created_at_ms: u64,
    /// Deadline for approval (unix ms).
    pub expires_at_ms: u64,
    /// Protocol (e.g. "pumpfun", "raydium_cpmm").
    pub protocol: String,
    /// Token mint address.
    pub mint: String,
    /// Trade direction.
    pub direction: TradeDirection,
    /// Amount in SOL.
    pub sol_amount: f64,
    /// Expected net profit in lamports.
    pub expected_profit_lamports: i64,
    /// Confidence score 0.0-1.0 (from strategy).
    pub confidence: f64,
    /// Estimated total cost in lamports (fees + tip).
    pub estimated_cost_lamports: u64,
    /// Current spread in basis points.
    pub spread_bps: u64,
    /// Free-form strategy reason.
    pub reason: String,
    /// Approval status.
    pub status: CanaryApprovalStatus,
}

/// Status of a pending canary trade.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CanaryApprovalStatus {
    /// Waiting for operator decision.
    Pending,
    /// Approved by operator.
    Approved,
    /// Rejected by operator.
    Rejected,
    /// Automatically rejected due to timeout.
    TimedOut,
    /// Submitted to the network.
    Submitted,
    /// Completed (landed on chain).
    Completed,
}

impl CanaryPendingTrade {
    fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }
}

// ---------------------------------------------------------------------------
// Manual approval gate
// ---------------------------------------------------------------------------

/// Holds pending trades and exposes approval/rejection operations.
///
/// Thread-safe: all mutations go through `Mutex<HashMap<String, ...>>`.
pub struct ManualApprovalGate {
    /// Pending trades indexed by approval ID.
    pending: Mutex<HashMap<String, CanaryPendingTrade>>,
    /// Approval timeout duration.
    timeout: Duration,
    /// Total number of trades processed (approved + rejected + timed out).
    total_processed: Mutex<u64>,
    /// Total approved.
    total_approved: Mutex<u64>,
    /// Total rejected.
    total_rejected: Mutex<u64>,
}

impl ManualApprovalGate {
    /// Create a new approval gate with the given timeout.
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            timeout: Duration::from_secs(timeout_secs),
            total_processed: Mutex::new(0),
            total_approved: Mutex::new(0),
            total_rejected: Mutex::new(0),
        }
    }

    /// Submit a trade plan for manual approval.
    /// Returns the approval ID.
    pub fn submit(
        &self,
        plan: &TradePlan,
        confidence: f64,
        spread_bps: u64,
        estimated_cost_lamports: u64,
        reason: String,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let expires_at = now + self.timeout.as_millis() as u64;

        let trade = CanaryPendingTrade {
            id: id.clone(),
            created_at_ms: now,
            expires_at_ms: expires_at,
            protocol: plan.intent.protocol.clone(),
            mint: plan.intent.mint.clone(),
            direction: plan.intent.direction,
            sol_amount: plan.intent.input_amount as f64 / 1_000_000_000.0, // lamports → SOL
            expected_profit_lamports: plan.intent.min_output_amount as i64,
            confidence,
            estimated_cost_lamports,
            spread_bps,
            reason,
            status: CanaryApprovalStatus::Pending,
        };

        let mut pending = self.pending.lock().unwrap();
        pending.insert(id.clone(), trade);
        info!(target: "sol_trade_sdk", "canary: trade {} submitted for approval (protocol={}, amount={:.6} SOL)",
            id, plan.intent.protocol, plan.intent.input_amount as f64 / 1_000_000_000.0);

        id
    }

    /// Approve a pending trade by ID. Returns the trade if found and pending.
    pub fn approve(&self, id: &str) -> Option<CanaryPendingTrade> {
        let mut pending = self.pending.lock().unwrap();
        if let Some(trade) = pending.get_mut(id) {
            if trade.status == CanaryApprovalStatus::Pending {
                trade.status = CanaryApprovalStatus::Approved;
                *self.total_approved.lock().unwrap() += 1;
                *self.total_processed.lock().unwrap() += 1;
                info!(target: "sol_trade_sdk", "canary: trade {} APPROVED by operator", id);
                return Some(trade.clone());
            }
        }
        None
    }

    /// Reject a pending trade by ID. Returns the trade if found and pending.
    pub fn reject(&self, id: &str, reason: &str) -> Option<CanaryPendingTrade> {
        let mut pending = self.pending.lock().unwrap();
        if let Some(trade) = pending.get_mut(id) {
            if trade.status == CanaryApprovalStatus::Pending {
                trade.status = CanaryApprovalStatus::Rejected;
                *self.total_rejected.lock().unwrap() += 1;
                *self.total_processed.lock().unwrap() += 1;
                warn!(target: "sol_trade_sdk", "canary: trade {} REJECTED by operator: {}", id, reason);
                return Some(trade.clone());
            }
        }
        None
    }

    /// Mark a trade as submitted (after execution).
    pub fn mark_submitted(&self, id: &str) {
        let mut pending = self.pending.lock().unwrap();
        if let Some(trade) = pending.get_mut(id) {
            trade.status = CanaryApprovalStatus::Submitted;
        }
    }

    /// Mark a trade as completed.
    pub fn mark_completed(&self, id: &str) {
        let mut pending = self.pending.lock().unwrap();
        if let Some(trade) = pending.get_mut(id) {
            trade.status = CanaryApprovalStatus::Completed;
        }
    }

    /// Prune expired trades (auto-reject them). Returns IDs of pruned trades.
    pub fn prune_expired(&self) -> Vec<String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut pending = self.pending.lock().unwrap();
        let mut expired = Vec::new();

        pending.retain(|id, trade| {
            if trade.is_expired(now) && trade.status == CanaryApprovalStatus::Pending {
                trade.status = CanaryApprovalStatus::TimedOut;
                *self.total_rejected.lock().unwrap() += 1;
                *self.total_processed.lock().unwrap() += 1;
                expired.push(id.clone());
                warn!(target: "sol_trade_sdk", "canary: trade {} TIMED OUT (expired at {})", id, trade.expires_at_ms);
                false // remove from pending
            } else {
                true // keep in pending if not expired or already processed
            }
        });

        expired
    }

    /// Get all currently pending (not approved, not rejected) trades.
    pub fn get_pending(&self) -> Vec<CanaryPendingTrade> {
        let pending = self.pending.lock().unwrap();
        pending
            .values()
            .filter(|t| t.status == CanaryApprovalStatus::Pending)
            .cloned()
            .collect()
    }

    /// Get a specific trade by ID.
    pub fn get(&self, id: &str) -> Option<CanaryPendingTrade> {
        let pending = self.pending.lock().unwrap();
        pending.get(id).cloned()
    }

    /// Approve ALL currently pending trades. Returns count approved.
    pub fn approve_all(&self) -> usize {
        let ids: Vec<String> = self.get_pending().iter().map(|t| t.id.clone()).collect();
        let count = ids.len();
        for id in &ids {
            self.approve(id);
        }
        info!(target: "sol_trade_sdk", "canary: bulk-approved {} pending trades", count);
        count
    }

    /// Reject ALL currently pending trades. Returns count rejected.
    pub fn reject_all(&self, reason: &str) -> usize {
        let ids: Vec<String> = self.get_pending().iter().map(|t| t.id.clone()).collect();
        let count = ids.len();
        for id in &ids {
            self.reject(&id, reason);
        }
        info!(target: "sol_trade_sdk", "canary: bulk-rejected {} pending trades: {}", count, reason);
        count
    }

    // ── Stats ──

    pub fn total_pending(&self) -> usize {
        self.get_pending().len()
    }

    pub fn total_approved(&self) -> u64 {
        *self.total_approved.lock().unwrap()
    }

    pub fn total_rejected(&self) -> u64 {
        *self.total_rejected.lock().unwrap()
    }

    pub fn total_processed(&self) -> u64 {
        *self.total_processed.lock().unwrap()
    }
}

// ---------------------------------------------------------------------------
// HTTP route helpers (returned as structured data for the HTTP server to render)
// ---------------------------------------------------------------------------

/// A summary of approval gate stats, suitable for JSON serialization.
#[derive(Debug, Serialize)]
pub struct ApprovalGateStats {
    pub pending: usize,
    pub approved: u64,
    pub rejected: u64,
    pub total_processed: u64,
    pub mode: String,
}

impl ManualApprovalGate {
    pub fn stats(&self) -> ApprovalGateStats {
        ApprovalGateStats {
            pending: self.total_pending(),
            approved: self.total_approved(),
            rejected: self.total_rejected(),
            total_processed: self.total_processed(),
            mode: "canary".to_string(),
        }
    }
}