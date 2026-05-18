//! Parallel executor for multi-SWQOS submit.
//!
//! **Hot path (submit):** no lock (OnceCell + lock-free ArrayQueue), no `get_core_ids()`, only Arc clones and queue push.
//! - **Pool**: Pre-spawned workers (default 18); hot path only enqueues jobs (no per-call tokio::spawn).
//! - **Dedicated threads** (opt-in via `with_dedicated_sender_threads`): N OS threads run sender work only, optionally pinned to cores.
//! - **Arc**: Shared data behind `Arc` → clone = refcount increment (no data copy).
//! - **Refs**: `build_transaction` takes refs only; worker path avoids extra clones.
//!
//! **Core affinity & latency:** Each job is assigned a core (round-robin from `effective_core_ids`). When a worker runs a job,
//! it sets thread affinity to that core. If that core is busy with other work (e.g. node sync, bot logic), SWQOS submit on that
//! core will compete for CPU and latency can increase. For lowest latency, reserve a subset of cores for SWQOS only via
//! `with_dedicated_sender_threads(Some(indices))` and avoid running other CPU-heavy work on those core indices.

use anyhow::{anyhow, Result};
use crossbeam_queue::ArrayQueue;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use solana_hash::Hash;
use solana_message::AddressLookupTableAccount;
use solana_sdk::{
    instruction::Instruction, pubkey::Pubkey, signature::Keypair, signature::Signature,
};
use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Notify;

use fnv::FnvHasher;

type FnvHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FnvHasher>>;

use crate::{
    common::gas_fee_strategy::{GasFeeStrategyType, GasFeeStrategyValue},
    common::{nonce_cache::DurableNonceInfo, GasFeeStrategy, SwqosSubmitTiming},
    swqos::{SwqosClient, SwqosType, TradeType},
    trading::core::params::SenderConcurrencyConfig,
    trading::{common::build_transaction, MiddlewareManager},
};

/// 与 transaction_pool::PARALLEL_SENDER_COUNT 一致，保证多路 build 不串行
const SWQOS_POOL_WORKERS: usize = 18;
const SWQOS_QUEUE_CAP: usize = 128;
const SWQOS_DEDICATED_DEFAULT_THREADS: usize = 18;

/// Shared across all jobs in one batch; built once, cloned as single Arc per job (minimal hot-path clone).
struct SwqosSharedContext {
    payer: Arc<Keypair>,
    instructions: Arc<Vec<Instruction>>,
    address_lookup_table_account: Option<AddressLookupTableAccount>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    with_tip: bool,
    collector: Arc<ResultCollector>,
}

/// One SWQOS submit task; only per-task data + one Arc to shared (reduces hot-path clones).
struct SwqosJob {
    shared: Arc<SwqosSharedContext>,
    tip: f64,
    unit_limit: u32,
    unit_price: u64,
    tip_account: Arc<Pubkey>,
    swqos_client: Arc<SwqosClient>,
    swqos_type: SwqosType,
    strategy_type: GasFeeStrategyType,
    core_id: Option<core_affinity::CoreId>,
    use_affinity: bool,
}

async fn run_one_swqos_job(job: SwqosJob) {
    let s = &job.shared;
    if job.use_affinity {
        if let Some(cid) = job.core_id {
            core_affinity::set_for_current(cid);
        }
    }

    let tip_amount = if s.with_tip { job.tip } else { 0.0 };

    let transaction = match build_transaction(
        &s.payer,
        job.unit_limit,
        job.unit_price,
        s.instructions.as_ref(),
        s.address_lookup_table_account.as_ref(),
        s.recent_blockhash,
        s.middleware_manager.as_ref(),
        s.protocol_name,
        s.is_buy,
        job.swqos_type != SwqosType::Default,
        &job.tip_account,
        tip_amount,
        s.durable_nonce.as_ref(),
    ) {
        Ok(tx) => tx,
        Err(e) => {
            s.collector.submit(TaskResult {
                success: false,
                signature: Signature::default(),
                error: Some(e),
                swqos_type: job.swqos_type,
                strategy_type: job.strategy_type,
                landed_on_chain: false,
                submit_done_us: crate::common::clock::now_micros(),
            });
            return;
        }
    };

    let (success, err, landed_on_chain) = match job
        .swqos_client
        .send_transaction(
            if s.is_buy { TradeType::Buy } else { TradeType::Sell },
            &transaction,
            s.wait_transaction_confirmed,
        )
        .await
    {
        Ok(()) => (true, None, true),
        Err(e) => {
            let landed = is_landed_error(&e);
            (false, Some(e), landed)
        }
    };

    let sig = transaction.signatures.first().copied().unwrap_or_default();
    s.collector.submit(TaskResult {
        success,
        signature: sig,
        error: err,
        swqos_type: job.swqos_type,
        strategy_type: job.strategy_type,
        landed_on_chain,
        submit_done_us: crate::common::clock::now_micros(),
    });
}

async fn swqos_worker_loop(queue: Arc<ArrayQueue<SwqosJob>>, notify: Arc<Notify>) {
    loop {
        if let Some(job) = queue.pop() {
            run_one_swqos_job(job).await;
        } else {
            notify.notified().await;
        }
    }
}

static SWQOS_QUEUE: OnceCell<Arc<ArrayQueue<SwqosJob>>> = OnceCell::new();
static SWQOS_NOTIFY: OnceCell<Arc<Notify>> = OnceCell::new();
static SWQOS_WORKER_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Dedicated OS-thread sender pool. Queue and notify are in OnceCell so hot path never takes a lock after init.
static DEDICATED_QUEUE: OnceCell<Arc<ArrayQueue<SwqosJob>>> = OnceCell::new();
static DEDICATED_NOTIFY: OnceCell<Arc<Notify>> = OnceCell::new();
static DEDICATED_WORKER_COUNT: AtomicUsize = AtomicUsize::new(0);
/// JoinHandles kept so dedicated threads are not detached; only touched during init under lock.
static DEDICATED_INIT: Mutex<Option<Vec<std::thread::JoinHandle<()>>>> = Mutex::new(None);

fn desired_dedicated_workers(
    sender_thread_cores: Option<&[usize]>,
    max_sender_concurrency: usize,
) -> usize {
    sender_thread_cores
        .map(|v| v.len().min(max_sender_concurrency))
        .unwrap_or_else(|| SWQOS_DEDICATED_DEFAULT_THREADS.min(max_sender_concurrency))
        .min(32)
        .max(1)
}

fn dedicated_core_ids(
    sender_thread_cores: Option<&[usize]>,
    n: usize,
) -> Vec<core_affinity::CoreId> {
    core_affinity::get_core_ids()
        .map(|all_ids| {
            sender_thread_cores
                .map(|indices| {
                    indices.iter().take(n).filter_map(|&i| all_ids.get(i).cloned()).collect()
                })
                .unwrap_or_else(|| all_ids.into_iter().take(n).collect())
        })
        .unwrap_or_default()
}

fn ensure_dedicated_pool(
    sender_thread_cores: Option<&[usize]>,
    max_sender_concurrency: usize,
) -> (Arc<ArrayQueue<SwqosJob>>, Arc<Notify>) {
    let target_workers = desired_dedicated_workers(sender_thread_cores, max_sender_concurrency);
    if let (Some(q), Some(n)) = (DEDICATED_QUEUE.get(), DEDICATED_NOTIFY.get()) {
        if DEDICATED_WORKER_COUNT.load(Ordering::Acquire) >= target_workers {
            return (q.clone(), n.clone());
        }
        ensure_dedicated_worker_count(q.clone(), n.clone(), sender_thread_cores, target_workers);
        return (q.clone(), n.clone());
    }
    let mut guard = DEDICATED_INIT.lock();
    if let (Some(q), Some(n)) = (DEDICATED_QUEUE.get(), DEDICATED_NOTIFY.get()) {
        if DEDICATED_WORKER_COUNT.load(Ordering::Acquire) < target_workers {
            ensure_dedicated_worker_count_locked(
                q.clone(),
                n.clone(),
                sender_thread_cores,
                target_workers,
                &mut guard,
            );
        }
        return (q.clone(), n.clone());
    }
    let queue = Arc::new(ArrayQueue::new(SWQOS_QUEUE_CAP));
    let notify = Arc::new(Notify::new());
    let _ = DEDICATED_QUEUE.set(queue.clone());
    let _ = DEDICATED_NOTIFY.set(notify.clone());
    *guard = Some(Vec::with_capacity(target_workers));
    ensure_dedicated_worker_count_locked(
        queue.clone(),
        notify.clone(),
        sender_thread_cores,
        target_workers,
        &mut guard,
    );
    (queue, notify)
}

fn ensure_dedicated_worker_count(
    queue: Arc<ArrayQueue<SwqosJob>>,
    notify: Arc<Notify>,
    sender_thread_cores: Option<&[usize]>,
    target_workers: usize,
) {
    if DEDICATED_WORKER_COUNT.load(Ordering::Acquire) >= target_workers {
        return;
    }
    let mut guard = DEDICATED_INIT.lock();
    ensure_dedicated_worker_count_locked(
        queue,
        notify,
        sender_thread_cores,
        target_workers,
        &mut guard,
    );
}

fn ensure_dedicated_worker_count_locked(
    queue: Arc<ArrayQueue<SwqosJob>>,
    notify: Arc<Notify>,
    sender_thread_cores: Option<&[usize]>,
    target_workers: usize,
    guard: &mut Option<Vec<std::thread::JoinHandle<()>>>,
) {
    let current = DEDICATED_WORKER_COUNT.load(Ordering::Acquire);
    if current >= target_workers {
        return;
    }
    let core_ids = dedicated_core_ids(sender_thread_cores, target_workers);
    let handles = guard.get_or_insert_with(Vec::new);
    handles.reserve(target_workers.saturating_sub(current));
    for i in current..target_workers {
        let queue = queue.clone();
        let notify = notify.clone();
        let core_id = core_ids.get(i).cloned();
        let handle = std::thread::spawn(move || {
            if let Some(cid) = core_id {
                core_affinity::set_for_current(cid);
            }
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("dedicated sender runtime");
            rt.block_on(swqos_worker_loop(queue, notify));
        });
        handles.push(handle);
    }
    DEDICATED_WORKER_COUNT.store(target_workers, Ordering::Release);
}

fn ensure_swqos_pool(queue: Arc<ArrayQueue<SwqosJob>>, max_sender_concurrency: usize) {
    let n = SWQOS_POOL_WORKERS.min(max_sender_concurrency).max(1);
    let mut current = SWQOS_WORKER_COUNT.load(Ordering::Acquire);
    while current < n {
        match SWQOS_WORKER_COUNT.compare_exchange(current, n, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
    if current >= n {
        return;
    }
    let notify = SWQOS_NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone();
    for _ in current..n {
        tokio::spawn(swqos_worker_loop(queue.clone(), notify.clone()));
    }
}

#[repr(align(64))]
struct TaskResult {
    success: bool,
    signature: Signature,
    error: Option<anyhow::Error>,
    swqos_type: SwqosType,
    strategy_type: GasFeeStrategyType,
    landed_on_chain: bool,
    /// Microsecond timestamp when this task finished (SWQOS returned); for per-SWQOS event→submit timing.
    submit_done_us: i64,
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
    landed_failed_flag: Arc<AtomicBool>, // 🔧 Tx landed on-chain but failed (nonce consumed)
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
        // ArrayQueue is already synchronized; no extra fence needed
        let is_success = result.success;
        let is_landed_failed = result.landed_on_chain && !result.success;

        let _ = self.results.push(result);

        if is_success {
            self.success_flag.store(true, Ordering::Release);
        } else if is_landed_failed {
            // 🔧 Tx landed but failed (e.g., ExceededSlippage) - nonce is consumed, no point waiting
            self.landed_failed_flag.store(true, Ordering::Release);
        }

        self.completed_count.fetch_add(1, Ordering::Release);
    }

    async fn wait_for_success(
        &self,
    ) -> Option<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
        let start = Instant::now();
        let timeout = std::time::Duration::from_secs(5);
        let poll_interval = std::time::Duration::from_millis(1000);

        loop {
            if self.success_flag.load(Ordering::Acquire) {
                let mut signatures = Vec::new();
                let mut has_success = false;
                let mut submit_timings = Vec::new();
                while let Some(result) = self.results.pop() {
                    signatures.push(result.signature);
                    submit_timings.push(result.submit_timing());
                    if result.success {
                        has_success = true;
                    }
                }
                if has_success && !signatures.is_empty() {
                    return Some((true, signatures, None, submit_timings));
                }
            }

            // Early exit: if a tx landed but failed (e.g., ExceededSlippage),
            // nonce is consumed and other channels can't succeed - return immediately
            if self.landed_failed_flag.load(Ordering::Acquire) {
                let mut signatures = Vec::new();
                let mut landed_error = None;
                let mut submit_timings = Vec::new();
                while let Some(result) = self.results.pop() {
                    signatures.push(result.signature);
                    submit_timings.push(result.submit_timing());
                    // Prefer the error from the tx that actually landed
                    if result.landed_on_chain && result.error.is_some() {
                        landed_error = result.error;
                    }
                }
                if !signatures.is_empty() {
                    return Some((false, signatures, landed_error, submit_timings));
                }
            }

            let completed = self.completed_count.load(Ordering::Acquire);
            if completed >= self.total_tasks {
                let mut signatures = Vec::new();
                let mut last_error = None;
                let mut any_success = false;
                let mut submit_timings = Vec::new();
                while let Some(result) = self.results.pop() {
                    signatures.push(result.signature);
                    submit_timings.push(result.submit_timing());
                    if result.success {
                        any_success = true;
                    }
                    if result.error.is_some() {
                        last_error = result.error;
                    }
                }
                if !signatures.is_empty() {
                    return Some((any_success, signatures, last_error, submit_timings));
                }
                return None;
            }

            if start.elapsed() > timeout {
                return None;
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    fn get_first(
        &self,
    ) -> Option<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
        let mut signatures = Vec::new();
        let mut has_success = false;
        let mut last_error = None;
        let mut submit_timings = Vec::new();

        while let Some(result) = self.results.pop() {
            signatures.push(result.signature);
            submit_timings.push(result.submit_timing());
            if result.success {
                has_success = true;
            }
            if result.error.is_some() {
                last_error = result.error;
            }
        }

        if !signatures.is_empty() {
            Some((has_success, signatures, last_error, submit_timings))
        } else {
            None
        }
    }

    /// 等待全部任务完成（不等待链上确认），然后收集并返回所有签名。用于「多路提交」时返回多笔签名。
    /// 轮询间隔 2ms，避免 50ms 间隔在最后一笔返回时多等几十 ms 拉高 submit 耗时。
    async fn wait_for_all_submitted(
        &self,
        timeout_secs: u64,
    ) -> Option<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
        let start = Instant::now();
        let primary = Duration::from_secs(timeout_secs);
        let poll_interval = Duration::from_millis(2);
        while self.completed_count.load(Ordering::Acquire) < self.total_tasks {
            if start.elapsed() > primary {
                break;
            }
            tokio::time::sleep(poll_interval).await;
        }
        // 「不等待链上确认」仍会等各 SWQOS 的 HTTP 回包；主循环在收齐或触达 `timeout_secs` 后结束。
        // 若主窗口到时仍有未回包通道，晚到的 TaskResult 若立刻 drain 会丢签名——仅在该路径上拉长 grace。
        let all_submitted = self.completed_count.load(Ordering::Acquire) >= self.total_tasks;
        if all_submitted {
            tokio::task::yield_now().await;
        } else {
            tokio::time::sleep(Duration::from_millis(600)).await;
            while self.completed_count.load(Ordering::Acquire) < self.total_tasks {
                if start.elapsed() > primary + Duration::from_secs(6) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        self.get_first()
    }
}

type GasFeeConfig = (SwqosType, GasFeeStrategyType, GasFeeStrategyValue);

#[derive(Debug, Clone, Copy)]
struct SwqosTaskConfig {
    task_ordinal: usize,
    swqos_index: usize,
    gas_fee_config: GasFeeConfig,
}

impl TaskResult {
    #[inline]
    fn submit_timing(&self) -> SwqosSubmitTiming {
        SwqosSubmitTiming {
            swqos_type: self.swqos_type,
            strategy_type: self.strategy_type,
            submit_done_us: self.submit_done_us,
        }
    }
}

fn select_swqos_task_configs(
    swqos_types: &[SwqosType],
    gas_fee_configs: &[GasFeeConfig],
    with_tip: bool,
    check_min_tip: bool,
    min_tip_by_swqos: impl Fn(SwqosType) -> f64,
) -> Vec<SwqosTaskConfig> {
    let mut task_configs = Vec::with_capacity(swqos_types.len() * 3);
    for (i, swqos_type) in swqos_types.iter().copied().enumerate() {
        if !with_tip && !matches!(swqos_type, SwqosType::Default) {
            continue;
        }
        let check_tip = with_tip && !matches!(swqos_type, SwqosType::Default) && check_min_tip;
        let min_tip = if check_tip { min_tip_by_swqos(swqos_type) } else { 0.0 };
        for config in gas_fee_configs {
            if config.0 != swqos_type {
                continue;
            }
            if check_tip && config.2.tip < min_tip {
                if crate::common::sdk_log::sdk_log_enabled() {
                    println!(
                        "⚠️ Config filtered: {:?} tip {} is below minimum required {}",
                        config.0, config.2.tip, min_tip
                    );
                }
                continue;
            }
            task_configs.push(SwqosTaskConfig {
                task_ordinal: task_configs.len(),
                swqos_index: i,
                gas_fee_config: *config,
            });
        }
    }
    task_configs
}

/// Execute trade on multiple SWQOS clients in parallel; returns success flag, all signatures, and last error.
///
/// `sender_config` merges sender_thread_cores, effective_core_ids, max_sender_concurrency (precomputed at SDK init; no get_core_ids on hot path).
pub async fn execute_parallel(
    swqos_clients: &[Arc<SwqosClient>],
    payer: Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_account: Option<AddressLookupTableAccount>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    with_tip: bool,
    gas_fee_strategy: GasFeeStrategy,
    use_dedicated_sender_threads: bool,
    sender_config: SenderConcurrencyConfig,
    check_min_tip: bool,
) -> Result<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
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

    let instructions = Arc::new(instructions);

    // One get_strategies call per batch (avoid N calls in loop).
    let gas_fee_configs =
        gas_fee_strategy.get_strategies(if is_buy { TradeType::Buy } else { TradeType::Sell });
    let swqos_types: Vec<SwqosType> =
        swqos_clients.iter().map(|swqos| swqos.get_swqos_type()).collect();
    let selected_task_configs = select_swqos_task_configs(
        &swqos_types,
        &gas_fee_configs,
        with_tip,
        check_min_tip,
        |swqos_type| {
            swqos_clients
                .iter()
                .find(|swqos| swqos.get_swqos_type() == swqos_type)
                .map(|swqos| swqos.min_tip_sol())
                .unwrap_or(0.0)
        },
    );

    if selected_task_configs.is_empty() {
        return Err(anyhow!("No available gas fee strategy configs"));
    }

    if is_buy && selected_task_configs.len() > 1 && durable_nonce.is_none() {
        return Err(anyhow!("Multiple swqos transactions require durable_nonce to be set.",));
    }

    // Task preparation completed: one shared context (clone once per batch), then minimal per-task data.
    let channel_count = selected_task_configs.len().max(1);
    let collector = Arc::new(ResultCollector::new(channel_count));
    // 上限最多 5s：单路卡死时才会等满；若所有通道在窗口内回完，主循环会提前结束（不睡满秒数）。
    let submit_timeout_secs: u64 =
        (3u64 + (channel_count.min(16) as u64).div_ceil(2).min(6)).clamp(3, 5);
    let shared = Arc::new(SwqosSharedContext {
        payer,
        instructions,
        address_lookup_table_account,
        recent_blockhash,
        durable_nonce,
        middleware_manager,
        protocol_name,
        is_buy,
        wait_transaction_confirmed,
        with_tip,
        collector: collector.clone(),
    });

    let (queue, notify) = if use_dedicated_sender_threads {
        ensure_dedicated_pool(
            sender_config.sender_thread_cores.as_ref().map(|a| a.as_slice()),
            sender_config.max_sender_concurrency,
        )
    } else {
        let q = SWQOS_QUEUE.get_or_init(|| Arc::new(ArrayQueue::new(SWQOS_QUEUE_CAP)));
        ensure_swqos_pool(q.clone(), sender_config.max_sender_concurrency);
        (q.clone(), SWQOS_NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone())
    };

    {
        let effective_core_ids = sender_config.effective_core_ids.as_slice();
        let core_len = effective_core_ids.len().max(1);
        let mut tip_cache: FnvHashMap<*const (), Arc<Pubkey>> =
            FnvHashMap::with_capacity_and_hasher(
                selected_task_configs.len(),
                BuildHasherDefault::default(),
            );
        for task_config in selected_task_configs {
            let swqos_client = swqos_clients[task_config.swqos_index].clone();
            let core_id = effective_core_ids.get(task_config.task_ordinal % core_len).copied();
            let swqos_type = swqos_client.get_swqos_type();
            let gas_fee_strategy_config = task_config.gas_fee_config;
            let key = Arc::as_ptr(&swqos_client) as *const ();
            let tip_account = match tip_cache.get(&key) {
                Some(t) => t.clone(),
                None => {
                    let s = swqos_client.get_tip_account()?;
                    let tip = Arc::new(Pubkey::from_str(&s).unwrap_or_default());
                    tip_cache.insert(key, tip.clone());
                    tip
                }
            };
            let (tip, unit_limit, unit_price) = (
                gas_fee_strategy_config.2.tip,
                gas_fee_strategy_config.2.cu_limit,
                gas_fee_strategy_config.2.cu_price,
            );
            let job = SwqosJob {
                shared: shared.clone(),
                tip,
                unit_limit,
                unit_price,
                tip_account,
                swqos_client,
                swqos_type,
                strategy_type: gas_fee_strategy_config.1,
                core_id,
                use_affinity: !effective_core_ids.is_empty(),
            };
            if let Err(job) = queue.push(job) {
                shared.collector.submit(TaskResult {
                    success: false,
                    signature: Signature::default(),
                    error: Some(anyhow!("SWQOS sender queue is full")),
                    swqos_type: job.swqos_type,
                    strategy_type: job.strategy_type,
                    landed_on_chain: false,
                    submit_done_us: crate::common::clock::now_micros(),
                });
            }
        }
    }

    notify.notify_waiters();

    // All jobs enqueued (no spawn on hot path)

    if !wait_transaction_confirmed {
        // submit_timeout_secs 为「等齐各 SWQOS HTTP 应答」的上限；收齐后会立刻进入短 settle，不会睡满整段秒数。
        // `wait_for_all_submitted` 仅在未收齐时追加长 grace，避免过早 drain 丢晚到签名。
        let ret = collector.wait_for_all_submitted(submit_timeout_secs).await.unwrap_or((
            false,
            vec![],
            Some(anyhow!("No SWQOS result within grace window (primary {}s)", submit_timeout_secs)),
            vec![],
        ));
        let (success, signatures, last_error, submit_timings) = ret;
        return Ok((success, signatures, last_error, submit_timings));
    }

    if let Some(result) = collector.wait_for_success().await {
        let (success, signatures, last_error, submit_timings) = result;
        Ok((success, signatures, last_error, submit_timings))
    } else {
        Err(anyhow!("All transactions failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(cu_price: u64, tip: f64) -> GasFeeStrategyValue {
        GasFeeStrategyValue { cu_limit: 100_000, cu_price, tip }
    }

    #[test]
    fn select_task_configs_keeps_two_fee_lanes_per_swqos() {
        let swqos_types = [SwqosType::Jito, SwqosType::Helius];
        let configs = [
            (SwqosType::Jito, GasFeeStrategyType::LowTipHighCuPrice, value(400_000, 0.002)),
            (SwqosType::Jito, GasFeeStrategyType::HighTipLowCuPrice, value(180_000, 0.005)),
            (SwqosType::Helius, GasFeeStrategyType::LowTipHighCuPrice, value(400_000, 0.002)),
            (SwqosType::Helius, GasFeeStrategyType::HighTipLowCuPrice, value(180_000, 0.005)),
        ];

        let selected = select_swqos_task_configs(&swqos_types, &configs, true, false, |_| 0.0);

        assert_eq!(selected.len(), 4);
        assert_eq!(
            selected.iter().filter(|task| task.gas_fee_config.0 == SwqosType::Jito).count(),
            2
        );
        assert_eq!(
            selected.iter().filter(|task| task.gas_fee_config.0 == SwqosType::Helius).count(),
            2
        );
        assert_eq!(
            selected.iter().map(|task| task.task_ordinal).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            selected.iter().map(|task| task.swqos_index).collect::<Vec<_>>(),
            vec![0, 0, 1, 1]
        );
    }

    #[test]
    fn select_task_configs_applies_min_tip_per_lane() {
        let swqos_types = [SwqosType::Jito];
        let configs = [
            (SwqosType::Jito, GasFeeStrategyType::LowTipHighCuPrice, value(400_000, 0.0001)),
            (SwqosType::Jito, GasFeeStrategyType::HighTipLowCuPrice, value(180_000, 0.005)),
        ];

        let selected = select_swqos_task_configs(&swqos_types, &configs, true, true, |_| 0.001);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].gas_fee_config.1, GasFeeStrategyType::HighTipLowCuPrice);
    }

    #[test]
    fn select_task_configs_without_tip_keeps_default_priority_fee_only() {
        let swqos_types = [SwqosType::Jito, SwqosType::Default];
        let configs = [
            (SwqosType::Jito, GasFeeStrategyType::LowTipHighCuPrice, value(400_000, 0.002)),
            (SwqosType::Default, GasFeeStrategyType::Normal, value(700_000, 0.0)),
        ];

        let selected = select_swqos_task_configs(&swqos_types, &configs, false, false, |_| 0.0);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].gas_fee_config.0, SwqosType::Default);
        assert_eq!(selected[0].gas_fee_config.2.cu_price, 700_000);
        assert_eq!(selected[0].gas_fee_config.2.tip, 0.0);
    }
}
