//! 并行执行器

use anyhow::{anyhow, Result};
use crossbeam_queue::ArrayQueue;
use solana_hash::Hash;
use solana_sdk::message::AddressLookupTableAccount;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signature::Signature,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{str::FromStr, sync::Arc, time::Instant};

use crate::{
    common::nonce_cache::DurableNonceInfo,
    common::{GasFeeStrategy, SolanaRpcClient},
    swqos::{SwqosClient, SwqosType, TradeType},
    trading::{common::build_transaction, MiddlewareManager},
    constants::swqos::{
        SWQOS_MIN_TIP_DEFAULT,
        SWQOS_MIN_TIP_JITO,
        SWQOS_MIN_TIP_NEXTBLOCK,
        SWQOS_MIN_TIP_ZERO_SLOT,
        SWQOS_MIN_TIP_TEMPORAL,
        SWQOS_MIN_TIP_BLOXROUTE,
        SWQOS_MIN_TIP_NODE1,
        SWQOS_MIN_TIP_FLASHBLOCK,
        SWQOS_MIN_TIP_BLOCKRAZOR,
        SWQOS_MIN_TIP_ASTRALANE,
        SWQOS_MIN_TIP_STELLIUM,
        SWQOS_MIN_TIP_LIGHTSPEED,
    },
};

#[repr(align(64))]
struct TaskResult {
    success: bool,
    signature: Signature,
    error: Option<anyhow::Error>,
    swqos_type: SwqosType,  // 🔧 增加：记录SWQOS类型
    landed_on_chain: bool,  // 🔧 Whether tx landed on-chain (even if failed)
}

/// Check if an error indicates the transaction landed on-chain (vs network/timeout error)
fn is_landed_error(error: &anyhow::Error) -> bool {
    use crate::swqos::common::TradeError;

    // If it's a TradeError with a non-zero code, the tx landed but failed on-chain
    if let Some(trade_error) = error.downcast_ref::<TradeError>() {
        // Code 500 with "timed out" message means tx never landed
        if trade_error.code == 500 && trade_error.message.contains("timed out") {
            return false;
        }
        // Any other TradeError means the tx landed (e.g., ExceededSlippage = 6004)
        return trade_error.code > 0;
    }

    // Check error message for timeout indication
    let msg = error.to_string();
    if msg.contains("timed out") || msg.contains("timeout") {
        return false;
    }

    // Assume other errors might indicate landed tx (be conservative)
    false
}

struct ResultCollector {
    results: Arc<ArrayQueue<TaskResult>>,
    success_flag: Arc<AtomicBool>,
    landed_failed_flag: Arc<AtomicBool>,  // 🔧 Tx landed on-chain but failed (nonce consumed)
    completed_count: Arc<AtomicUsize>,
    total_tasks: usize,
}

impl ResultCollector {
    fn new(capacity: usize) -> Self {
        Self {
            results: Arc::new(ArrayQueue::new(capacity)),
            success_flag: Arc::new(AtomicBool::new(false)),
            landed_failed_flag: Arc::new(AtomicBool::new(false)),
            completed_count: Arc::new(AtomicUsize::new(0)),
            total_tasks: capacity,
        }
    }

    fn submit(&self, result: TaskResult) {
        // 🚀 优化：ArrayQueue 内部已保证同步，无需额外 fence
        let is_success = result.success;
        let is_landed_failed = result.landed_on_chain && !result.success;

        let _ = self.results.push(result);

        if is_success {
            self.success_flag.store(true, Ordering::Release); // Release 确保 push 可见
        } else if is_landed_failed {
            // 🔧 Tx landed but failed (e.g., ExceededSlippage) - nonce is consumed, no point waiting
            self.landed_failed_flag.store(true, Ordering::Release);
        }

        self.completed_count.fetch_add(1, Ordering::Release);
    }

    async fn wait_for_success(&self) -> Option<(bool, Vec<Signature>, Option<anyhow::Error>)> {
        let start = Instant::now();
        let timeout = std::time::Duration::from_secs(30);

        loop {
            // 🚀 Acquire 确保看到 push 的内容
            if self.success_flag.load(Ordering::Acquire) {
                // 🔧 修复：收集所有签名
                let mut signatures = Vec::new();
                let mut has_success = false;
                while let Some(result) = self.results.pop() {
                    signatures.push(result.signature);
                    if result.success {
                        has_success = true;
                    }
                }
                if has_success && !signatures.is_empty() {
                    return Some((true, signatures, None));
                }
            }

            // 🔧 Early exit: if a tx landed but failed (e.g., ExceededSlippage),
            // nonce is consumed and other channels can't succeed - return immediately
            if self.landed_failed_flag.load(Ordering::Acquire) {
                let mut signatures = Vec::new();
                let mut landed_error = None;
                while let Some(result) = self.results.pop() {
                    signatures.push(result.signature);
                    // Prefer the error from the tx that actually landed
                    if result.landed_on_chain && result.error.is_some() {
                        landed_error = result.error;
                    }
                }
                if !signatures.is_empty() {
                    return Some((false, signatures, landed_error));
                }
            }

            let completed = self.completed_count.load(Ordering::Acquire);
            if completed >= self.total_tasks {
                // 🔧 修复：收集所有签名
                let mut signatures = Vec::new();
                let mut last_error = None;
                let mut any_success = false;
                while let Some(result) = self.results.pop() {
                    signatures.push(result.signature);
                    if result.success {
                        any_success = true;
                    }
                    if result.error.is_some() {
                        last_error = result.error;
                    }
                }
                if !signatures.is_empty() {
                    return Some((any_success, signatures, last_error));
                }
                return None;
            }

            if start.elapsed() > timeout {
                return None;
            }
            tokio::task::yield_now().await;
        }
    }

    fn get_first(&self) -> Option<(bool, Vec<Signature>, Option<anyhow::Error>)> {
        // 🔧 修复：收集已提交的所有签名
        let mut signatures = Vec::new();
        let mut has_success = false;
        let mut last_error = None;
        
        while let Some(result) = self.results.pop() {
            signatures.push(result.signature);
            if result.success {
                has_success = true;
            }
            if result.error.is_some() {
                last_error = result.error;
            }
        }
        
        if !signatures.is_empty() {
            Some((has_success, signatures, last_error))
        } else {
            None
        }
    }
}

/// 🔧 修复：返回Vec<Signature>支持多SWQOS并发交易
pub async fn execute_parallel(
    swqos_clients: Vec<Arc<SwqosClient>>,
    payer: Arc<Keypair>,
    rpc: Option<Arc<SolanaRpcClient>>,
    instructions: Vec<Instruction>,
    address_lookup_table_account: Option<AddressLookupTableAccount>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    data_size_limit: u32,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    with_tip: bool,
    gas_fee_strategy: GasFeeStrategy,
) -> Result<(bool, Vec<Signature>, Option<anyhow::Error>)> {
    let _exec_start = Instant::now();

    if swqos_clients.is_empty() {
        return Err(anyhow!("swqos_clients is empty"));
    }

    if !with_tip
        && swqos_clients
            .iter()
            .find(|swqos| matches!(swqos.get_swqos_type(), SwqosType::Default))
            .is_none()
    {
        return Err(anyhow!("No Rpc Default Swqos configured."));
    }

    let cores = core_affinity::get_core_ids().unwrap();
    let instructions = Arc::new(instructions);

    // 预先计算所有有效的组合
    let task_configs: Vec<_> = swqos_clients
        .iter()
        .enumerate()
        .filter(|(_, swqos_client)| {
            with_tip || matches!(swqos_client.get_swqos_type(), SwqosType::Default)
        })
        .flat_map(|(i, swqos_client)| {
            let gas_fee_strategy_configs = gas_fee_strategy.get_strategies(if is_buy {
                TradeType::Buy
            } else {
                TradeType::Sell
            });
            gas_fee_strategy_configs
                .into_iter()
                .filter(|config| config.0.eq(&swqos_client.get_swqos_type()))
                .filter(|config| {
                    // 当需要 tip 且不是 Default 时，按 provider 最低小费进行筛选
                    if with_tip && !matches!(config.0, SwqosType::Default) {
                        let min_tip = match config.0 {
                            SwqosType::Jito => SWQOS_MIN_TIP_JITO,
                            SwqosType::NextBlock => SWQOS_MIN_TIP_NEXTBLOCK,
                            SwqosType::ZeroSlot => SWQOS_MIN_TIP_ZERO_SLOT,
                            SwqosType::Temporal => SWQOS_MIN_TIP_TEMPORAL,
                            SwqosType::Bloxroute => SWQOS_MIN_TIP_BLOXROUTE,
                            SwqosType::Node1 => SWQOS_MIN_TIP_NODE1,
                            SwqosType::FlashBlock => SWQOS_MIN_TIP_FLASHBLOCK,
                            SwqosType::BlockRazor => SWQOS_MIN_TIP_BLOCKRAZOR,
                            SwqosType::Astralane => SWQOS_MIN_TIP_ASTRALANE,
                            SwqosType::Stellium => SWQOS_MIN_TIP_STELLIUM,
                            SwqosType::Lightspeed => SWQOS_MIN_TIP_LIGHTSPEED,
                            SwqosType::Default => SWQOS_MIN_TIP_DEFAULT,
                        };
                        if config.2.tip < min_tip {
                            println!(
                                "⚠️ Config filtered: {:?} tip {} is below minimum required tip {}",
                                config.0, config.2.tip, min_tip
                            );
                        }
                        config.2.tip >= min_tip
                    } else {
                        true
                    }
                })
                .map(move |config| (i, swqos_client.clone(), config))
        })
        .collect();

    if task_configs.is_empty() {
        return Err(anyhow!("No available gas fee strategy configs"));
    }

    if is_buy && task_configs.len() > 1 && durable_nonce.is_none() {
        return Err(anyhow!("Multiple swqos transactions require durable_nonce to be set.",));
    }

    // Task preparation completed

    let collector = Arc::new(ResultCollector::new(task_configs.len()));
    let _spawn_start = Instant::now();

    for (i, swqos_client, gas_fee_strategy_config) in task_configs {
        let core_id = cores[i % cores.len()];
        let payer = payer.clone();
        let instructions = instructions.clone();
        let middleware_manager = middleware_manager.clone();
        let swqos_type = swqos_client.get_swqos_type();
        let tip_account_str = swqos_client.get_tip_account()?;
        let tip_account = Arc::new(Pubkey::from_str(&tip_account_str).unwrap_or_default());
        let collector = collector.clone();

        let tip = gas_fee_strategy_config.2.tip;
        let unit_limit = gas_fee_strategy_config.2.cu_limit;
        let unit_price = gas_fee_strategy_config.2.cu_price;
        let rpc = rpc.clone();
        let durable_nonce = durable_nonce.clone();
        let address_lookup_table_account = address_lookup_table_account.clone();

        tokio::spawn(async move {
            let _task_start = Instant::now();
            core_affinity::set_for_current(core_id);

            let tip_amount = if with_tip { tip } else { 0.0 };

            let _build_start = Instant::now();
            let transaction = match build_transaction(
                payer,
                rpc,
                unit_limit,
                unit_price,
                instructions.as_ref().clone(),
                address_lookup_table_account,
                recent_blockhash,
                data_size_limit,
                middleware_manager,
                protocol_name,
                is_buy,
                swqos_type != SwqosType::Default,
                &tip_account,
                tip_amount,
                durable_nonce,
            )
            .await
            {
                Ok(tx) => tx,
                Err(e) => {
                    // Build transaction failed
                    collector.submit(TaskResult {
                        success: false,
                        signature: Signature::default(),
                        error: Some(e),
                        swqos_type,  // 🔧 记录SWQOS类型
                        landed_on_chain: false,  // Build failed, tx never sent
                    });
                    return;
                }
            };

            // Transaction built

            let _send_start = Instant::now();
            let mut err: Option<anyhow::Error> = None;
            let mut landed_on_chain = false;
            let success = match swqos_client
                .send_transaction(
                    if is_buy { TradeType::Buy } else { TradeType::Sell },
                    &transaction,
                )
                .await
            {
                Ok(()) => {
                    landed_on_chain = true;  // Success means tx confirmed on-chain
                    true
                }
                Err(e) => {
                    // Check if this error indicates the tx landed but failed (e.g., ExceededSlippage)
                    landed_on_chain = is_landed_error(&e);
                    err = Some(e);
                    // Send transaction failed
                    false
                }
            };

            // Transaction sent

            if let Some(signature) = transaction.signatures.first() {
                collector.submit(TaskResult {
                    success,
                    signature: *signature,
                    error: err,
                    swqos_type,  // 🔧 记录SWQOS类型
                    landed_on_chain,  // 🔧 Whether tx landed (even if it failed)
                });
            }
        });
    }

    // All tasks spawned

    if !wait_transaction_confirmed {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        if let Some(result) = collector.get_first() {
            return Ok(result);
        }
        return Err(anyhow!("No transaction signature available"));
    }

    if let Some(result) = collector.wait_for_success().await {
        Ok(result)
    } else {
        Err(anyhow!("All transactions failed"))
    }
}
