//! Canary mode — tiny wallet, tight limits, manual triggers first.
//!
//! Phase 12 of the productionization roadmap: run a tiny-wallet trading
//! binary alongside shadow mode. All trades require manual HTTP approval
//! before submission. After each trade, outcomes are compared against
//! shadow-mode decisions for accuracy measurement.
//!
//! # Architecture
//!
//! ```text
//! Orchestrator ──plan_trade()──→ ManualApprovalGate
//!                                     │
//!                                     ├─ pending queue (Mutex<VecDeque<CanaryPendingTrade>>)
//!                                     ├─ HTTP: GET  /canary/pending
//!                                     ├─ HTTP: POST /canary/approve/:id
//!                                     ├─ HTTP: POST /canary/reject/:id
//!                                     ├─ HTTP: POST /canary/approve-all
//!                                     └─ HTTP: POST /canary/reject-all
//!                                              │
//!                                   approved ──┤
//!                                              │
//!                                              ▼
//!                                      Orchestrator.execute_plan()
//!                                              │
//!                                              ▼
//!                                     ShadowComparator.record_outcome()
//!                                              │
//!                                     HTTP: GET /canary/shadow-compare
//! ```

mod approval;
mod comparator;

pub use approval::ManualApprovalGate;
pub use comparator::ShadowComparator;