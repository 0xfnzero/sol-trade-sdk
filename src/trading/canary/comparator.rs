//! Shadow comparator — records canary trade outcomes alongside shadow-mode
//! decisions for accuracy comparison.
//!
//! After each canary trade completes, the outcome is recorded here. If shadow
//! mode was running in parallel, the corresponding shadow decision (by slot
//! and token mint) is cross-referenced to measure how well shadow predictions
//! match real outcomes.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::trading::core::state::TradeDirection;

// ---------------------------------------------------------------------------
// Record
// ---------------------------------------------------------------------------

/// A single canary trade outcome record, optionally linked to a shadow decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryTradeRecord {
    /// Unique record ID.
    pub id: u64,
    /// Approval ID from the ManualApprovalGate.
    pub approval_id: String,
    /// Protocol used.
    pub protocol: String,
    /// Token mint address.
    pub mint: String,
    /// Trade direction.
    pub direction: TradeDirection,
    /// SOL amount.
    pub sol_amount: f64,
    /// Whether the trade landed on chain.
    pub landed: bool,
    /// Actual profit in lamports (negative = loss).
    pub actual_profit_lamports: i64,
    /// Transaction signature (if landed).
    pub signature: Option<String>,
    /// Slot the trade landed at.
    pub slot: Option<u64>,
    /// Timestamp (unix ms).
    pub timestamp_ms: u64,

    // ── Shadow comparison fields ──
    /// Shadow decision direction at the same slot (if available).
    pub shadow_direction: Option<TradeDirection>,
    /// Shadow confidence at decision time (if available).
    pub shadow_confidence: Option<f64>,
    /// Whether shadow agreed with the direction.
    pub shadow_agreed: Option<bool>,
}

/// Comparison summary between canary and shadow mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowComparisonSummary {
    /// Total canary trades recorded.
    pub total_canary_trades: u64,
    /// Trades with a corresponding shadow decision.
    pub total_with_shadow: u64,
    /// Trades where shadow agreed with direction.
    pub shadow_agreed: u64,
    /// Trades where shadow disagreed.
    pub shadow_disagreed: u64,
    /// Agreement rate (0.0-1.0).
    pub shadow_agreement_rate: f64,
    /// Total profit from canary trades (lamports).
    pub total_profit_lamports: i64,
    /// Trades that landed on chain.
    pub landed_count: u64,
    /// Trades that failed.
    pub failed_count: u64,
    /// Mode label.
    pub mode: String,
}

// ---------------------------------------------------------------------------
// Shadow Comparator
// ---------------------------------------------------------------------------

/// Records canary trade outcomes and compares them with shadow decisions.
///
/// Thread-safe via `Mutex<Vec<CanaryTradeRecord>>`.
pub struct ShadowComparator {
    records: Mutex<Vec<CanaryTradeRecord>>,
    next_id: Mutex<u64>,
}

impl ShadowComparator {
    /// Create a new shadow comparator.
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::with_capacity(1024)),
            next_id: Mutex::new(1),
        }
    }

    /// Record a canary trade outcome.
    ///
    /// * `approval_id` — ID from the ManualApprovalGate
    /// * `protocol` — DEX protocol name
    /// * `mint` — token mint address
    /// * `direction` — BUY or SELL
    /// * `sol_amount` — amount in SOL
    /// * `landed` — whether the transaction confirmed on chain
    /// * `actual_profit_lamports` — net profit/loss in lamports
    /// * `signature` — optional tx signature
    /// * `slot` — optional slot number
    /// * `shadow_direction` — optional shadow mode direction for comparison
    /// * `shadow_confidence` — optional shadow confidence
    pub fn record(
        &self,
        approval_id: String,
        protocol: String,
        mint: String,
        direction: TradeDirection,
        sol_amount: f64,
        landed: bool,
        actual_profit_lamports: i64,
        signature: Option<String>,
        slot: Option<u64>,
        shadow_direction: Option<TradeDirection>,
        shadow_confidence: Option<f64>,
    ) -> u64 {
        let mut id_lock = self.next_id.lock().unwrap();
        let id = *id_lock;
        *id_lock += 1;

        let shadow_agreed = shadow_direction.map(|sd| sd == direction);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let record = CanaryTradeRecord {
            id,
            approval_id,
            protocol,
            mint,
            direction,
            sol_amount,
            landed,
            actual_profit_lamports,
            signature,
            slot,
            timestamp_ms: now,
            shadow_direction,
            shadow_confidence,
            shadow_agreed,
        };

        let mut records = self.records.lock().unwrap();
        records.push(record);

        info!(target: "sol_trade_sdk", "canary: trade #{} recorded (landed={}, profit={} lamports)", id, landed, actual_profit_lamports);

        id
    }

    /// Get a summary of the shadow comparison.
    pub fn summary(&self) -> ShadowComparisonSummary {
        let records = self.records.lock().unwrap();
        let total = records.len() as u64;

        let with_shadow = records.iter().filter(|r| r.shadow_direction.is_some()).count() as u64;
        let agreed = records.iter().filter(|r| r.shadow_agreed == Some(true)).count() as u64;
        let disagreed = records.iter().filter(|r| r.shadow_agreed == Some(false)).count() as u64;
        let total_profit: i64 = records.iter().map(|r| r.actual_profit_lamports).sum();
        let landed = records.iter().filter(|r| r.landed).count() as u64;
        let failed = records.iter().filter(|r| !r.landed).count() as u64;

        let agreement_rate = if with_shadow > 0 {
            agreed as f64 / with_shadow as f64
        } else {
            0.0
        };

        ShadowComparisonSummary {
            total_canary_trades: total,
            total_with_shadow: with_shadow,
            shadow_agreed: agreed,
            shadow_disagreed: disagreed,
            shadow_agreement_rate: agreement_rate,
            total_profit_lamports: total_profit,
            landed_count: landed,
            failed_count: failed,
            mode: "canary".to_string(),
        }
    }

    /// Get all records (for HTTP display).
    pub fn get_records(&self, limit: usize) -> Vec<CanaryTradeRecord> {
        let records = self.records.lock().unwrap();
        records.iter().rev().take(limit).cloned().collect()
    }

    /// Get total record count.
    pub fn count(&self) -> u64 {
        let records = self.records.lock().unwrap();
        records.len() as u64
    }
}

impl Default for ShadowComparator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_comparator_starts_empty() {
        let c = ShadowComparator::new();
        let s = c.summary();
        assert_eq!(s.total_canary_trades, 0);
        assert_eq!(s.shadow_agreement_rate, 0.0);
    }

    #[test]
    fn record_trade_increments_count() {
        let c = ShadowComparator::new();
        c.record(
            "approval-1".into(),
            "pumpfun".into(),
            "mint123".into(),
            TradeDirection::Buy,
            0.01,
            true,
            5000,
            Some("sig1".into()),
            Some(12345),
            Some(TradeDirection::Buy),
            Some(0.65),
        );
        assert_eq!(c.count(), 1);
    }

    #[test]
    fn shadow_agreement_tracking() {
        let c = ShadowComparator::new();

        // Agreeing shadow
        c.record(
            "a1".into(), "pumpfun".into(), "mint1".into(),
            TradeDirection::Buy, 0.01, true, 1000,
            None, None, Some(TradeDirection::Buy), Some(0.7),
        );

        // Disagreeing shadow
        c.record(
            "a2".into(), "pumpfun".into(), "mint2".into(),
            TradeDirection::Buy, 0.01, true, 2000,
            None, None, Some(TradeDirection::Sell), Some(0.6),
        );

        let s = c.summary();
        assert_eq!(s.total_canary_trades, 2);
        assert_eq!(s.total_with_shadow, 2);
        assert_eq!(s.shadow_agreed, 1);
        assert_eq!(s.shadow_disagreed, 1);
        assert!((s.shadow_agreement_rate - 0.5).abs() < 1e-6);
    }

    #[test]
    fn no_shadow_no_agreement_check() {
        let c = ShadowComparator::new();
        c.record(
            "a1".into(), "pumpfun".into(), "mint1".into(),
            TradeDirection::Buy, 0.01, true, 1000,
            None, None, None, None,
        );
        let s = c.summary();
        assert_eq!(s.total_canary_trades, 1);
        assert_eq!(s.total_with_shadow, 0);
        assert_eq!(s.shadow_agreed, 0);
    }

    #[test]
    fn records_returned_reverse_chronological() {
        let c = ShadowComparator::new();
        for i in 0..5 {
            c.record(
                format!("a{}", i), "p".into(), "m".into(),
                TradeDirection::Buy, 0.01, true, i * 100,
                None, None, None, None,
            );
        }
        let recent = c.get_records(3);
        assert_eq!(recent.len(), 3);
        // Most recent first — IDs are 5, 4, 3
        assert_eq!(recent[0].id, 5);
        assert_eq!(recent[1].id, 4);
        assert_eq!(recent[2].id, 3);
    }
}