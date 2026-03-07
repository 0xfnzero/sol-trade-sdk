use anyhow::Result;
use solana_hash::Hash;
use solana_sdk::{
    instruction::Instruction, message::AddressLookupTableAccount, pubkey::Pubkey,
    signature::Keypair, signature::Signature,
};
use std::{sync::Arc, time::{Duration, Instant}};
#[allow(unused_imports)]
use tracing::{info, trace, warn};

use crate::{
    common::{nonce_cache::DurableNonceInfo, GasFeeStrategy, SolanaRpcClient},
    perf::syscall_bypass::SystemCallBypassManager,
    swqos::common::poll_any_transaction_confirmation,
    trading::core::{
        async_executor::execute_parallel,
        execution::{InstructionProcessor, Prefetch},
        traits::TradeExecutor,
    },
    trading::MiddlewareManager,
};
use once_cell::sync::Lazy;
use crate::swqos::TradeType;
use super::{params::SwapParams, traits::InstructionBuilder};

/// Global syscall bypass manager (reserved for future time/IO optimizations).
/// 全局系统调用绕过管理器（预留，后续可接入时间/IO 等优化）。
#[allow(dead_code)]
static SYSCALL_BYPASS: Lazy<SystemCallBypassManager> = Lazy::new(|| {
    use crate::perf::syscall_bypass::SyscallBypassConfig;
    SystemCallBypassManager::new(SyscallBypassConfig::default())
        .expect("Failed to create SystemCallBypassManager")
});

/// Generic trade executor implementation
pub struct GenericTradeExecutor {
    instruction_builder: Arc<dyn InstructionBuilder>,
    protocol_name: &'static str,
}

impl GenericTradeExecutor {
    pub fn new(
        instruction_builder: Arc<dyn InstructionBuilder>,
        protocol_name: &'static str,
    ) -> Self {
        Self { instruction_builder, protocol_name }
    }
}

#[async_trait::async_trait]
impl TradeExecutor for GenericTradeExecutor {
    async fn swap(&self, params: SwapParams) -> Result<(bool, Vec<Signature>, Option<anyhow::Error>)> {
        // Sample total start only when logging or simulate. 仅在有日志或 simulate 时取起点。
        let total_start = (params.log_enabled || params.simulate).then(Instant::now);
        let timing_start_us: Option<i64> = if params.log_enabled {
            Some(params.grpc_recv_us.unwrap_or_else(crate::common::clock::now_micros))
        } else {
            None
        };

        let is_buy = params.trade_type == TradeType::Buy || params.trade_type == TradeType::CreateAndBuy;

        Prefetch::keypair(&params.payer);

        // Time build only when log_enabled to avoid cold-path syscalls. 仅 log_enabled 时计时，减少冷路径 syscall。
        let build_start = params.log_enabled.then(Instant::now);
        let instructions = if is_buy {
            self.instruction_builder.build_buy_instructions(&params).await?
        } else {
            self.instruction_builder.build_sell_instructions(&params).await?
        };
        let _build_elapsed = build_start.map(|s| s.elapsed()).unwrap_or(Duration::ZERO);

        InstructionProcessor::preprocess(&instructions)?;

        let final_instructions = match &params.middleware_manager {
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name,
                    is_buy,
                )?,
            None => instructions,
        };

        let build_end_us = (params.log_enabled && crate::common::sdk_log::sdk_log_enabled())
            .then(crate::common::clock::now_micros);
        let _before_submit_elapsed = total_start.as_ref().map(|s| s.elapsed()).unwrap_or(Duration::ZERO);
        let before_submit_us = (params.log_enabled && crate::common::sdk_log::sdk_log_enabled())
            .then(crate::common::clock::now_micros);

        if params.simulate {
            let send_start = crate::common::sdk_log::sdk_log_enabled().then(Instant::now);
            let result = simulate_transaction(
                params.rpc,
                params.payer,
                final_instructions,
                params.address_lookup_table_account,
                params.recent_blockhash,
                params.durable_nonce,
                params.middleware_manager,
                self.protocol_name,
                is_buy,
                if is_buy { true } else { params.with_tip },
                params.gas_fee_strategy,
            )
            .await;
            let send_elapsed = send_start.map(|s| s.elapsed()).unwrap_or(Duration::ZERO);
            let total_elapsed = total_start.as_ref().map(|s| s.elapsed()).unwrap_or(Duration::ZERO);

            if crate::common::sdk_log::sdk_log_enabled() {
                let dir = if is_buy { "Buy" } else { "Sell" };
                if let (Some(start_us), Some(end_us)) = (timing_start_us, build_end_us) {
                    println!(" [SDK] {} build_instructions: {:.4} ms", dir, (end_us - start_us) as f64 / 1000.0);
                }
                if let (Some(start_us), Some(end_us)) = (timing_start_us, before_submit_us) {
                    println!(" [SDK] {} before_submit: {:.4} ms", dir, (end_us - start_us) as f64 / 1000.0);
                }
                println!(" [SDK] {} simulate (dry-run): {:.4} ms", dir, send_elapsed.as_secs_f64() * 1000.0);
                println!(" [SDK] {} total: {:.4} ms", dir, total_elapsed.as_secs_f64() * 1000.0);
            }

            return result;
        }

        let need_confirm = params.wait_transaction_confirmed;
        let result = execute_parallel(
            &params.swqos_clients,
            params.payer,
            params.rpc.clone(),
            final_instructions,
            params.address_lookup_table_account,
            params.recent_blockhash,
            params.durable_nonce,
            params.middleware_manager,
            self.protocol_name,
            is_buy,
            false, // submit only here; confirmation and log timing handled below
            if is_buy { true } else { params.with_tip },
            params.gas_fee_strategy,
            params.use_core_affinity,
            params.check_min_tip,
        )
        .await;

        let log_enabled = params.log_enabled && crate::common::sdk_log::sdk_log_enabled();

        let (ok, signatures, err, submit_timings) = match result {
            Ok((success, sigs, last_error, timings)) => (
                success,
                sigs,
                last_error.map(|e| anyhow::anyhow!("{}", e)),
                timings,
            ),
            Err(e) => (false, vec![], Some(anyhow::anyhow!("{}", e)), vec![]),
        };
        let submit_timings_ref: &[(crate::swqos::SwqosType, i64)] = submit_timings.as_slice();

        let result = if need_confirm {
            let confirm_result = if let Some(rpc) = params.rpc.as_ref() {
                if signatures.is_empty() {
                    (ok, signatures, err)
                } else {
                    let poll_res = poll_any_transaction_confirmation(rpc, &signatures, true).await;
                    let confirm_done_us = log_enabled.then(crate::common::clock::now_micros);
                    if log_enabled {
                        let dir = if is_buy { "Buy" } else { "Sell" };
                        if let Some(start_us) = timing_start_us {
                            if let Some(end_us) = build_end_us {
                                println!(" [SDK] {} build_instructions: {:.4} ms", dir, (end_us - start_us) as f64 / 1000.0);
                            }
                            if let Some(end_us) = before_submit_us {
                                println!(" [SDK] {} before_submit: {:.4} ms", dir, (end_us - start_us) as f64 / 1000.0);
                            }
                            if let Some(confirm_us) = confirm_done_us {
                                let total_ms = (confirm_us - start_us) as f64 / 1000.0;
                                for (swqos_type, submit_done_us) in submit_timings_ref {
                                    let submit_ms = (*submit_done_us - start_us).max(0) as f64 / 1000.0;
                                    let confirmed_ms = (confirm_us - *submit_done_us).max(0) as f64 / 1000.0;
                                    println!(" [SDK] {} {:?} submit: {:.4} ms, confirmed: {:.4} ms, total: {:.4} ms", dir, swqos_type, submit_ms, confirmed_ms, total_ms);
                                }
                            }
                        }
                    }
                    match poll_res {
                        Ok(_) => (true, signatures, None),
                        Err(e) => (false, signatures, Some(e)),
                    }
                }
            } else {
                (ok, signatures, err)
            };
            Ok(confirm_result)
        } else {
            if log_enabled {
                let dir = if is_buy { "Buy" } else { "Sell" };
                if let Some(start_us) = timing_start_us {
                    if let Some(end_us) = build_end_us {
                        println!(" [SDK] {} build_instructions: {:.4} ms", dir, (end_us - start_us) as f64 / 1000.0);
                    }
                    if let Some(end_us) = before_submit_us {
                        println!(" [SDK] {} before_submit: {:.4} ms", dir, (end_us - start_us) as f64 / 1000.0);
                    }
                    for (swqos_type, submit_done_us) in submit_timings_ref {
                        let submit_ms = (*submit_done_us - start_us).max(0) as f64 / 1000.0;
                        println!(" [SDK] {} {:?} submit: {:.4} ms, confirmed: -, total: {:.4} ms", dir, swqos_type, submit_ms, submit_ms);
                    }
                }
            }
            Ok((ok, signatures, err))
        };

        result
    }

    fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }
}

/// Simulate mode: single RPC simulation, returns Vec<Signature> for API consistency.
/// 模拟模式：单次 RPC 模拟，返回 Vec<Signature> 以与 API 一致。
async fn simulate_transaction(
    rpc: Option<Arc<SolanaRpcClient>>,
    payer: Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_account: Option<AddressLookupTableAccount>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    with_tip: bool,
    gas_fee_strategy: GasFeeStrategy,
) -> Result<(bool, Vec<Signature>, Option<anyhow::Error>)> {
    use crate::trading::common::build_transaction;
    use solana_client::rpc_config::RpcSimulateTransactionConfig;
    use solana_commitment_config::CommitmentLevel;
    use solana_transaction_status::UiTransactionEncoding;

    let rpc = rpc.ok_or_else(|| anyhow::anyhow!("RPC client is required for simulation"))?;

    // Get gas fee strategy for simulation (use Default swqos type)
    let trade_type =
        if is_buy { crate::swqos::TradeType::Buy } else { crate::swqos::TradeType::Sell };
    let gas_fee_configs = gas_fee_strategy.get_strategies(trade_type);

    let default_config = gas_fee_configs
        .iter()
        .find(|config| config.0 == crate::swqos::SwqosType::Default)
        .ok_or_else(|| anyhow::anyhow!("No default gas fee strategy found"))?;

    let tip = if with_tip { default_config.2.tip } else { 0.0 };
    let unit_limit = default_config.2.cu_limit;
    let unit_price = default_config.2.cu_price;

    let transaction = build_transaction(
        &payer,
        Some(&rpc),
        unit_limit,
        unit_price,
        &instructions,
        address_lookup_table_account.as_ref(),
        recent_blockhash,
        middleware_manager.as_ref(),
        protocol_name,
        is_buy,
        false,
        &Pubkey::default(),
        tip,
        durable_nonce.as_ref(),
    )
    .await?;

    // Simulate the transaction
    use solana_commitment_config::CommitmentConfig;
    let simulate_result = rpc
        .simulate_transaction_with_config(
            &transaction,
            RpcSimulateTransactionConfig {
                sig_verify: false,               // Don't verify signature during simulation for speed
                replace_recent_blockhash: false, // Use actual blockhash from transaction
                commitment: Some(CommitmentConfig {
                    commitment: CommitmentLevel::Processed, // Use Processed level to get latest state
                }),
                encoding: Some(UiTransactionEncoding::Base64), // Base64 encoding
                accounts: None,           // Don't return specific account states (can be specified if needed)
                min_context_slot: None,   // Don't specify minimum context slot
                inner_instructions: true, // Enable inner instructions for debugging and detailed execution flow
            },
        )
        .await?;

    let signature = transaction
        .signatures
        .first()
        .ok_or_else(|| anyhow::anyhow!("Transaction has no signatures"))?
        .clone();

    if let Some(err) = simulate_result.value.err {
        #[cfg(feature = "perf-trace")]
        {
            warn!(target: "sol_trade_sdk", "[Simulation Failed] error={:?} signature={:?}", err, signature);
            if let Some(logs) = &simulate_result.value.logs {
                trace!(target: "sol_trade_sdk", "Transaction logs: {:?}", logs);
            }
            if let Some(units_consumed) = simulate_result.value.units_consumed {
                trace!(target: "sol_trade_sdk", "Compute Units Consumed: {}", units_consumed);
            }
        }
        return Ok((false, vec![signature], Some(anyhow::anyhow!("{:?}", err))));
    }

    // Simulation succeeded
    #[cfg(feature = "perf-trace")]
    {
        info!(target: "sol_trade_sdk", "[Simulation Succeeded] signature={:?}", signature);
        if let Some(units_consumed) = simulate_result.value.units_consumed {
            trace!(target: "sol_trade_sdk", "Compute Units Consumed: {}", units_consumed);
        }
        if let Some(logs) = &simulate_result.value.logs {
            trace!(target: "sol_trade_sdk", "Transaction logs: {:?}", logs);
        }
    }

    Ok((true, vec![signature], None))
}

#[cfg(test)]
mod tests {
    use crate::swqos::SwqosType;

    /// 运行 `cargo test -p sol-trade-sdk log_timing_preview -- --nocapture` 查看日志打印效果
    #[test]
    fn log_timing_preview() {
        let dir = "Buy";
        let build_ms = 12.34;
        let before_submit_ms = 15.67;
        println!("\n--- 1. 构建指令耗时 / 提交前耗时（各打印一次，统一 ms，保留 4 位小数）---\n");
        println!(" [SDK] {} build_instructions: {:.4} ms", dir, build_ms);
        println!(" [SDK] {} before_submit: {:.4} ms", dir, before_submit_ms);

        println!("\n--- 2. 每个 SWQOS 独立耗时：submit=起点→该通道返回, confirmed=该通道提交→链上确认, total=起点→链上确认 ---\n");
        for (swqos_type, submit_ms, confirmed_ms, total_ms) in [
            (SwqosType::Jito, 45.12, 83.38, 128.50),
            (SwqosType::Helius, 52.30, 76.20, 128.50),
            (SwqosType::ZeroSlot, 48.90, 79.60, 128.50),
        ] {
            println!(
                " [SDK] {} {:?} submit: {:.4} ms, confirmed: {:.4} ms, total: {:.4} ms",
                dir, swqos_type, submit_ms, confirmed_ms, total_ms
            );
        }

        println!("\n--- 3. 不等待链上确认时：每行 total = 该通道 submit 耗时（独立）---\n");
        for (swqos_type, submit_ms, total_ms) in [
            (SwqosType::Jito, 44.20, 44.20),
            (SwqosType::Helius, 51.80, 51.80),
        ] {
            println!(
                " [SDK] {} {:?} submit: {:.4} ms, confirmed: -, total: {:.4} ms",
                dir, swqos_type, submit_ms, total_ms
            );
        }

        println!("\n--- 4. Simulate 模式（build/before_submit 仍从 grpc_recv_us 起算）---\n");
        println!(" [SDK] {} build_instructions: {:.4} ms", dir, build_ms);
        println!(" [SDK] {} before_submit: {:.4} ms", dir, before_submit_ms);
        println!(" [SDK] {} simulate (dry-run): {:.4} ms", dir, 8.50);
        println!(" [SDK] {} total: {:.4} ms", dir, 36.51);
        println!();
    }
}
