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
};

#[repr(align(64))]
struct TaskResult {
    success: bool,
    signature: Signature,
    _error: Option<anyhow::Error>,
}

struct ResultCollector {
    results: Arc<ArrayQueue<TaskResult>>,
    success_flag: Arc<AtomicBool>,
    completed_count: Arc<AtomicUsize>,
    total_tasks: usize,
}

impl ResultCollector {
    fn new(capacity: usize) -> Self {
        Self {
            results: Arc::new(ArrayQueue::new(capacity)),
            success_flag: Arc::new(AtomicBool::new(false)),
            completed_count: Arc::new(AtomicUsize::new(0)),
            total_tasks: capacity,
        }
    }

    fn submit(&self, result: TaskResult) {
        // 🚀 优化：ArrayQueue 内部已保证同步，无需额外 fence
        let is_success = result.success;

        let _ = self.results.push(result);

        if is_success {
            self.success_flag.store(true, Ordering::Release); // Release 确保 push 可见
        }

        self.completed_count.fetch_add(1, Ordering::Release);
    }

    async fn wait_for_success(&self) -> Option<(bool, Signature)> {
        let start = Instant::now();
        let timeout = std::time::Duration::from_secs(30);

        loop {
            // 🚀 Acquire 确保看到 push 的内容
            if self.success_flag.load(Ordering::Acquire) {
                while let Some(result) = self.results.pop() {
                    if result.success {
                        return Some((true, result.signature));
                    }
                }
            }

            let completed = self.completed_count.load(Ordering::Acquire);
            if completed >= self.total_tasks {
                while let Some(result) = self.results.pop() {
                    return Some((result.success, result.signature));
                }
                return None;
            }

            if start.elapsed() > timeout {
                return None;
            }
            tokio::task::yield_now().await;
        }
    }

    fn get_first(&self) -> Option<(bool, Signature)> {
        if let Some(result) = self.results.pop() {
            Some((result.success, result.signature))
        } else {
            None
        }
    }
}

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
) -> Result<(bool, Signature)> {
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
                .map(move |config| (i, swqos_client.clone(), config))
        })
        .collect();

    if task_configs.is_empty() {
        return Err(anyhow!("No available gas fee strategy configs"));
    }

    if task_configs.len() > 1 && durable_nonce.is_none() {
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
                        _error: Some(e),
                    });
                    return;
                }
            };

            // Transaction built

            let _send_start = Instant::now();
            let success = match swqos_client
                .send_transaction(
                    if is_buy { TradeType::Buy } else { TradeType::Sell },
                    &transaction,
                )
                .await
            {
                Ok(()) => true,
                Err(_e) => {
                    // Send transaction failed
                    false
                }
            };

            // Transaction sent

            if let Some(signature) = transaction.signatures.first() {
                collector.submit(TaskResult { success, signature: *signature, _error: None });
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
