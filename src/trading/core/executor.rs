use anyhow::Result;
use solana_sdk::signature::Signature;
use std::{sync::Arc, time::Instant};

use crate::{
    perf::syscall_bypass::SystemCallBypassManager,
    trading::core::{
        async_executor::execute_parallel,
        execution::{Prefetch, InstructionProcessor, ExecutionPath},
        traits::TradeExecutor,
    },
};
use once_cell::sync::Lazy;

use super::{params::SwapParams, traits::InstructionBuilder};

/// 🚀 全局系统调用绕过管理器
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
        Self {
            instruction_builder,
            protocol_name,
        }
    }
}

#[async_trait::async_trait]
impl TradeExecutor for GenericTradeExecutor {
    async fn swap(&self, params: SwapParams) -> Result<(bool, Signature)> {
        let total_start = Instant::now();

        // 判断买卖方向
        let is_buy = ExecutionPath::is_buy(&params.input_mint)
            || (params.input_mint == crate::constants::USD1_TOKEN_ACCOUNT
                && params.output_mint != crate::constants::WSOL_TOKEN_ACCOUNT);

        // CPU 预取
        Prefetch::keypair(&params.payer);

        // 构建指令
        let build_start = Instant::now();
        let instructions = if is_buy {
            self.instruction_builder.build_buy_instructions(&params).await?
        } else {
            self.instruction_builder.build_sell_instructions(&params).await?
        };
        let build_elapsed = build_start.elapsed();

        // 指令预处理
        InstructionProcessor::preprocess(&instructions)?;

        // 中间件处理
        let final_instructions = match &params.middleware_manager {
            Some(middleware_manager) => {
                middleware_manager
                    .apply_middlewares_process_protocol_instructions(
                        instructions,
                        self.protocol_name.to_string(),
                        is_buy,
                    )?
            }
            None => instructions
        };

        // 提交前耗时
        let before_submit_elapsed = total_start.elapsed();

        // 并行发送交易
        let send_start = Instant::now();
        let result = execute_parallel(
            params.swqos_clients.clone(),
            params.payer,
            params.rpc,
            final_instructions,
            params.lookup_table_key,
            params.recent_blockhash,
            params.durable_nonce,
            if is_buy { params.data_size_limit } else { 0 },
            params.middleware_manager,
            self.protocol_name,
            is_buy,
            params.wait_transaction_confirmed,
            if is_buy { true } else { params.with_tip },
        )
        .await;
        let send_elapsed = send_start.elapsed();
        let total_elapsed = total_start.elapsed();

        // 使用快速时间戳获取性能指标
        let timestamp_ns = SYSCALL_BYPASS.fast_timestamp_nanos();

        // 在完成后一次性打印所有耗时，避免阻塞关键路径
        println!("[时间戳] {}ns", timestamp_ns);
        println!("[构建指令] 耗时: {:.3}ms ({:.0}μs)",
                 build_elapsed.as_micros() as f64 / 1000.0, build_elapsed.as_micros());
        println!("[提交前耗时] {:.3}ms ({:.0}μs)",
                 before_submit_elapsed.as_micros() as f64 / 1000.0, before_submit_elapsed.as_micros());
        println!("[发送交易] 耗时: {:.3}ms ({:.0}μs)",
                 send_elapsed.as_micros() as f64 / 1000.0, send_elapsed.as_micros());
        println!("[总耗时] {:.3}ms ({:.0}μs)",
                 total_elapsed.as_micros() as f64 / 1000.0, total_elapsed.as_micros());

        result
    }

    fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }
}
