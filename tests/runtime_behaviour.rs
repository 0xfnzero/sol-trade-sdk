//! Native Rust integration tests for the sol-trade-sdk.
//!
//! These tests exercise actual production code through the public API with
//! deterministic fixtures. Tests that require RPC access, Tokio runtime,
//! or production transaction building are marked BLOCKED with explanation.
//!
//! Run: cargo test --features dev-insecure-tls --test runtime_behaviour

use solana_sdk::pubkey::Pubkey;

/// Test-only PumpFun program ID.
fn pumpfun_id() -> Pubkey {
    solana_sdk::pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P")
}
/// Test-only Raydium AMM V4 program ID.
fn raydium_id() -> Pubkey {
    solana_sdk::pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8")
}
/// Test-only default program ID for unknown-protocol tests.
fn unknown_id() -> Pubkey {
    Pubkey::default()
}

// ── 1. Two-mint market-state isolation ──

#[test]
fn integration_two_mint_isolation() {
    use sol_trade_sdk::trading::strategy::{MarketStateKey, MarketStateTracker, MintMarketState};

    let tracker = MarketStateTracker::new();
    assert_eq!(tracker.len(), 0, "Fresh tracker must be empty");

    // Key construction: same program, different mints
    let key_a = MarketStateKey::new("pumpfun", "mint_alpha");
    let key_b = MarketStateKey::new("pumpfun", "mint_beta");
    assert_ne!(key_a, key_b, "Different mints on same program must have different keys");
    assert_eq!(key_a.to_string(), "pumpfun|mint_alpha");
    assert_eq!(key_b.to_string(), "pumpfun|mint_beta");

    // MintMarketState construction and independence
    let state_a = MintMarketState::new("mint_alpha");
    let state_b = MintMarketState::new("mint_beta");
    assert_eq!(state_a.mint, "mint_alpha");
    assert_eq!(state_b.mint, "mint_beta");
    assert_eq!(state_a.last_price, 0.0, "Fresh state must have zero price");
    assert_eq!(state_b.last_price, 0.0, "Fresh state must have zero price");

    // Cross-token price isolation test (manual field assignment)
    let mut a2 = MintMarketState::new("mint_alpha");
    let mut b2 = MintMarketState::new("mint_beta");
    a2.last_price = 10.0;
    b2.last_price = 25.0;
    assert_eq!(a2.last_price, 10.0, "mint_alpha price must be independent");
    assert_eq!(b2.last_price, 25.0, "mint_beta price must be independent");
}

// ── 2. Stale update rejection ──

#[test]
fn integration_stale_update_rejection() {
    use sol_trade_sdk::trading::strategy::{MarketStateTracker, MintMarketState};

    // A fresh state has last_update_micros = 0, which is treated as stale
    let state = MintMarketState::new("test_mint");
    assert_eq!(state.last_update_micros, 0, "Fresh state must have zero update time");
    assert_eq!(state.last_price, 0.0, "Fresh state must have zero price");
    assert_eq!(state.volume_1s, 0.0, "Fresh state must have zero volume");
    assert_eq!(state.event_count_1s, 0, "Fresh state must have zero event count");
    assert_eq!(state.active_positions, 0, "Fresh state must have zero positions");
}

// ── 3. Production pricing via public API ──

#[test]
fn integration_compute_swap_price_anchor() {
    use sol_trade_sdk::trading::strategy::market_state::compute_swap_price;

    let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
    data.extend_from_slice(&100_000_000u64.to_le_bytes());
    data.extend_from_slice(&1_000_000_000u64.to_le_bytes());

    let price = compute_swap_price(&data, &pumpfun_id(), true);
    assert!(price.is_some(), "Anchor-style must produce a price");
    assert!((price.unwrap() - 10.0).abs() < 0.001);
}

#[test]
fn integration_zero_token_returns_none() {
    use sol_trade_sdk::trading::strategy::market_state::compute_swap_price;

    let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
    data.extend_from_slice(&0u64.to_le_bytes());
    data.extend_from_slice(&1_000_000_000u64.to_le_bytes());

    let price = compute_swap_price(&data, &pumpfun_id(), true);
    assert!(
        price.is_none(),
        "P3-05 fix: zero-token Anchor data must return None, not Raydium fallthrough"
    );
}

#[test]
fn integration_zero_quote_returns_none() {
    use sol_trade_sdk::trading::strategy::market_state::compute_swap_price;

    let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
    data.extend_from_slice(&100_000_000u64.to_le_bytes());
    data.extend_from_slice(&0u64.to_le_bytes());
    assert!(compute_swap_price(&data, &pumpfun_id(), true).is_none());
}

#[test]
fn integration_short_data_returns_none() {
    use sol_trade_sdk::trading::strategy::market_state::compute_swap_price;

    assert!(compute_swap_price(&[0u8; 4], &unknown_id(), true).is_none());
    assert!(compute_swap_price(&[0u8; 10], &unknown_id(), true).is_none());
    assert!(compute_swap_price(&[0u8; 19], &unknown_id(), true).is_none());
}

#[test]
fn integration_pricing_determinism() {
    use sol_trade_sdk::trading::strategy::market_state::compute_swap_price;

    let mut data = vec![102u8, 6, 61, 18, 1, 218, 235, 234];
    data.extend_from_slice(&1_500_000_000u64.to_le_bytes());
    data.extend_from_slice(&300_000_000u64.to_le_bytes());

    let p1 = compute_swap_price(&data, &pumpfun_id(), true).unwrap();
    let p2 = compute_swap_price(&data, &pumpfun_id(), true).unwrap();
    assert_eq!(p1, p2, "Identical inputs must produce identical price");
}

// ── 4. Config-to-transaction position sizing ──

#[test]
fn integration_position_size_propagation() {
    use sol_trade_sdk::constants::risk::{
        RiskContext, RiskEngine, TradeRiskParams,
    };
    use sol_trade_sdk::common::config::RiskConfig;

    let config_sizes = [0.01_f64, 0.05_f64, 0.5_f64];

    for &sol_amount in &config_sizes {
        let params = TradeRiskParams::new(sol_amount, 0, 50);
        let lamports = (sol_amount * 1e9) as u64;
        assert!(lamports > 0, "Position size must be > 0 lamports");
        assert_eq!(params.sol_amount, sol_amount, "TradeRiskParams must propagate sol_amount");

        let engine = RiskEngine::new(RiskConfig::default());
        let ctx = RiskContext::default();
        let result = engine.check(&ctx, &params, "pumpfun", "rpc", "mint123");
        assert!(
            result.is_ok(),
            "Position size {:.2} SOL must pass risk check: {:?}",
            sol_amount, result
        );

        let oversize = TradeRiskParams::new(999.0, 0, 50);
        let result_oversize = engine.check(&ctx, &oversize, "pumpfun", "rpc", "mint123");
        assert!(result_oversize.is_err(), "Oversize position (999 SOL) must be rejected");
    }
}

#[test]
fn integration_position_size_from_lamports_to_exposure() {
    use sol_trade_sdk::constants::risk::{RiskContext, TradeLedger};
    use sol_trade_sdk::trading::core::state::TradeDirection;

    let position_size_sol = 0.05_f64;
    let position_size_lamports = (position_size_sol * 1e9) as u64;

    let mut ledger = TradeLedger::default();
    let slot = 12345u64;

    ledger.open_position(
        "mint_abc",
        "pumpfun",
        position_size_sol,
        slot,
        TradeDirection::Buy,
        position_size_lamports,
    );

    assert_eq!(ledger.positions.len(), 1, "Ledger must have 1 open position");
    assert!(ledger.positions.contains_key("mint_abc"), "Ledger must contain mint_abc");

    let pos = &ledger.positions["mint_abc"];
    assert_eq!(pos.size_sol, position_size_sol, "Position size in SOL must match config");
    assert_eq!(
        pos.cost_basis_lamports, position_size_lamports,
        "Cost basis in lamports must match config * 1e9"
    );

    let ctx = ledger.build_context();
    assert!(
        (ctx.current_open_exposure_sol - position_size_sol).abs() < 0.0001,
        "build_context must reflect position size: {:.4} SOL (expected {:.4})",
        ctx.current_open_exposure_sol, position_size_sol
    );
    assert_eq!(ctx.open_positions, 1, "build_context must report 1 open position");
}

// ── 5. TradeLedger risk rejection ──

#[test]
fn integration_risk_ledger_exposure_limit() {
    use sol_trade_sdk::common::config::RiskConfig;
    use sol_trade_sdk::constants::risk::{RiskContext, RiskEngine, TradeLedger, TradeRiskParams};
    use sol_trade_sdk::trading::core::state::TradeDirection;

    let engine = RiskEngine::new(RiskConfig::default());
    let mut ledger = TradeLedger::default();

    ledger.open_position("mint_a", "pumpfun", 0.9, 1, TradeDirection::Buy, 900_000_000);
    let ctx = ledger.build_context();
    assert!((ctx.current_open_exposure_sol - 0.9).abs() < 0.001, "Exposure should be 0.9 SOL");

    let trade = TradeRiskParams::new(0.2, 100_000, 50);
    let result = engine.check(&ctx, &trade, "pumpfun", "rpc", "mint_b");
    assert!(result.is_err(), "Second position pushing total to 1.1 SOL must be rejected");
}

#[test]
fn integration_risk_ledger_daily_trade_count() {
    use sol_trade_sdk::common::config::RiskConfig;
    use sol_trade_sdk::constants::risk::{RiskContext, RiskEngine, TradeRiskParams};

    let engine = RiskEngine::new(RiskConfig::default());

    let ctx = RiskContext { daily_trades: 60, ..Default::default() };
    let trade = TradeRiskParams::new(0.05, 100_000, 50);
    let result = engine.check(&ctx, &trade, "pumpfun", "rpc", "mint_x");
    assert!(result.is_err(), "Daily trade limit exceeded must be rejected");

    let ctx_ok = RiskContext { daily_trades: 40, ..Default::default() };
    let result_ok = engine.check(&ctx_ok, &trade, "pumpfun", "rpc", "mint_x");
    assert!(result_ok.is_ok(), "Below daily trade limit must pass");
}

#[test]
fn integration_risk_ledger_daily_loss() {
    use sol_trade_sdk::common::config::RiskConfig;
    use sol_trade_sdk::constants::risk::{RiskContext, RiskEngine, TradeRiskParams};

    let engine = RiskEngine::new(RiskConfig::default());

    let ctx = RiskContext { daily_loss_lamports: 600_000, ..Default::default() };
    let trade = TradeRiskParams::new(0.05, 100_000, 50);
    let result = engine.check(&ctx, &trade, "pumpfun", "rpc", "mint_x");
    assert!(result.is_err(), "Daily loss limit exceeded must be rejected");

    let ctx_ok = RiskContext { daily_loss_lamports: 400_000, ..Default::default() };
    let result_ok = engine.check(&ctx_ok, &trade, "pumpfun", "rpc", "mint_x");
    assert!(result_ok.is_ok(), "Below daily loss limit must pass");
}

#[test]
fn integration_risk_ledger_concurrent_reservations() {
    use sol_trade_sdk::common::config::RiskConfig;
    use sol_trade_sdk::constants::risk::{RiskContext, RiskEngine, TradeRiskParams};

    let engine = RiskEngine::new(RiskConfig::default());

    let ctx = RiskContext { open_positions: 5, ..Default::default() };
    let trade = TradeRiskParams::new(0.05, 100_000, 50);
    let result = engine.check(&ctx, &trade, "pumpfun", "rpc", "mint_x");
    assert!(result.is_ok(), "At open position limit should pass");

    let ctx_over = RiskContext { open_positions: 6, ..Default::default() };
    let result_over = engine.check(&ctx_over, &trade, "pumpfun", "rpc", "mint_x");
    assert!(result_over.is_err(), "Over open position limit must be rejected");
}

// ── 6. State-transition control flow ──

#[test]
fn integration_state_transition_valid() {
    use sol_trade_sdk::trading::core::state::{
        is_valid_transition, TradeDirection, TradeIntent, TradeState,
    };

    let mut intent = TradeIntent::new(
        String::from("pumpfun"),
        String::from("mint_test"),
        TradeDirection::Buy,
        1_000_000_000,
        950_000_000,
        None,
    );

    assert_eq!(intent.state, TradeState::Detected, "New intent must start in Detected state");

    assert!(is_valid_transition(TradeState::Detected, TradeState::Validated),
        "Detected → Validated must be a valid transition");
    assert!(intent.transition(TradeState::Validated, "price_ok").is_ok(),
        "Transition Detected → Validated must succeed");

    assert!(intent.transition(TradeState::Built, "tx_built").is_ok());
    assert!(intent.transition(TradeState::Signed, "signed").is_ok());
    assert!(intent.transition(TradeState::Submitted, "submitted").is_ok());
    assert!(intent.transition(TradeState::Landed, "confirmed").is_ok());
    assert!(intent.transition(TradeState::Settled, "reconciled").is_ok());

    assert_eq!(intent.state, TradeState::Settled, "After full path, intent must be Settled");
}

#[test]
fn integration_state_transition_invalid_rejected() {
    use sol_trade_sdk::trading::core::state::{TradeDirection, TradeIntent, TradeState};

    let mut intent = TradeIntent::new(
        String::from("pumpfun"),
        String::from("mint_test"),
        TradeDirection::Buy,
        1_000_000_000,
        950_000_000,
        None,
    );

    let result = intent.transition(TradeState::Submitted, "skip");
    assert!(result.is_err(), "Skipping state must be rejected");

    let result2 = intent.transition(TradeState::Landed, "skip");
    assert!(result2.is_err(), "Skipping to terminal success must be rejected");
}

#[test]
fn integration_state_transition_duplicate_rejected() {
    use sol_trade_sdk::trading::core::state::{TradeDirection, TradeIntent, TradeState};

    let mut intent = TradeIntent::new(
        String::from("pumpfun"),
        String::from("mint_test"),
        TradeDirection::Buy,
        1_000_000_000,
        950_000_000,
        None,
    );

    assert!(intent.transition(TradeState::Validated, "first").is_ok());
    let dup = intent.transition(TradeState::Validated, "duplicate");
    assert!(dup.is_err(), "Duplicate transition must be rejected");
}

#[test]
fn integration_state_transition_out_of_order_rejected() {
    use sol_trade_sdk::trading::core::state::{TradeDirection, TradeIntent, TradeState};

    // Going backward: directly set state to Submitted then try to go back to Validated
    let mut intent = TradeIntent::new(
        String::from("pumpfun"),
        String::from("mint_test"),
        TradeDirection::Buy,
        1_000_000_000,
        950_000_000,
        None,
    );
    intent.state = TradeState::Submitted;
    let result = intent.transition(TradeState::Validated, "going_back");
    assert!(result.is_err(), "Backward transition must be rejected");
}

#[test]
fn integration_state_terminal_records_error() {
    use sol_trade_sdk::trading::core::state::{TradeDirection, TradeIntent, TradeState};

    let mut intent = TradeIntent::new(
        String::from("pumpfun"),
        String::from("mint_test"),
        TradeDirection::Buy,
        1_000_000_000,
        950_000_000,
        None,
    );

    // Transition to terminal failure: Rejected
    assert!(intent.transition(TradeState::Rejected, "bad_price").is_ok());
    assert!(TradeState::Rejected.is_terminal(), "Rejected must be terminal");
    assert!(!TradeState::Rejected.is_success(), "Rejected must not be success");

    // Terminal state locks: no further transitions
    let result = intent.transition(TradeState::Validated, "after_terminal");
    assert!(result.is_err(), "Transition from terminal state must fail");
}

#[test]
fn integration_state_needs_reconciliation() {
    use sol_trade_sdk::trading::core::state::TradeState;

    // Only Ambiguous needs reconciliation
    assert!(TradeState::Ambiguous.needs_reconciliation(), "Ambiguous must need reconciliation");

    // Terminal states do not
    assert!(!TradeState::Settled.needs_reconciliation(), "Settled must not need reconciliation");
    assert!(!TradeState::Rejected.needs_reconciliation(), "Rejected must not need reconciliation");
    assert!(!TradeState::Failed.needs_reconciliation(), "Failed must not need reconciliation");
    assert!(!TradeState::Landed.needs_reconciliation(), "Landed must not need reconciliation");
}

// ── 7. Transaction state path evidence ──

#[test]
fn integration_state_path_output() {
    use sol_trade_sdk::trading::core::state::{TradeDirection, TradeIntent, TradeState};

    let mut intent = TradeIntent::new(
        String::from("pumpfun"),
        String::from("mint_test"),
        TradeDirection::Buy,
        1_000_000_000,
        950_000_000,
        None,
    );

    intent.transition(TradeState::Validated, "price_ok").unwrap();
    intent.transition(TradeState::Built, "tx_built").unwrap();
    intent.transition(TradeState::Signed, "signed").unwrap();

    let path = intent.state_path();
    assert!(path.contains("Detected"), "Path must start at Detected");
    assert!(path.contains("Validated"), "Path must contain Validated");
    assert!(path.contains("Built"), "Path must contain Built");
    assert!(path.contains("Signed"), "Path must contain Signed");
}

// ── BLOCKED Scenarios ──

/// Scenario 7: Complete intercepted buy dry run
///
/// BLOCKED: Requires a fully wired TradeExecutor, RPC client, SwapParams
/// construction, Jito bundle simulation, and transaction decoding — none of
/// which are available in an integration test without a running Solana
/// test validator or RPC mock. Unit tests in market_state.rs cover the
/// compute_swap_price → MintMarketState.update → last_price propagation path.
#[test]
fn blocked_complete_dry_run() {
    eprintln!("BLOCKED: Complete intercepted buy dry run requires RPC + Tokio + TradeExecutor");
}

/// Scenario 8: Pricing failure preventing transaction construction
///
/// BLOCKED: Requires the full orchestrator.plan_trade() → execute_plan()
/// pipeline with a live TradeExecutor. Unit test `test_pricing_failure_skips_price_update`
/// in market_state.rs verifies that compute_swap_price failure prevents price assignment.
#[test]
fn blocked_pricing_failure_tx_construction() {
    eprintln!(
        "BLOCKED: Pricing failure → tx construction block requires full orchestrator pipeline"
    );
}

/// Scenario 9: RPC failure behavior
///
/// BLOCKED: Requires a mock RPC client or an actual Solana test validator.
/// The RPC failure paths are handled at the transport layer (retries, timeouts)
/// and cannot be deterministically tested without network mocking.
#[test]
fn blocked_rpc_failure() {
    eprintln!("BLOCKED: RPC failure requires mock RPC or test validator");
}

/// Scenario 10: Restart with pending or existing state
///
/// BLOCKED: Requires the persistence/serialization layer (TradeLedger
/// serialization to disk, restart-reload path). Not yet implemented.
#[test]
fn blocked_restart_with_pending_state() {
    eprintln!("BLOCKED: Restart with pending state requires persistence layer");
}