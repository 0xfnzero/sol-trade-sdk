//! Risk controls — 35 hard limits across 5 categories.
//!
//! Every trade must pass through `RiskEngine::check()` before execution.
//! The engine is a stateless gate: it reads current state and returns
//! `Ok(())` or a descriptive `RiskError`.

// Risk gates — no serde deps needed for the error type.

use crate::trading::core::state::TradeDirection;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// A descriptive risk gate failure.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RiskError {
    #[error("Per-trade limit: {0}")]
    PerTrade(String),

    #[error("Global limit: {0}")]
    Global(String),

    #[error("Fee ceiling: {0}")]
    FeeCeiling(String),

    #[error("Health check: {0}")]
    Health(String),

    #[error("Switch denied: {0}")]
    Switch(String),
}

// ---------------------------------------------------------------------------
// Risk engine
// ---------------------------------------------------------------------------

/// Snapshot of current state needed by the risk engine.
#[derive(Debug, Clone, Default)]
pub struct RiskContext {
    /// Current open exposure in SOL.
    pub current_open_exposure_sol: f64,
    /// Current exposure for a specific mint (SOL).
    pub current_mint_exposure_sol: f64,
    /// Current exposure for a specific protocol (SOL).
    pub current_protocol_exposure_sol: f64,
    /// Trades submitted in the last 60 seconds.
    pub trades_last_minute: u32,
    /// Failed trades in the last 5 minutes.
    pub failures_last_5min: u32,
    /// Total positions currently open.
    pub open_positions: u32,
    /// Positions open on a specific protocol.
    pub open_positions_protocol: u32,
    /// Trades today.
    pub daily_trades: u32,
    /// Daily loss in lamports.
    pub daily_loss_lamports: u64,
    /// Consecutive losses.
    pub consecutive_losses: u32,
    /// Current RPC slot lag.
    pub rpc_slot_lag: u64,
    /// Current packet loss percentage.
    pub packet_loss_pct: f64,
    /// Current queue delay in ms.
    pub queue_delay_ms: u64,
    /// Current blockhash age in ms.
    pub blockhash_age_ms: u64,
    /// Reconciliation backlog count.
    pub reconciliation_backlog: u32,
    /// Current shred loss percentage.
    pub shred_loss_pct: f64,
    /// Total net loss across last 100 trades in lamports.
    pub net_loss_last_100_trades_lamports: u64,
}

/// The risk engine applies all hard limits from configuration against
/// the current [`RiskContext`].
#[derive(Debug, Clone)]
pub struct RiskEngine {
    config: crate::common::config::RiskConfig,
}

impl RiskEngine {
    pub fn new(config: crate::common::config::RiskConfig) -> Self {
        Self { config }
    }

    /// Run all gates. Returns `Ok(())` if all pass, or the first failure.
    pub fn check(
        &self,
        ctx: &RiskContext,
        trade: &TradeRiskParams,
        protocol: &str,
        provider: &str,
        mint: &str,
    ) -> Result<(), RiskError> {
        self.check_switches(protocol, provider, mint)?;
        self.check_per_trade(trade)?;
        self.check_global(ctx)?;
        self.check_fee_ceilings(ctx, trade)?;
        self.check_health(ctx)?;
        Ok(())
    }

    /// Run all gates with trade direction awareness.
    /// Preferred over `check()` when direction is known.
    pub fn check_with_direction(
        &self,
        ctx: &RiskContext,
        trade: &TradeRiskParams,
        protocol: &str,
        provider: &str,
        mint: &str,
        direction: TradeDirection,
    ) -> Result<(), RiskError> {
        self.check_switches_with_direction(protocol, provider, mint, direction)?;
        self.check_per_trade(trade)?;
        self.check_global(ctx)?;
        self.check_fee_ceilings(ctx, trade)?;
        self.check_health(ctx)?;
        Ok(())
    }

    // ── Switches (fastest checks first) ──

    fn check_switches(&self, protocol: &str, provider: &str, mint: &str) -> Result<(), RiskError> {
        if !self.config.switches.is_allowed(protocol, provider, mint) {
            return Err(RiskError::Switch(format!(
                "blocked by switch: protocol={}, provider={}, mint={}",
                protocol, provider, mint
            )));
        }
        // buy_only/sell_only require TradeDirection — without it, skip direction checks
        Ok(())
    }

    fn check_switches_with_direction(
        &self,
        protocol: &str,
        provider: &str,
        mint: &str,
        direction: TradeDirection,
    ) -> Result<(), RiskError> {
        if !self.config.switches.is_allowed(protocol, provider, mint) {
            return Err(RiskError::Switch(format!(
                "blocked by switch: protocol={}, provider={}, mint={}",
                protocol, provider, mint
            )));
        }
        if self.config.switches.buy_only && direction == TradeDirection::Sell {
            return Err(RiskError::Switch(
                "buy-only mode, sells blocked".into(),
            ));
        }
        if self.config.switches.sell_only && direction == TradeDirection::Buy {
            return Err(RiskError::Switch(
                "sell-only mode, buys blocked".into(),
            ));
        }
        Ok(())
    }

    // ── Per-trade (7 checks) ──

    fn check_per_trade(&self, trade: &TradeRiskParams) -> Result<(), RiskError> {
        if trade.sol_amount > self.config.max_sol_per_trade {
            return Err(RiskError::PerTrade(format!(
                "max_sol_per_trade: {:.4} > limit {:.4}",
                trade.sol_amount, self.config.max_sol_per_trade
            )));
        }
        if trade.token_amount > self.config.max_token_amount_per_trade {
            return Err(RiskError::PerTrade(format!(
                "max_token_amount: {} > limit {}",
                trade.token_amount, self.config.max_token_amount_per_trade
            )));
        }
        if trade.slippage_basis_points > self.config.max_slippage_basis_points {
            return Err(RiskError::PerTrade(format!(
                "slippage: {} bps > limit {} bps",
                trade.slippage_basis_points, self.config.max_slippage_basis_points
            )));
        }
        if trade.quote_age_ms > self.config.max_quote_age_ms {
            return Err(RiskError::PerTrade(format!(
                "quote age: {} ms > limit {} ms",
                trade.quote_age_ms, self.config.max_quote_age_ms
            )));
        }
        if trade.source_slot_age > self.config.max_source_slot_age {
            return Err(RiskError::PerTrade(format!(
                "source slot age: {} > limit {}",
                trade.source_slot_age, self.config.max_source_slot_age
            )));
        }
        if trade.expected_output < self.config.min_expected_output {
            return Err(RiskError::PerTrade(format!(
                "expected output: {} < min {}",
                trade.expected_output, self.config.min_expected_output
            )));
        }
        if trade.net_profit_lamports < self.config.min_expected_net_profit_lamports {
            return Err(RiskError::PerTrade(format!(
                "net profit: {} lamports < min {}",
                trade.net_profit_lamports, self.config.min_expected_net_profit_lamports
            )));
        }
        Ok(())
    }

    // ── Global (9 checks) ──

    fn check_global(&self, ctx: &RiskContext) -> Result<(), RiskError> {
        if ctx.current_open_exposure_sol > self.config.global.max_open_exposure_sol {
            return Err(RiskError::Global(format!(
                "open exposure: {:.4} SOL > limit {:.4} SOL",
                ctx.current_open_exposure_sol, self.config.global.max_open_exposure_sol
            )));
        }
        if ctx.current_mint_exposure_sol > self.config.global.per_mint_cap_sol {
            return Err(RiskError::Global(format!(
                "mint exposure: {:.4} SOL > per-mint cap {:.4} SOL",
                ctx.current_mint_exposure_sol, self.config.global.per_mint_cap_sol
            )));
        }
        if ctx.trades_last_minute > self.config.global.max_tx_per_minute {
            return Err(RiskError::Global(format!(
                "tx/min: {} > limit {}",
                ctx.trades_last_minute, self.config.global.max_tx_per_minute
            )));
        }
        if ctx.failures_last_5min > self.config.global.max_failures_per_5min {
            return Err(RiskError::Global(format!(
                "failures/5min: {} > limit {}",
                ctx.failures_last_5min, self.config.global.max_failures_per_5min
            )));
        }
        if ctx.open_positions > self.config.global.max_open_positions {
            return Err(RiskError::Global(format!(
                "open positions: {} > max {}",
                ctx.open_positions, self.config.global.max_open_positions
            )));
        }
        if ctx.open_positions_protocol > self.config.global.max_positions_per_protocol {
            return Err(RiskError::Global(format!(
                "positions on protocol: {} > limit {}",
                ctx.open_positions_protocol, self.config.global.max_positions_per_protocol
            )));
        }
        if ctx.daily_trades > self.config.global.max_daily_trades {
            return Err(RiskError::Global(format!(
                "daily trades: {} > limit {}",
                ctx.daily_trades, self.config.global.max_daily_trades
            )));
        }
        // Check max_exposure_per_mint_pct against current_mint_exposure_sol
        let mint_exposure_pct = if ctx.current_open_exposure_sol > 0.0 {
            (ctx.current_mint_exposure_sol / ctx.current_open_exposure_sol) * 100.0
        } else {
            0.0
        };
        if mint_exposure_pct > self.config.global.max_exposure_per_mint_pct {
            return Err(RiskError::Global(format!(
                "mint exposure: {:.2}% of total > limit {:.2}%",
                mint_exposure_pct, self.config.global.max_exposure_per_mint_pct
            )));
        }
        // Check max_exposure_per_protocol_pct against current_protocol_exposure_sol
        let protocol_exposure_pct = if ctx.current_open_exposure_sol > 0.0 {
            (ctx.current_protocol_exposure_sol / ctx.current_open_exposure_sol) * 100.0
        } else {
            0.0
        };
        if protocol_exposure_pct > self.config.global.max_exposure_per_protocol_pct {
            return Err(RiskError::Global(format!(
                "protocol exposure: {:.2}% of total > limit {:.2}%",
                protocol_exposure_pct, self.config.global.max_exposure_per_protocol_pct
            )));
        }
        Ok(())
    }

    // ── Fee ceilings (7 checks) ──

    fn check_fee_ceilings(&self, ctx: &RiskContext, trade: &TradeRiskParams) -> Result<(), RiskError> {
        if trade.fee_cost_lamports > self.config.fee_ceilings.max_priority_fee_lamports {
            return Err(RiskError::FeeCeiling(format!(
                "priority fee: {} lamports > limit {}",
                trade.fee_cost_lamports, self.config.fee_ceilings.max_priority_fee_lamports
            )));
        }
        if trade.fee_cost_lamports > self.config.fee_ceilings.max_relay_tip_lamports {
            return Err(RiskError::FeeCeiling(format!(
                "relay tip: {} lamports > limit {}",
                trade.fee_cost_lamports, self.config.fee_ceilings.max_relay_tip_lamports
            )));
        }
        if trade.fee_cost_lamports > self.config.fee_ceilings.max_total_tx_cost_lamports {
            return Err(RiskError::FeeCeiling(format!(
                "total tx cost: {} lamports > limit {}",
                trade.fee_cost_lamports, self.config.fee_ceilings.max_total_tx_cost_lamports
            )));
        }
        if ctx.daily_loss_lamports > self.config.fee_ceilings.max_daily_loss_lamports {
            return Err(RiskError::FeeCeiling(format!(
                "daily loss: {} lamports > limit {}",
                ctx.daily_loss_lamports, self.config.fee_ceilings.max_daily_loss_lamports
            )));
        }
        if ctx.consecutive_losses > self.config.fee_ceilings.max_consecutive_losses {
            return Err(RiskError::FeeCeiling(format!(
                "consecutive losses: {} > limit {}",
                ctx.consecutive_losses, self.config.fee_ceilings.max_consecutive_losses
            )));
        }
        if trade.net_profit_lamports < self.config.fee_ceilings.max_net_loss_per_trade_lamports
            && trade.net_profit_lamports < self.config.min_expected_net_profit_lamports
        {
            return Err(RiskError::FeeCeiling(format!(
                "net loss per trade: {} lamports > limit {}",
                trade.net_profit_lamports, self.config.fee_ceilings.max_net_loss_per_trade_lamports
            )));
        }
        // Check max_loss_rate_per_100_trades_pct using net_loss_last_100_trades_lamports
        let avg_trade_size = if trade.sol_amount > 0.0 { trade.sol_amount } else { 1.0 };
        let loss_rate_pct = (ctx.net_loss_last_100_trades_lamports as f64)
            / (avg_trade_size * 1e9 * 100.0)
            * 100.0;
        if loss_rate_pct > self.config.fee_ceilings.max_loss_rate_per_100_trades_pct {
            return Err(RiskError::FeeCeiling(format!(
                "loss rate last 100 trades: {:.2}% > limit {:.2}%",
                loss_rate_pct, self.config.fee_ceilings.max_loss_rate_per_100_trades_pct
            )));
        }
        Ok(())
    }

    // ── Health (6 checks) ──

    fn check_health(&self, ctx: &RiskContext) -> Result<(), RiskError> {
        if ctx.rpc_slot_lag > self.config.health.max_rpc_slot_lag {
            return Err(RiskError::Health(format!(
                "RPC slot lag: {} > limit {}",
                ctx.rpc_slot_lag, self.config.health.max_rpc_slot_lag
            )));
        }
        if ctx.packet_loss_pct > self.config.health.max_packet_loss_pct {
            return Err(RiskError::Health(format!(
                "packet loss: {:.2}% > limit {:.2}%",
                ctx.packet_loss_pct, self.config.health.max_packet_loss_pct
            )));
        }
        if ctx.queue_delay_ms > self.config.health.max_queue_delay_ms {
            return Err(RiskError::Health(format!(
                "queue delay: {} ms > limit {} ms",
                ctx.queue_delay_ms, self.config.health.max_queue_delay_ms
            )));
        }
        if ctx.blockhash_age_ms > self.config.health.max_blockhash_age_ms {
            return Err(RiskError::Health(format!(
                "blockhash age: {} ms > limit {} ms",
                ctx.blockhash_age_ms, self.config.health.max_blockhash_age_ms
            )));
        }
        if ctx.reconciliation_backlog > self.config.health.max_reconciliation_backlog {
            return Err(RiskError::Health(format!(
                "reconciliation backlog: {} > limit {}",
                ctx.reconciliation_backlog, self.config.health.max_reconciliation_backlog
            )));
        }
        if ctx.shred_loss_pct > self.config.health.max_shred_loss_pct {
            return Err(RiskError::Health(format!(
                "shred loss: {:.2}% > limit {:.2}%",
                ctx.shred_loss_pct, self.config.health.max_shred_loss_pct
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Per-trade risk parameters
// ---------------------------------------------------------------------------

/// Per-trade parameters submitted to the risk engine.
#[derive(Debug, Clone)]
pub struct TradeRiskParams {
    pub sol_amount: f64,
    pub token_amount: u64,
    pub slippage_basis_points: u64,
    pub quote_age_ms: u64,
    pub source_slot_age: u64,
    pub expected_output: u64,
    pub net_profit_lamports: u64,
    /// Fee cost for this trade (lamports).
    pub fee_cost_lamports: u64,
}

impl TradeRiskParams {
    pub fn new(sol_amount: f64, token_amount: u64, slippage_basis_points: u64) -> Self {
        Self {
            sol_amount,
            token_amount,
            slippage_basis_points,
            quote_age_ms: 0,
            source_slot_age: 0,
            expected_output: 0,
            net_profit_lamports: 0,
            fee_cost_lamports: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// TradeLedger — tracks live open positions, daily trade counts, and P&L
// ---------------------------------------------------------------------------

use std::collections::HashMap;

/// A single open position tracked by the ledger.
#[derive(Debug, Clone)]
pub struct OpenPosition {
    /// The mint address of the token.
    pub mint: String,
    /// Protocol this position was opened on.
    pub protocol: String,
    /// Position size in SOL.
    pub size_sol: f64,
    /// Slot at which the position was opened.
    pub open_slot: u64,
    /// Direction of the position.
    pub direction: crate::trading::core::state::TradeDirection,
    /// Cost basis in lamports of SOL.
    pub cost_basis_lamports: u64,
}

/// Runtime trade ledger that feeds [`RiskContext`] before every risk check.
///
/// # Thread safety
/// Wrapped behind `Mutex<TradeLedger>` in the orchestrator — all mutation is
/// single-threaded so no interior complexity needed.
#[derive(Debug, Clone)]
pub struct TradeLedger {
    /// Open positions keyed by mint.
    pub positions: HashMap<String, OpenPosition>,
    /// Trades executed today (resets daily).
    pub daily_trade_count: u32,
    /// Daily P&L in lamports (positive = profit, negative = loss).
    pub daily_pnl_lamports: i64,
    /// Total net loss across the last 100 trades in lamports.
    pub net_loss_last_100_trades_lamports: u64,
    /// Circular buffer of recent trade P&L values (lamports, negative = loss).
    pub recent_pnl_window: Vec<i64>,
    /// Consecutive losing trades.
    pub consecutive_losses: u32,
    /// Last daily reset timestamp (Unix micros).
    pub last_reset_micros: u64,
}

impl Default for TradeLedger {
    fn default() -> Self {
        Self {
            positions: HashMap::new(),
            daily_trade_count: 0,
            daily_pnl_lamports: 0,
            net_loss_last_100_trades_lamports: 0,
            recent_pnl_window: Vec::with_capacity(100),
            consecutive_losses: 0,
            last_reset_micros: crate::common::fast_timing::fast_now_micros(),
        }
    }
}

impl TradeLedger {
    /// Reset daily counters if a new day has started.
    pub fn daily_reset_if_needed(&mut self) {
        let now = crate::common::fast_timing::fast_now_micros();
        // 24 hours in micros
        const DAY_MICROS: u64 = 86_400_000_000;
        if now.saturating_sub(self.last_reset_micros) >= DAY_MICROS {
            self.daily_trade_count = 0;
            self.daily_pnl_lamports = 0;
            self.last_reset_micros = now;
        }
    }

    /// Record a new open position.
    pub fn open_position(
        &mut self,
        mint: &str,
        protocol: &str,
        size_sol: f64,
        slot: u64,
        direction: crate::trading::core::state::TradeDirection,
        cost_basis_lamports: u64,
    ) {
        self.positions.insert(
            mint.to_string(),
            OpenPosition {
                mint: mint.to_string(),
                protocol: protocol.to_string(),
                size_sol,
                open_slot: slot,
                direction,
                cost_basis_lamports,
            },
        );
        self.daily_reset_if_needed();
        self.daily_trade_count += 1;
    }

    /// Record a closed position and its P&L impact.
    pub fn close_position(&mut self, mint: &str, profit_lamports: i64) -> Option<OpenPosition> {
        let pos = self.positions.remove(mint)?;
        self.daily_reset_if_needed();
        self.daily_trade_count += 1;
        self.daily_pnl_lamports += profit_lamports;

        // Track net loss for the last-100-trades risk gate
        if profit_lamports < 0 {
            self.consecutive_losses += 1;
            let loss = (-profit_lamports) as u64;
            self.net_loss_last_100_trades_lamports += loss;
        } else {
            self.consecutive_losses = 0;
        }

        // Maintain sliding window of 100 trades
        self.recent_pnl_window.push(profit_lamports);
        if self.recent_pnl_window.len() > 100 {
            // Remove the oldest entry from net loss tracking
            let oldest = self.recent_pnl_window.remove(0);
            if oldest < 0 {
                self.net_loss_last_100_trades_lamports =
                    self.net_loss_last_100_trades_lamports.saturating_sub((-oldest) as u64);
            }
        }

        Some(pos)
    }

    /// Record a failed trade (no position was opened).
    pub fn record_failed_trade(&mut self) {
        self.daily_reset_if_needed();
        self.daily_trade_count += 1;
    }

    /// Build a [`RiskContext`] snapshot from the current ledger state.
    pub fn build_context(&self) -> RiskContext {
        let mut open_exposure_sol = 0.0_f64;
        let mut positions_by_protocol: HashMap<&str, u32> = HashMap::new();
        let mut exposure_by_mint: HashMap<&str, f64> = HashMap::new();
        let mut exposure_by_protocol: HashMap<&str, f64> = HashMap::new();

        for pos in self.positions.values() {
            open_exposure_sol += pos.size_sol;
            *positions_by_protocol.entry(&pos.protocol).or_insert(0) += 1;
            *exposure_by_mint.entry(&pos.mint).or_insert(0.0) += pos.size_sol;
            *exposure_by_protocol.entry(&pos.protocol).or_insert(0.0) += pos.size_sol;
        }

        // Pick the highest mint/protocol exposure for context limits
        let current_mint_exposure_sol = exposure_by_mint.values().copied().fold(0.0_f64, f64::max);
        let current_protocol_exposure_sol =
            exposure_by_protocol.values().copied().fold(0.0_f64, f64::max);
        let max_protocol_positions = positions_by_protocol.values().copied().max().unwrap_or(0);

        RiskContext {
            current_open_exposure_sol: open_exposure_sol,
            current_mint_exposure_sol,
            current_protocol_exposure_sol,
            trades_last_minute: self.daily_trade_count.min(u32::MAX as u32) as u32,
            failures_last_5min: 0, // Tracked externally or by reconciliation
            open_positions: self.positions.len() as u32,
            open_positions_protocol: max_protocol_positions,
            daily_trades: self.daily_trade_count,
            daily_loss_lamports: if self.daily_pnl_lamports < 0 {
                (-self.daily_pnl_lamports) as u64
            } else {
                0
            },
            consecutive_losses: self.consecutive_losses,
            rpc_slot_lag: 0,      // Populated externally by health monitor
            packet_loss_pct: 0.0,  // Populated externally by health monitor
            queue_delay_ms: 0,     // Populated externally by health monitor
            blockhash_age_ms: 0,   // Populated externally by health monitor
            reconciliation_backlog: 0, // Populated externally by reconciliation service
            shred_loss_pct: 0.0,   // Populated externally by health monitor
            net_loss_last_100_trades_lamports: self.net_loss_last_100_trades_lamports,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::config::RiskConfig;

    fn default_engine() -> RiskEngine {
        RiskEngine::new(RiskConfig::default())
    }

    fn passing_trade() -> TradeRiskParams {
        TradeRiskParams {
            sol_amount: 0.05,
            token_amount: 100_000,
            slippage_basis_points: 50,
            quote_age_ms: 100,
            source_slot_age: 1,
            expected_output: 99_000,
            net_profit_lamports: 1_000,
            fee_cost_lamports: 5_000,
        }
    }

    fn passing_context() -> RiskContext {
        RiskContext::default()
    }

    #[test]
    fn happy_path_passes() {
        let engine = default_engine();
        let result =
            engine.check(&passing_context(), &passing_trade(), "pumpfun", "rpc", "mint123");
        assert!(result.is_ok(), "all gates should pass: {:?}", result);
    }

    #[test]
    fn per_trade_sol_amount_exceeded() {
        let engine = default_engine();
        let trade = TradeRiskParams { sol_amount: 999.0, ..passing_trade() };
        let result = engine.check(&passing_context(), &trade, "pumpfun", "rpc", "x");
        assert!(matches!(result, Err(RiskError::PerTrade(_))));
    }

    #[test]
    fn per_trade_slippage_exceeded() {
        let engine = default_engine();
        let trade = TradeRiskParams { slippage_basis_points: 5000, ..passing_trade() };
        let result = engine.check(&passing_context(), &trade, "pumpfun", "rpc", "x");
        assert!(matches!(result, Err(RiskError::PerTrade(_))));
    }

    #[test]
    fn global_open_exposure_exceeded() {
        let engine = default_engine();
        let ctx = RiskContext { current_open_exposure_sol: 5.0, ..Default::default() };
        let result = engine.check(&ctx, &passing_trade(), "pumpfun", "rpc", "x");
        assert!(matches!(result, Err(RiskError::Global(_))));
    }

    #[test]
    fn global_tx_rate_exceeded() {
        let engine = default_engine();
        let ctx = RiskContext { trades_last_minute: 100, ..Default::default() };
        let result = engine.check(&ctx, &passing_trade(), "pumpfun", "rpc", "x");
        assert!(matches!(result, Err(RiskError::Global(_))));
    }

    #[test]
    fn health_rpc_lag_exceeded() {
        let engine = default_engine();
        let ctx = RiskContext { rpc_slot_lag: 50, ..Default::default() };
        let result = engine.check(&ctx, &passing_trade(), "pumpfun", "rpc", "x");
        assert!(matches!(result, Err(RiskError::Health(_))));
    }

    #[test]
    fn switch_global_kill_blocks() {
        let mut cfg = RiskConfig::default();
        cfg.switches.global_kill = true;
        let engine = RiskEngine::new(cfg);
        let result = engine.check(&passing_context(), &passing_trade(), "pumpfun", "rpc", "x");
        assert!(matches!(result, Err(RiskError::Switch(_))));
    }

    #[test]
    fn daily_loss_exceeded() {
        let engine = default_engine();
        let ctx = RiskContext { daily_loss_lamports: 1_000_000, ..Default::default() };
        let result = engine.check(&ctx, &passing_trade(), "pumpfun", "rpc", "x");
        assert!(matches!(result, Err(RiskError::FeeCeiling(_))));
    }

    #[test]
    fn consecutive_losses_exceeded() {
        let engine = default_engine();
        let ctx = RiskContext { consecutive_losses: 10, ..Default::default() };
        let result = engine.check(&ctx, &passing_trade(), "pumpfun", "rpc", "x");
        assert!(matches!(result, Err(RiskError::FeeCeiling(_))));
    }
}
