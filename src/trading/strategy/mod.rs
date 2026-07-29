//! Strategy Engine module — multi-factor signal evaluation pipeline.
//!
//! The Strategy Engine sits between the ShredStream classifier (event input)
//! and the execution layer (Orchestrator). It:
//!
//! - Tracks per-mint market state (momentum, volume, spread, freshness)
//! - Evaluates multiple signal factors against the current state
//! - Applies gates (composite threshold, spread, stale data, event rate)
//! - Produces `StrategyOutcome` — either `Trade(TradeSignal)` or `NoTrade`
//!
//! Designed to be shared between Shadow Mode (observation) and Production
//! Mode (execution). The same engine feeds both paths.

pub mod engine;
pub mod factors;
pub mod market_state;
pub mod metrics;
pub mod momentum;
pub mod signal;

pub use engine::{GateConfig, StrategyConfig, StrategyEngine, StrategyStats};
pub use factors::{FactorConfig, FactorOutput, FactorWeights, SignalFactors};
pub use market_state::{MarketStateTracker, MintMarketState};
pub use momentum::{MomentumTracker, SlidingWindowMomentum};
pub use signal::{
    FactorContribution, NoTrade, SignalStrength, StrategyOutcome, TradeSignal,
};