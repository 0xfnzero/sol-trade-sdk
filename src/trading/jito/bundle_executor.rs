//! Trade execution loop — polls the strategy engine for signals, simulates bundles,
//! submits via Jito with strategy-aware tips, retries with escalation.
//!
//! # Architecture
//!
//! This is a background task spawned alongside the event-ingest loop in `solbot`.
//! It runs on a timer (default 200ms) and never blocks the event processing path.
//!
//! ```text
//! loop {
//!   sleep(200ms)
//!   skip if kill switch
//!   strategy_stats() — any signals?
//!   if signal_count > 0:
//!     lock engine, call evaluate()
//!     if Trade(signal):
//!       plan_trade()  ──→ 3 gates: kill → risk → strategy
//!       compute tip
//!       simulate bundle (dry-run)
//!       if simulation OK:
//!         execute_plan() with strategy-aware tip
//!         if nack && retries < max:
//!           tip *= escalation_factor
//!           retry
//!       record metrics
//! }
//! ```

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use solana_sdk::pubkey::Pubkey;
use std::time::{Duration, Instant};
use tracing::{info, trace, warn};

use crate::common::{config::AppConfig, gas_fee_strategy::GasFeeStrategy, SolanaRpcClient};
use crate::swqos::{SwqosClient, TradeType};
use crate::trading::core::params::BonkParams;
use crate::trading::core::{
    orchestrator::Orchestrator,
    params::{DexParamEnum, SwapParams},
    state::TradeDirection,
    traits::TradeExecutor,
};
use crate::trading::factory::{DexType, TradeFactory};
use crate::trading::jito::{BundleMetrics, BundleOutcome, TipCalculator, TipConfig};

/// Start the trade execution loop as a background task.
///
/// `target_mint` — the mint to trade (single-mint mode for now; extend to multi-mint later).
///
/// Returns a `tokio::task::JoinHandle` that can be awaited on shutdown.
pub fn spawn_trade_execution_loop(
    orchestrator: Arc<Orchestrator>,
    dex_type: DexType,
    target_mint: Pubkey,
    tip_config: TipConfig,
    swqos_clients: Arc<Vec<Arc<SwqosClient>>>,
    rpc: Arc<SolanaRpcClient>,
    app_config: AppConfig,
    running: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    let executor = TradeFactory::create_executor(dex_type);
    let pool_interval = Duration::from_millis(200);

    tokio::spawn(async move {
        info!(
            target: "sol_trade_sdk",
            "jito: trade execution loop started — polling every {}ms (mint={}, tip_strategy={})",
            pool_interval.as_millis(),
            target_mint,
            tip_config.strategy,
        );
        let position_size_sol = app_config.strategy.position_size_sol;
        let position_size_lamports = (position_size_sol * 1e9) as u64;

        let mut retry_state: Option<RetryState> = None;
        let mut last_eval_micros: i64 = 0;

        while running.load(Ordering::Acquire) {
            tokio::time::sleep(pool_interval).await;

            if orchestrator.is_killed() {
                retry_state = None;
                continue;
            }

            // ── Check if we have pending retries ──
            if let Some(retry) = &retry_state {
                if retry.attempts < tip_config.max_retries {
                    let outcome = execute_bundle_with_tip(
                        &orchestrator,
                        &*executor,
                        &target_mint,
                        &retry.direction,
                        &retry.protocol,
                        retry.input_amount,
                        retry.min_output_amount,
                        retry.detected_slot,
                        retry.escalated_tip,
                        &swqos_clients,
                        &rpc,
                        &tip_config,
                        position_size_sol,
                    )
                    .await;

                    match outcome {
                        BundleOutcome::Landed { tip_sol, landing_ms, attempts, .. } => {
                            BundleMetrics::landed(tip_sol, landing_ms, attempts);
                            info!(target: "sol_trade_sdk", "jito: bundle landed for {m} after {a} attempts — tip={t:.6} SOL landed_in={l}ms",
                                m = target_mint, a = attempts, t = tip_sol, l = landing_ms);
                            retry_state = None;
                        }
                        BundleOutcome::Nacked { tip_sol: _, attempts, .. } => {
                            BundleMetrics::nacked();
                            let new_attempts = attempts + 1;
                            let escalated = TipCalculator::compute_retry_tip(
                                retry.base_tip,
                                new_attempts,
                                tip_config.tip_escalation_factor,
                            );
                            warn!(target: "sol_trade_sdk",
                                "jito: bundle nack'd for {m} — retry {a}/{max} tip={t:.6} SOL",
                                m = target_mint, a = new_attempts, max = tip_config.max_retries, t = escalated);
                            retry_state = Some(RetryState {
                                attempts: new_attempts,
                                protocol: retry.protocol.clone(),
                                direction: retry.direction,
                                input_amount: retry.input_amount,
                                min_output_amount: retry.min_output_amount,
                                detected_slot: retry.detected_slot,
                                base_tip: retry.base_tip,
                                escalated_tip: escalated,
                            });
                        }
                        BundleOutcome::Exhausted { .. }
                        | BundleOutcome::SimulationFailed { .. } => {
                            warn!(target: "sol_trade_sdk",
                                "jito: bundle exhausted for {m} — giving up",
                                m = target_mint);
                            retry_state = None;
                        }
                        BundleOutcome::Rejected { .. } => {
                            retry_state = None;
                        }
                    }
                    continue;
                } else {
                    info!(target: "sol_trade_sdk",
                        "jito: max retries ({max}) exhausted for {m}",
                        max = tip_config.max_retries, m = target_mint);
                    retry_state = None;
                    continue;
                }
            }

            // ── Poll strategy engine for a new signal ──
            let signal = {
                let stats = orchestrator.strategy_stats();
                if stats.signal_count == 0 && retry_state.is_none() {
                    continue; // No signals pending
                }

                // Evaluate on the target mint
                let mint_str = target_mint.to_string();
                let protocol = "pumpfun"; // TODO: derive from config / multi-mint

                let mut engine = match orchestrator.strategy_engine.lock() {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let mid_price =
                    engine.market_state.find_by_mint(&mint_str).map(|s| s.last_price).unwrap_or(0.0);

                let now_micros = crate::common::fast_timing::fast_now_micros() as i64;

                // Time-gate: skip if too early
                if now_micros - last_eval_micros < 200_000 {
                    continue;
                }
                last_eval_micros = now_micros;

                let slot = crate::common::fast_timing::fast_now_micros() as u64; // placeholder slot

                match engine.evaluate(&mint_str, protocol, slot, mid_price, now_micros) {
                    Some(crate::trading::strategy::StrategyOutcome::Trade(s)) => Some(s),
                    _ => None,
                }
            };

            let signal = match signal {
                Some(s) => s,
                None => continue,
            };

            // ── Convert signal to TradeDirection ──
            let direction = match signal.strength.direction_sign() {
                1 => TradeDirection::Buy,
                -1 => TradeDirection::Sell,
                _ => continue, // neutral — skip
            };

            // ── Derive min output from swap-implied price and slippage ──
            // price = quote/token (SOL per token from instruction data)
            // For BUY: expected_tokens = position_size_sol / price
            // For SELL: expected_sol = token_amount * price
            // Apply 500 bps slippage as floor (configurable via P1-06 fix)
            let min_output_amount = if signal.midpoint_price > 0.0 {
                let expected_tokens = (position_size_sol / signal.midpoint_price) * 1e9; // token lamports
                (expected_tokens * (1.0 - 500.0f64 / 10000.0)) as u64
            } else {
                0u64
            };

            // ── Compute strategy-aware tip ──
            let tip_sol =
                TipCalculator::compute(signal.composite_score, signal.confidence, &tip_config);

            // ── Compute expected profit for profit/tip ratio gate ──
            // For buys: expected profit ~ price * amount * composite_score (simplified)
            // For sells: expected profit ~ price * amount * |composite_score|
            // NOTE: position_size_sol matches the actual input_amount used below.
            let expected_profit_sol = signal.midpoint_price
                * (signal.composite_score.abs() as f64)
                * position_size_sol;

            // ── Profit/tip ratio gate ──
            if tip_config.strategy == crate::trading::jito::TipStrategy::StrategyAware
                && expected_profit_sol > 0.0
                && tip_sol > 0.0
                && expected_profit_sol / tip_sol < tip_config.min_profit_to_tip_ratio
            {
                trace!(
                    target: "sol_trade_sdk",
                    "jito: skipping {m} — profit/tip ratio {r:.2} < min {min} (profit={p:.6} tip={t:.6})",
                    m = target_mint, r = expected_profit_sol / tip_sol,
                    min = tip_config.min_profit_to_tip_ratio,
                    p = expected_profit_sol, t = tip_sol,
                );
                continue;
            }

            // ── Simulate first (dry-run, no cost) ──
            if tip_config.enable_simulation_gate {
                let sim_ok = simulate_bundle(
                    &orchestrator,
                    &*executor,
                    &target_mint,
                    &direction,
                    &signal.protocol,
                    position_size_lamports, // from AppConfig.strategy.position_size_sol
                    min_output_amount,
                    None,
                    &swqos_clients,
                    &rpc,
                    position_size_sol,
                )
                .await;

                if !sim_ok {
                    BundleMetrics::sim_failed();
                    warn!(target: "sol_trade_sdk",
                        "jito: bundle simulation failed for {m} — skipping",
                        m = target_mint);
                    continue;
                }
            }

            // ── Execute with strategy-aware tip ──
            let input_amount = position_size_lamports; // from AppConfig.strategy.position_size_sol


            let outcome = execute_bundle_with_tip(
                &orchestrator,
                &*executor,
                &target_mint,
                &direction,
                &signal.protocol,
                input_amount,
                min_output_amount,
                None,
                tip_sol,
                &swqos_clients,
                &rpc,
                &tip_config,
                position_size_sol,
            )
            .await;

            match outcome {
                BundleOutcome::Landed { tip_sol, landing_ms, attempts, .. } => {
                    BundleMetrics::landed(tip_sol, landing_ms, attempts);
                    info!(target: "sol_trade_sdk",
                        "jito: bundle landed for {m} — tip={t:.6} SOL landed_in={l}ms attempts={a}",
                        m = target_mint, t = tip_sol, l = landing_ms, a = attempts);
                    retry_state = None;
                }
                BundleOutcome::Nacked { tip_sol, attempts, .. } => {
                    BundleMetrics::nacked();
                    let escalated = TipCalculator::compute_retry_tip(
                        tip_sol,
                        attempts + 1,
                        tip_config.tip_escalation_factor,
                    );
                    warn!(target: "sol_trade_sdk",
                        "jito: bundle nack'd for {m} — retry {a}/1 tip={t:.6} SOL",
                        m = target_mint, a = 1, t = escalated);
                    retry_state = Some(RetryState {
                        attempts: 1,
                        protocol: signal.protocol.clone(),
                        direction,
                        input_amount,
                        min_output_amount,
                        detected_slot: None,
                        base_tip: tip_sol,
                        escalated_tip: escalated,
                    });
                }
                BundleOutcome::Exhausted { .. } => {
                    warn!(target: "sol_trade_sdk",
                        "jito: bundle exhausted for {m} after retries",
                        m = target_mint);
                }
                BundleOutcome::SimulationFailed { reason } => {
                    warn!(target: "sol_trade_sdk",
                        "jito: simulation failed for {m}: {r}",
                        m = target_mint, r = reason);
                }
                BundleOutcome::Rejected { reason } => {
                    trace!(target: "sol_trade_sdk",
                        "jito: trade rejected for {m}: {r}",
                        m = target_mint, r = reason);
                }
            }
        }

        info!(target: "sol_trade_sdk", "jito: trade execution loop terminated");
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Internal retry state for a single bundle.
struct RetryState {
    attempts: u32,
    protocol: String,
    direction: TradeDirection,
    input_amount: u64,
    min_output_amount: u64,
    detected_slot: Option<u64>,
    base_tip: f64,
    escalated_tip: f64,
}

/// Simulate a bundle (dry-run, no cost).
/// Returns true if simulation succeeded.
async fn simulate_bundle(
    orchestrator: &Orchestrator,
    executor: &dyn TradeExecutor,
    mint: &Pubkey,
    direction: &TradeDirection,
    protocol: &str,
    input_amount: u64,
    min_output_amount: u64,
    detected_slot: Option<u64>,
    swqos_clients: &[Arc<SwqosClient>],
    rpc: &Arc<SolanaRpcClient>,
    position_size_sol: f64,
) -> bool {
    let plan = orchestrator
        .plan_trade(
            protocol,
            mint,
            *direction,
            input_amount,
            min_output_amount,
            detected_slot,
            &crate::constants::risk::TradeRiskParams::new(position_size_sol, 0, 500), // TODO: P1-10 — populate expected_output and net_profit from real quote
        )
        .await;

    let plan = match plan {
        Some(p) => p,
        None => return false,
    };

    // Build a simulation-only GasFeeStrategy (zero tip, no Jito)
    let sim_strategy = GasFeeStrategy::new();
    sim_strategy.set_global_fee_strategy(
        400_000, // buy cu_limit
        400_000, // sell cu_limit
        plan.cu_price,
        plan.cu_price,
        0.0, // no tip for simulation
        0.0,
    );
    sim_strategy.set_default_rpc_fee_strategy(400_000, 400_000, plan.cu_price, plan.cu_price);

    let swap_params = SwapParams {
        rpc: Some(rpc.clone()),
        payer: orchestrator.payer.clone(),
        trade_type: match direction {
            TradeDirection::Buy => TradeType::Buy,
            TradeDirection::Sell => TradeType::Sell,
        },
        input_mint: *mint,
        input_token_program: None,
        output_mint: solana_sdk::pubkey!("So11111111111111111111111111111111111111112"), // WSOL
        output_token_program: None,
        input_amount: Some(input_amount),
        slippage_basis_points: Some(500), // 5% slippage
        address_lookup_table_accounts: vec![],
        recent_blockhash: None,
        wait_tx_confirmed: false,
        protocol_params: DexParamEnum::Bonk(BonkParams::default()),
        open_seed_optimize: true,
        swqos_clients: Arc::new(swqos_clients.to_vec()),
        middleware_manager: None,
        durable_nonce: None,
        with_tip: false, // no tip for simulation
        create_input_mint_ata: false,
        close_input_mint_ata: false,
        create_output_mint_ata: false,
        close_output_mint_ata: false,
        fixed_output_amount: None,
        gas_fee_strategy: sim_strategy,
        simulate: true, // dry-run mode
        log_enabled: false,
        wait_for_all_submits: false,
        use_dedicated_sender_threads: false,
        sender_thread_cores: None,
        max_sender_concurrency: 1,
        effective_core_ids: Arc::new(vec![]),
        check_min_tip: false,
        grpc_recv_us: None,
        use_exact_sol_amount: None,
    };

    let result = orchestrator.execute_plan(plan, executor, swap_params).await;
    result.success
}

/// Execute a bundle with a specific tip amount.
async fn execute_bundle_with_tip(
    orchestrator: &Orchestrator,
    executor: &dyn TradeExecutor,
    mint: &Pubkey,
    direction: &TradeDirection,
    protocol: &str,
    input_amount: u64,
    min_output_amount: u64,
    detected_slot: Option<u64>,
    tip_sol: f64,
    swqos_clients: &[Arc<SwqosClient>],
    rpc: &Arc<SolanaRpcClient>,
    _tip_config: &TipConfig,
    position_size_sol: f64,
) -> BundleOutcome {
    BundleMetrics::submitted();

    let plan = match orchestrator
        .plan_trade(
            protocol,
            mint,
            *direction,
            input_amount,
            min_output_amount,
            detected_slot,
            &crate::constants::risk::TradeRiskParams::new(position_size_sol, 0, 500), // TODO: P1-10 — populate expected_output and net_profit from real quote
        )
        .await
    {
        Some(p) => p,
        None => {
            return BundleOutcome::Rejected {
                reason: "plan_trade returned None (kill switch, risk, or strategy gate)".into(),
            };
        }
    };

    // Build a GasFeeStrategy with the desired Jito tip
    let exec_strategy = GasFeeStrategy::new();
    exec_strategy.set_global_fee_strategy(
        400_000, // buy cu_limit
        400_000, // sell cu_limit
        plan.cu_price,
        plan.cu_price,
        tip_sol, // Jito tip (applied to all SWQOS providers)
        tip_sol,
    );
    exec_strategy.set_default_rpc_fee_strategy(400_000, 400_000, plan.cu_price, plan.cu_price);

    let start = Instant::now();
    let mut attempts = 0u32;

    let swap_params = SwapParams {
        rpc: Some(rpc.clone()),
        payer: orchestrator.payer.clone(),
        trade_type: match direction {
            TradeDirection::Buy => TradeType::Buy,
            TradeDirection::Sell => TradeType::Sell,
        },
        input_mint: *mint,
        input_token_program: None,
        output_mint: solana_sdk::pubkey!("So11111111111111111111111111111111111111112"), // WSOL
        output_token_program: None,
        input_amount: Some(input_amount),
        slippage_basis_points: Some(500), // 5% slippage
        address_lookup_table_accounts: vec![],
        recent_blockhash: None,
        wait_tx_confirmed: true, // Wait for confirmation
        protocol_params: DexParamEnum::Bonk(BonkParams::default()),
        open_seed_optimize: true,
        swqos_clients: Arc::new(swqos_clients.to_vec()),
        middleware_manager: None,
        durable_nonce: None,
        with_tip: true, // Enable tip injection
        create_input_mint_ata: false,
        close_input_mint_ata: false,
        create_output_mint_ata: false,
        close_output_mint_ata: false,
        fixed_output_amount: None,
        gas_fee_strategy: exec_strategy,
        simulate: false, // real execution
        log_enabled: false,
        wait_for_all_submits: false,
        use_dedicated_sender_threads: false,
        sender_thread_cores: None,
        max_sender_concurrency: 1,
        effective_core_ids: Arc::new(vec![]),
        check_min_tip: true,
        grpc_recv_us: None,
        use_exact_sol_amount: None,
    };

    let result = orchestrator.execute_plan(plan, executor, swap_params).await;
    attempts += 1;

    if result.success {
        let elapsed = start.elapsed().as_millis() as u64;
        BundleOutcome::Landed { tip_sol, landing_ms: elapsed, attempts, bundle_id: None }
    } else if result.signatures.is_empty() && result.error.is_some() {
        // No signatures = submission failed entirely (not just nack, but RPC/network error)
        BundleOutcome::Nacked {
            tip_sol,
            attempts,
            reason: result.error.unwrap_or_else(|| "unknown error".into()),
            bundle_id: None,
        }
    } else {
        // Signatures exist but confirmation failed = nack or timeout
        let reason = result.error.unwrap_or_else(|| "confirmation failed / nack".into());
        BundleOutcome::Nacked { tip_sol, attempts, reason, bundle_id: None }
    }
}