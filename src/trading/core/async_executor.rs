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
use futures::FutureExt;
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
use std::{str::FromStr, sync::Arc, time::Duration};
use tokio::sync::Notify;

use fnv::FnvHasher;

type FnvHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FnvHasher>>;

use crate::{
    common::gas_fee_strategy::{GasFeeStrategyType, GasFeeStrategyValue},
    common::{nonce_cache::DurableNonceInfo, GasFeeStrategy, SwqosSubmitTiming},
    swqos::{SwqosClient, SwqosType, TradeType},
    trading::core::params::SenderConcurrencyConfig,
    trading::{common::build_transaction_with_version, MiddlewareManager},
};

/// 与 transaction_pool::PARALLEL_SENDER_COUNT 一致，保证多路 build 不串行
const SWQOS_POOL_WORKERS: usize = 18;
const SWQOS_QUEUE_CAP: usize = 128;
const SWQOS_DEDICATED_DEFAULT_THREADS: usize = 18;
const FAST_SUBMIT_RESULT_TIMEOUT: Duration = Duration::from_secs(5);
const FAST_SUBMIT_DRAIN_GRACE: Duration = Duration::from_millis(20);

/// Shared across all jobs in one batch; built once, cloned as single Arc per job (minimal hot-path clone).
struct SwqosSharedContext {
    payer: Arc<Keypair>,
    instructions: Arc<Vec<Instruction>>,
    address_lookup_table_accounts: Arc<Vec<AddressLookupTableAccount>>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    with_tip: bool,
    collector: Arc<ResultCollector>,
    transaction_version: crate::common::TradeTransactionVersion,
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

    let transaction = match build_transaction_with_version(
        &s.payer,
        job.unit_limit,
        job.unit_price,
        s.transaction_version,
        s.instructions.as_ref(),
        s.address_lookup_table_accounts.as_slice(),
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
        let job = {
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match queue.pop() {
                Some(job) => Some(job),
                None => {
                    notified.await;
                    None
                }
            }
        };

        if let Some(job) = job {
            // User-supplied transports/middleware may panic. Keep this worker
            // alive and complete its lane rather than stranding later batches.
            let collector = job.shared.collector.clone();
            let swqos_type = job.swqos_type;
            let strategy_type = job.strategy_type;
            if std::panic::AssertUnwindSafe(run_one_swqos_job(job)).catch_unwind().await.is_err() {
                collector.submit(TaskResult {
                    success: false,
                    signature: Signature::default(),
                    error: Some(anyhow!("SWQOS sender task panicked")),
                    swqos_type,
                    strategy_type,
                    landed_on_chain: false,
                    submit_done_us: crate::common::clock::now_micros(),
                });
            }
        }
    }
}

static SWQOS_QUEUE: OnceCell<Arc<ArrayQueue<SwqosJob>>> = OnceCell::new();
static SWQOS_NOTIFY: OnceCell<Arc<Notify>> = OnceCell::new();
static SWQOS_WORKER_COUNT: AtomicUsize = AtomicUsize::new(0);
// The process-wide queue must have a process-wide runtime owner. Workers must
// not be cancelled when the runtime of an individual caller is dropped.
static SWQOS_RUNTIME: OnceCell<tokio::runtime::Runtime> = OnceCell::new();

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
    drop(guard);
    (queue, notify)
}

/// Pre-spawn dedicated sender threads during SDK initialization, avoiding first-submit thread
/// creation cost on the trading hot path.
pub fn warm_dedicated_sender_pool(
    sender_thread_cores: Option<&[usize]>,
    max_sender_concurrency: usize,
) {
    let _ = ensure_dedicated_pool(sender_thread_cores, max_sender_concurrency);
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

fn ensure_swqos_pool(queue: Arc<ArrayQueue<SwqosJob>>, max_sender_concurrency: usize) -> Result<()> {
    let runtime = SWQOS_RUNTIME.get_or_try_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("sol-trade-swqos")
            .enable_all()
            .build()
    })?;
    let n = SWQOS_POOL_WORKERS.min(max_sender_concurrency).max(1);
    let mut current = SWQOS_WORKER_COUNT.load(Ordering::Acquire);
    while current < n {
        match SWQOS_WORKER_COUNT.compare_exchange(current, n, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
    if current >= n {
        return Ok(());
    }
    let notify = SWQOS_NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone();
    for _ in current..n {
        runtime.spawn(swqos_worker_loop(queue.clone(), notify.clone()));
    }
    Ok(())
}

/// Cold-path initialization for the default process-owned sender runtime.
/// Call before trade triggers to avoid first-submit runtime/thread startup.
pub fn warm_swqos_sender_pool(max_sender_concurrency: usize) -> Result<()> {
    let queue = SWQOS_QUEUE.get_or_init(|| Arc::new(ArrayQueue::new(SWQOS_QUEUE_CAP)));
    ensure_swqos_pool(queue.clone(), max_sender_concurrency)
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

    // Only confirmation polling supplies an instruction index. Transport errors
    // also use TradeError (including code 500), so a numeric code is not evidence
    // that a durable nonce was consumed. Custom program error zero is valid.
    error.downcast_ref::<TradeError>().is_some_and(|error| error.instruction.is_some())
}

struct ResultCollector {
    results: ArrayQueue<TaskResult>,
    success_flag: AtomicBool,
    landed_failed_flag: AtomicBool, // 🔧 Tx landed on-chain but failed (nonce consumed)
    completed_count: AtomicUsize,
    result_notify: Notify,
    total_tasks: usize,
    uses_durable_nonce: bool,
}

impl ResultCollector {
    fn new(capacity: usize, uses_durable_nonce: bool) -> Self {
        Self {
            results: ArrayQueue::new(capacity),
            success_flag: AtomicBool::new(false),
            landed_failed_flag: AtomicBool::new(false),
            completed_count: AtomicUsize::new(0),
            result_notify: Notify::new(),
            total_tasks: capacity,
            uses_durable_nonce,
        }
    }

    fn submit(&self, result: TaskResult) {
        // ArrayQueue is already synchronized; no extra fence needed
        let is_success = result.success;
        let is_landed_failed = self.uses_durable_nonce && result.landed_on_chain && !result.success;

        let _ = self.results.push(result);

        if is_success {
            self.success_flag.store(true, Ordering::Release);
        } else if is_landed_failed {
            // 🔧 Tx landed but failed (e.g., ExceededSlippage) - nonce is consumed, no point waiting
            self.landed_failed_flag.store(true, Ordering::Release);
        }

        self.completed_count.fetch_add(1, Ordering::Release);
        self.result_notify.notify_one();
    }

    async fn wait_for_success(
        &self,
    ) -> Option<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
        let deadline = tokio::time::Instant::now() + FAST_SUBMIT_RESULT_TIMEOUT;
        let deadline_sleep = tokio::time::sleep_until(deadline);
        tokio::pin!(deadline_sleep);

        loop {
            // Register before checking state so a concurrent submit cannot be missed
            // between the predicate check and the await.
            let notified = self.result_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

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

            tokio::select! {
                _ = notified.as_mut() => {}
                _ = deadline_sleep.as_mut() => return None,
            }
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

    /// Fast submit mode for callers that do not wait for on-chain confirmation.
    /// Return as soon as one route accepts, a landed failure consumes the nonce, all routes finish,
    /// or the submit result window expires. Slow HTTP routes continue in worker tasks but no longer
    /// block post-buy monitoring / sell scheduling.
    async fn wait_for_first_submitted(
        &self,
        timeout: Duration,
    ) -> Option<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
        let deadline = tokio::time::Instant::now() + timeout;
        let deadline_sleep = tokio::time::sleep_until(deadline);
        tokio::pin!(deadline_sleep);

        loop {
            let notified = self.result_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if self.success_flag.load(Ordering::Acquire)
                || self.landed_failed_flag.load(Ordering::Acquire)
                || self.completed_count.load(Ordering::Acquire) >= self.total_tasks
            {
                return self.get_first();
            }

            tokio::select! {
                _ = notified.as_mut() => {}
                _ = deadline_sleep.as_mut() => return self.get_first(),
            }
        }
    }

    /// 等待全部任务完成（不等待链上确认），然后收集并返回所有已返回的签名。
    /// 提交完成时由 worker 主动唤醒，避免固定间隔轮询增加调度开销和返回延迟。
    /// Re-enabled via `SwapParams.wait_for_all_submits` for callers that need
    /// every submitted signature, either for external monitoring or for
    /// executor-level poll-any confirmation after parallel submit.
    async fn wait_for_all_submitted(
        &self,
        timeout_secs: u64,
    ) -> Option<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let deadline_sleep = tokio::time::sleep_until(deadline);
        tokio::pin!(deadline_sleep);

        let mut timed_out = false;
        while self.completed_count.load(Ordering::Acquire) < self.total_tasks {
            let notified = self.result_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if self.completed_count.load(Ordering::Acquire) >= self.total_tasks {
                break;
            }

            tokio::select! {
                _ = notified.as_mut() => {}
                _ = deadline_sleep.as_mut() => {
                    timed_out = true;
                    break;
                }
            }
        }

        // Bound the opt-in "all submits" path tightly. A slow relay must not
        // delay poll-any confirmation by multiple seconds after the submit
        // window; give only a short grace for a just-finished worker to publish.
        if timed_out {
            let grace_deadline = tokio::time::Instant::now() + FAST_SUBMIT_DRAIN_GRACE;
            while self.completed_count.load(Ordering::Acquire) < self.total_tasks {
                let notified = self.result_notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                if self.completed_count.load(Ordering::Acquire) >= self.total_tasks {
                    break;
                }

                tokio::select! {
                    _ = notified.as_mut() => {}
                    _ = tokio::time::sleep_until(grace_deadline) => break,
                }
            }
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
    min_tip_by_index: impl Fn(usize) -> f64,
    configured_min_tip: impl Fn(usize) -> Option<u64>,
) -> Vec<SwqosTaskConfig> {
    let mut task_configs = Vec::with_capacity(swqos_types.len() * 3);
    for (i, swqos_type) in swqos_types.iter().copied().enumerate() {
        if !with_tip && !matches!(swqos_type, SwqosType::Default) {
            continue;
        }
        let check_tip = with_tip && !matches!(swqos_type, SwqosType::Default) && check_min_tip;
        let min_tip = if check_tip { min_tip_by_index(i) } else { 0.0 };
        let configured_minimum = configured_min_tip(i);
        for config in gas_fee_configs {
            if config.0 != swqos_type {
                continue;
            }
            if configured_minimum.is_some_and(|minimum| {
                let tip_lamports = if with_tip {
                    crate::trading::common::transaction_builder::sol_f64_to_lamports(config.2.tip)
                } else { 0 };
                tip_lamports < minimum
            }) {
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
#[allow(clippy::too_many_arguments)]
pub async fn execute_parallel(
    swqos_clients: &[Arc<SwqosClient>],
    payer: Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    wait_for_all_submits: bool,
    with_tip: bool,
    gas_fee_strategy: GasFeeStrategy,
    use_dedicated_sender_threads: bool,
    sender_config: SenderConcurrencyConfig,
    check_min_tip: bool,
) -> Result<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
    execute_parallel_with_version(
        swqos_clients,
        payer,
        instructions,
        address_lookup_table_accounts,
        recent_blockhash,
        durable_nonce,
        middleware_manager,
        protocol_name,
        is_buy,
        wait_transaction_confirmed,
        wait_for_all_submits,
        with_tip,
        gas_fee_strategy,
        use_dedicated_sender_threads,
        sender_config,
        check_min_tip,
        crate::common::TradeTransactionVersion::V0,
    )
    .await
}

/// Execute a trade batch using an explicitly selected transaction message version.
#[allow(clippy::too_many_arguments)]
pub async fn execute_parallel_with_version(
    swqos_clients: &[Arc<SwqosClient>],
    payer: Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    wait_transaction_confirmed: bool,
    wait_for_all_submits: bool,
    with_tip: bool,
    gas_fee_strategy: GasFeeStrategy,
    use_dedicated_sender_threads: bool,
    sender_config: SenderConcurrencyConfig,
    check_min_tip: bool,
    transaction_version: crate::common::TradeTransactionVersion,
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
        |index| swqos_clients[index].min_tip_sol(),
        |index| swqos_clients[index].configured_min_tip_lamports(),
    );

    if selected_task_configs.is_empty() {
        return Err(anyhow!("No available gas fee strategy configs"));
    }

    // Task preparation completed: one shared context (clone once per batch), then minimal per-task data.
    let channel_count = selected_task_configs.len().max(1);
    let collector = Arc::new(ResultCollector::new(channel_count, durable_nonce.is_some()));
    let shared = Arc::new(SwqosSharedContext {
        payer,
        instructions,
        address_lookup_table_accounts: Arc::new(address_lookup_table_accounts),
        recent_blockhash,
        durable_nonce,
        middleware_manager,
        protocol_name,
        is_buy,
        wait_transaction_confirmed,
        with_tip,
        collector: collector.clone(),
        transaction_version,
    });

    // Resolve every fallible provider input before making any lane visible to
    // workers. A failed preparation must never leak a transaction into a later call.
    let mut jobs = Vec::with_capacity(channel_count);
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
                    // Default RPC has no tip destination and deliberately returns
                    // an empty string. All actual tip addresses must parse.
                    let tip = Arc::new(if s.is_empty() && swqos_type == SwqosType::Default {
                        Pubkey::default()
                    } else {
                        Pubkey::from_str(&s)?
                    });
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
            jobs.push(job);
        }
    }

    let (queue, notify) = if use_dedicated_sender_threads {
        ensure_dedicated_pool(
            sender_config.sender_thread_cores.as_ref().map(|a| a.as_slice()),
            sender_config.max_sender_concurrency,
        )
    } else {
        let q = SWQOS_QUEUE.get_or_init(|| Arc::new(ArrayQueue::new(SWQOS_QUEUE_CAP)));
        ensure_swqos_pool(q.clone(), sender_config.max_sender_concurrency)?;
        (q.clone(), SWQOS_NOTIFY.get_or_init(|| Arc::new(Notify::new())).clone())
    };

    for job in jobs {
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

    notify.notify_waiters();

    // All jobs enqueued (no spawn on hot path)

    if !wait_transaction_confirmed {
        let ret = if wait_for_all_submits {
            collector.wait_for_all_submitted(FAST_SUBMIT_RESULT_TIMEOUT.as_secs()).await.unwrap_or(
                (
                    false,
                    vec![],
                    Some(anyhow!("No SWQOS result within submit result window")),
                    vec![],
                ),
            )
        } else {
            collector.wait_for_first_submitted(FAST_SUBMIT_RESULT_TIMEOUT).await.unwrap_or((
                false,
                vec![],
                Some(anyhow!("No SWQOS result within submit result window")),
                vec![],
            ))
        };
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
    use std::time::Instant;

    struct LifecycleTransport {
        sends: Arc<AtomicUsize>,
        fail_tip: bool,
        invalid_tip: bool,
        panic_send: bool,
    }

    #[async_trait::async_trait]
    impl crate::swqos::SwqosClientTrait for LifecycleTransport {
        async fn send_transaction(
            &self, _: TradeType, _: &solana_sdk::transaction::VersionedTransaction, _: bool,
        ) -> Result<()> {
            assert!(!self.panic_send, "mock transport panic");
            self.sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn send_transactions(
            &self, _: TradeType, _: &Vec<solana_sdk::transaction::VersionedTransaction>, _: bool,
        ) -> Result<()> {
            unreachable!()
        }
        fn get_tip_account(&self) -> Result<String> {
            if self.fail_tip { return Err(anyhow!("mock tip lookup failed")); }
            Ok(if self.invalid_tip { "invalid tip".to_owned() } else { Pubkey::default().to_string() })
        }
        fn get_swqos_type(&self) -> SwqosType { SwqosType::Default }
    }

    async fn submit_lifecycle_batch(
        clients: &[Arc<SwqosClient>], dedicated: bool,
    ) -> Result<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<SwqosSubmitTiming>)> {
        let strategy = GasFeeStrategy::new();
        strategy.set_default_rpc_fee_strategy(200_000, 200_000, 0, 0);
        tokio::time::timeout(Duration::from_secs(2), execute_parallel(
            clients, Arc::new(Keypair::new()), vec![], vec![], Some(Hash::new_unique()),
            None, None, "offline-lifecycle", true, false, true, false, strategy, dedicated,
            SenderConcurrencyConfig {
                sender_thread_cores: None, effective_core_ids: Arc::new(vec![]), max_sender_concurrency: 1,
            }, false,
        )).await.expect("sender pool must remain responsive")
    }

    #[test]
    fn default_pool_survives_replacement_caller_runtimes() {
        let sends = Arc::new(AtomicUsize::new(0));
        let client: Arc<SwqosClient> = Arc::new(LifecycleTransport {
            sends: sends.clone(), fail_tip: false, invalid_tip: false, panic_send: false,
        });
        for _ in 0..3 {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let result = runtime.block_on(submit_lifecycle_batch(&[client.clone()], false)).unwrap();
            assert!(result.0);
            assert_eq!(result.1.len(), 1);
        }
        assert_eq!(sends.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn default_rpc_empty_tip_preserves_local_nonce_validation() {
        let client = crate::swqos::SwqosConfig::get_swqos_client(
            "http://127.0.0.1:1".into(), Default::default(),
            crate::swqos::SwqosConfig::Default("http://127.0.0.1:1".into()), false,
        ).await.unwrap();
        let strategy = GasFeeStrategy::new();
        strategy.set_default_rpc_fee_strategy(200_000, 200_000, 0, 0);
        let result = tokio::time::timeout(Duration::from_secs(2), execute_parallel(
            &[client], Arc::new(Keypair::new()), vec![], vec![], Some(Hash::new_unique()),
            Some(DurableNonceInfo { nonce_account: None, current_nonce: Some(Hash::new_unique()) }),
            None, "offline-validation", true, false, true, false, strategy, false,
            SenderConcurrencyConfig {
                sender_thread_cores: None, effective_core_ids: Arc::new(vec![]), max_sender_concurrency: 1,
            }, false,
        )).await.unwrap().unwrap();
        assert!(!result.0);
        assert_eq!(result.1.len(), 1);
        assert!(result.2.unwrap().to_string().contains("nonce_account is None"));
    }

    #[tokio::test]
    async fn failed_batch_preparation_never_publishes_partial_jobs() {
        for dedicated in [false, true] {
            let sends = Arc::new(AtomicUsize::new(0));
            let good: Arc<SwqosClient> = Arc::new(LifecycleTransport {
                sends: sends.clone(), fail_tip: false, invalid_tip: false, panic_send: false,
            });
            assert!(submit_lifecycle_batch(&[good.clone()], dedicated).await.unwrap().0);
            for invalid_tip in [false, true] {
                let bad: Arc<SwqosClient> = Arc::new(LifecycleTransport {
                    sends: sends.clone(), fail_tip: !invalid_tip, invalid_tip, panic_send: false,
                });
                assert!(submit_lifecycle_batch(&[good.clone(), bad], dedicated).await.is_err());
                assert_eq!(sends.load(Ordering::SeqCst), if invalid_tip { 2 } else { 1 });
                assert!(submit_lifecycle_batch(&[good.clone()], dedicated).await.unwrap().0);
            }
            assert_eq!(sends.load(Ordering::SeqCst), 3);
        }
    }

    #[tokio::test]
    async fn transport_panic_does_not_remove_pool_worker() {
        let sends = Arc::new(AtomicUsize::new(0));
        let panicking: Arc<SwqosClient> = Arc::new(LifecycleTransport {
            sends: sends.clone(), fail_tip: false, invalid_tip: false, panic_send: true,
        });
        let result = submit_lifecycle_batch(&[panicking], false).await.unwrap();
        assert!(!result.0);
        assert!(result.2.unwrap().to_string().contains("panicked"));
        let good: Arc<SwqosClient> = Arc::new(LifecycleTransport {
            sends: sends.clone(), fail_tip: false, invalid_tip: false, panic_send: false,
        });
        assert!(submit_lifecycle_batch(&[good], false).await.unwrap().0);
        assert_eq!(sends.load(Ordering::SeqCst), 1);
    }

    fn value(cu_price: u64, tip: f64) -> GasFeeStrategyValue {
        GasFeeStrategyValue { cu_limit: 100_000, cu_price, tip }
    }

    fn task_result(success: bool, landed_on_chain: bool) -> TaskResult {
        TaskResult {
            success,
            signature: Signature::default(),
            error: (!success).then(|| anyhow!("submit failed")),
            swqos_type: SwqosType::Default,
            strategy_type: GasFeeStrategyType::Normal,
            landed_on_chain,
            submit_done_us: crate::common::clock::now_micros(),
        }
    }

    #[test]
    fn landed_error_requires_confirmation_evidence() {
        use crate::swqos::common::TradeError;
        for code in [0, 500, 6004] {
            let transport = anyhow::Error::new(TradeError {
                code, message: "submission failed".into(), instruction: None,
            });
            assert!(!is_landed_error(&transport));
            let confirmed = anyhow::Error::new(TradeError {
                code, message: "instruction failed".into(), instruction: Some(0),
            });
            assert!(is_landed_error(&confirmed));
        }
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

        let selected = select_swqos_task_configs(&swqos_types, &configs, true, false, |_| 0.0, |_| None);

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

        let selected = select_swqos_task_configs(&swqos_types, &configs, true, true, |_| 0.001, |_| None);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].gas_fee_config.1, GasFeeStrategyType::HighTipLowCuPrice);
    }

    #[test]
    fn built_in_minimum_is_endpoint_specific_for_helius_modes() {
        let types = [SwqosType::Helius, SwqosType::Helius];
        let configs = [(SwqosType::Helius, GasFeeStrategyType::Normal, value(0, 0.000005))];
        for minima in [[0.0002, 0.000005], [0.000005, 0.0002]] {
            let selected = select_swqos_task_configs(
                &types, &configs, true, true, |i| minima[i], |_| None,
            );
            assert_eq!(selected.len(), 1);
            assert_eq!(minima[selected[0].swqos_index], 0.000005);
        }
    }

    #[test]
    fn configured_min_tip_filters_each_route_and_lane_without_global_check() {
        // Two endpoints of the same provider must retain independent thresholds.
        let types = [SwqosType::Jito, SwqosType::Jito, SwqosType::Default];
        let configs = [
            (SwqosType::Jito, GasFeeStrategyType::LowTipHighCuPrice, value(400_000, 0.099999999)),
            (SwqosType::Jito, GasFeeStrategyType::HighTipLowCuPrice, value(180_000, 0.1)),
            (SwqosType::Default, GasFeeStrategyType::Normal, value(180_000, 0.0)),
        ];
        let selected = select_swqos_task_configs(&types, &configs, true, false,
            |_| panic!("built-in minimum must not be consulted"),
            |i| if i == 0 { Some(100_000_000) } else { None });
        assert_eq!(selected.iter().map(|t| t.swqos_index).collect::<Vec<_>>(), vec![0, 1, 1, 2]);
        assert_eq!(selected[0].gas_fee_config.1, GasFeeStrategyType::HighTipLowCuPrice);
        // Explicit zero preserves participation even for a zero-tip lane.
        let selected = select_swqos_task_configs(&types, &configs, true, false,
            |_| 0.0, |_| Some(0));
        assert_eq!(selected.len(), 5);
    }

    #[test]
    fn select_task_configs_without_tip_keeps_default_priority_fee_only() {
        let swqos_types = [SwqosType::Jito, SwqosType::Default];
        let configs = [
            (SwqosType::Jito, GasFeeStrategyType::LowTipHighCuPrice, value(400_000, 0.002)),
            (SwqosType::Default, GasFeeStrategyType::Normal, value(700_000, 0.0)),
        ];

        let selected = select_swqos_task_configs(&swqos_types, &configs, false, false, |_| 0.0, |_| None);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].gas_fee_config.0, SwqosType::Default);
        assert_eq!(selected[0].gas_fee_config.2.cu_price, 700_000);
        assert_eq!(selected[0].gas_fee_config.2.tip, 0.0);
    }

    #[tokio::test]
    async fn wait_for_all_submitted_timeout_is_bounded() {
        let collector = ResultCollector::new(1, false);
        let start = Instant::now();

        let result = collector.wait_for_all_submitted(0).await;

        assert!(result.is_none());
        assert!(
            start.elapsed() < Duration::from_millis(150),
            "wait_for_all_submitted should not add multi-second grace after timeout"
        );
    }

    #[tokio::test]
    async fn wait_for_first_submitted_wakes_on_success() {
        let collector = Arc::new(ResultCollector::new(2, false));
        let waiter = collector.clone();
        let waiting =
            tokio::spawn(
                async move { waiter.wait_for_first_submitted(Duration::from_secs(1)).await },
            );

        tokio::task::yield_now().await;
        collector.submit(task_result(true, true));

        let result = tokio::time::timeout(Duration::from_millis(100), waiting)
            .await
            .expect("success notification should wake the waiter")
            .expect("waiter task should finish")
            .expect("submitted result should be returned");
        assert!(result.0);
        assert_eq!(result.1.len(), 1);
    }

    #[tokio::test]
    async fn wait_for_success_wakes_on_landed_failure() {
        let collector = Arc::new(ResultCollector::new(2, true));
        let waiter = collector.clone();
        let waiting = tokio::spawn(async move { waiter.wait_for_success().await });

        tokio::task::yield_now().await;
        collector.submit(task_result(false, true));

        let result = tokio::time::timeout(Duration::from_millis(100), waiting)
            .await
            .expect("landed-failure notification should wake the waiter")
            .expect("waiter task should finish")
            .expect("landed failure should be returned");
        assert!(!result.0);
        assert!(result.2.is_some());
    }

    #[tokio::test]
    async fn recent_blockhash_landed_failure_waits_for_another_lane() {
        for confirmed in [false, true] {
            let collector = Arc::new(ResultCollector::new(2, false));
            collector.submit(task_result(false, true));
            let waiter = collector.clone();
            let mut waiting = tokio::spawn(async move {
                if confirmed {
                    waiter.wait_for_success().await
                } else {
                    waiter.wait_for_first_submitted(Duration::from_secs(1)).await
                }
            });
            assert!(tokio::time::timeout(Duration::from_millis(10), &mut waiting).await.is_err());
            collector.submit(task_result(true, true));
            let result = tokio::time::timeout(Duration::from_millis(100), waiting)
                .await.unwrap().unwrap().unwrap();
            assert!(result.0);
            assert_eq!(result.1.len(), 2);
        }
    }

    #[tokio::test]
    async fn durable_nonce_failure_short_circuits_fast_submit() {
        let collector = ResultCollector::new(2, true);
        collector.submit(task_result(false, true));
        let result = tokio::time::timeout(Duration::from_millis(100),
            collector.wait_for_first_submitted(Duration::from_secs(1)))
            .await.unwrap().unwrap();
        assert!(!result.0);
        assert!(result.2.is_some());
    }

    #[tokio::test]
    async fn independent_lanes_still_return_failure_when_all_complete() {
        let collector = ResultCollector::new(2, false);
        collector.submit(task_result(false, true));
        collector.submit(task_result(false, false));
        let result = collector.wait_for_success().await.unwrap();
        assert!(!result.0);
        assert_eq!(result.1.len(), 2);
        assert!(result.2.is_some());
    }

    #[tokio::test]
    async fn wait_for_all_submitted_handles_preexisting_notifications() {
        let collector = ResultCollector::new(2, false);
        collector.submit(task_result(false, false));
        collector.submit(task_result(true, true));

        let result =
            tokio::time::timeout(Duration::from_millis(100), collector.wait_for_all_submitted(1))
                .await
                .expect("completed state should be observed without polling")
                .expect("completed results should be returned");
        assert!(result.0);
        assert_eq!(result.1.len(), 2);
    }
}
