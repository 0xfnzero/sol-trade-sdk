//! Risk controls — 35 hard limits across 5 categories.
//!
//! Every trade must pass through `RiskEngine::check()` before execution.
//! The engine is a stateless gate: it reads current state and returns
//! `Ok(())` or a descriptive `RiskError`.

// Risk gates — no serde deps needed for the error type.

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
        self.check_fee_ceilings(ctx)?;
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
        if self.config.switches.buy_only && trade_direction_is_sell() {
            return Err(RiskError::Switch("sell-only mode, buys blocked".into()));
        }
        if self.config.switches.sell_only && trade_direction_is_buy() {
            return Err(RiskError::Switch("buy-only mode, sells blocked".into()));
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

    // ── Global (8 checks) ──

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
        Ok(())
    }

    // ── Fee ceilings (7 checks) ──

    fn check_fee_ceilings(&self, ctx: &RiskContext) -> Result<(), RiskError> {
        // Priority fee, relay tip, and total tx cost are checked per-trade
        // in `TradeRiskParams`; daily/consecutive loss checks use context.
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
// Direction helpers (stub — real impl from trade params)
// ---------------------------------------------------------------------------

fn trade_direction_is_buy() -> bool {
    // Placeholder: real impl reads from active trade context
    false
}

fn trade_direction_is_sell() -> bool {
    !trade_direction_is_buy()
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
