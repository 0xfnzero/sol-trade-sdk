use anyhow::Result;
use solana_hash::Hash;
use solana_sdk::{
    instruction::Instruction, message::AddressLookupTableAccount, pubkey::Pubkey,
    signature::Keypair, signature::Signature,
};
use std::{sync::Arc, time::Instant};

use crate::{
    common::{nonce_cache::DurableNonceInfo, GasFeeStrategy, SolanaRpcClient},
    perf::syscall_bypass::SystemCallBypassManager,
    trading::core::{
        async_executor::execute_parallel,
        execution::{ExecutionPath, InstructionProcessor, Prefetch},
        traits::TradeExecutor,
    },
    trading::MiddlewareManager,
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
        Self { instruction_builder, protocol_name }
    }
}

#[async_trait::async_trait]
impl TradeExecutor for GenericTradeExecutor {
    async fn swap(&self, params: SwapParams) -> Result<(bool, Signature, Option<anyhow::Error>)> {
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
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name.to_string(),
                    is_buy,
                )?,
            None => instructions,
        };

        // 提交前耗时
        let before_submit_elapsed = total_start.elapsed();

        // 如果是模拟模式，直接通过 RPC 模拟交易
        if params.simulate {
            let send_start = Instant::now();
            let result = simulate_transaction(
                params.rpc,
                params.payer,
                final_instructions,
                params.address_lookup_table_account,
                params.recent_blockhash,
                params.durable_nonce,
                if is_buy { params.data_size_limit } else { 0 },
                params.middleware_manager,
                self.protocol_name,
                is_buy,
                if is_buy { true } else { params.with_tip },
                params.gas_fee_strategy,
            )
            .await;
            let send_elapsed = send_start.elapsed();
            let total_elapsed = total_start.elapsed();

            // Get performance metrics using fast timestamp
            let timestamp_ns = SYSCALL_BYPASS.fast_timestamp_nanos();

            // Print all timing metrics at once to avoid blocking critical path
            println!("[Timestamp] {}ns", timestamp_ns);
            println!(
                "[Build Instructions] Time: {:.3}ms ({:.0}μs)",
                build_elapsed.as_micros() as f64 / 1000.0,
                build_elapsed.as_micros()
            );
            println!(
                "[Before Submit] {:.3}ms ({:.0}μs)",
                before_submit_elapsed.as_micros() as f64 / 1000.0,
                before_submit_elapsed.as_micros()
            );
            println!(
                "[Simulate Transaction] Time: {:.3}ms ({:.0}μs)",
                send_elapsed.as_micros() as f64 / 1000.0,
                send_elapsed.as_micros()
            );
            println!(
                "[Total Time] {:.3}ms ({:.0}μs)",
                total_elapsed.as_micros() as f64 / 1000.0,
                total_elapsed.as_micros()
            );

            return result;
        }

        // 并行发送交易
        let send_start = Instant::now();
        let result = execute_parallel(
            params.swqos_clients.clone(),
            params.payer,
            params.rpc,
            final_instructions,
            params.address_lookup_table_account,
            params.recent_blockhash,
            params.durable_nonce,
            if is_buy { params.data_size_limit } else { 0 },
            params.middleware_manager,
            self.protocol_name,
            is_buy,
            params.wait_transaction_confirmed,
            if is_buy { true } else { params.with_tip },
            params.gas_fee_strategy,
        )
        .await;
        let send_elapsed = send_start.elapsed();
        let total_elapsed = total_start.elapsed();

        // Get performance metrics using fast timestamp
        let timestamp_ns = SYSCALL_BYPASS.fast_timestamp_nanos();

        // Print all timing metrics at once to avoid blocking critical path
        println!("[Timestamp] {}ns", timestamp_ns);
        println!(
            "[Build Instructions] Time: {:.3}ms ({:.0}μs)",
            build_elapsed.as_micros() as f64 / 1000.0,
            build_elapsed.as_micros()
        );
        println!(
            "[Before Submit] {:.3}ms ({:.0}μs)",
            before_submit_elapsed.as_micros() as f64 / 1000.0,
            before_submit_elapsed.as_micros()
        );
        println!(
            "[Send Transaction] Time: {:.3}ms ({:.0}μs)",
            send_elapsed.as_micros() as f64 / 1000.0,
            send_elapsed.as_micros()
        );
        println!(
            "[Total Time] {:.3}ms ({:.0}μs)",
            total_elapsed.as_micros() as f64 / 1000.0,
            total_elapsed.as_micros()
        );

        result
    }

    fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }
}

/// Simulate transaction using RPC client
async fn simulate_transaction(
    rpc: Option<Arc<SolanaRpcClient>>,
    payer: Arc<Keypair>,
    instructions: Vec<Instruction>,
    address_lookup_table_account: Option<AddressLookupTableAccount>,
    recent_blockhash: Option<Hash>,
    durable_nonce: Option<DurableNonceInfo>,
    data_size_limit: u32,
    middleware_manager: Option<Arc<MiddlewareManager>>,
    protocol_name: &'static str,
    is_buy: bool,
    with_tip: bool,
    gas_fee_strategy: GasFeeStrategy,
) -> Result<(bool, Signature, Option<anyhow::Error>)> {
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

    // Build transaction for simulation
    let transaction = build_transaction(
        payer.clone(),
        Some(rpc.clone()),
        unit_limit,
        unit_price,
        instructions,
        address_lookup_table_account,
        recent_blockhash,
        data_size_limit,
        middleware_manager,
        protocol_name,
        is_buy,
        false, // simulate doesn't need tip instruction
        &Pubkey::default(),
        tip,
        durable_nonce,
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
        println!("\n========== [Simulation Failed] ==========");
        println!("Error Type: {:?}", err);
        println!("Signature: {:?}", signature);

        // Print logs
        if let Some(logs) = simulate_result.value.logs {
            println!("\n========== Transaction Logs ==========");
            for (i, log) in logs.iter().enumerate() {
                println!("{:3}. {}", i + 1, log);
            }
        }

        // Print account usage
        if let Some(units_consumed) = simulate_result.value.units_consumed {
            println!("\n========== Resource Consumption ==========");
            println!("Compute Units Consumed: {}", units_consumed);
        }

        println!("=========================================\n");
        return Ok((false, signature, Some(anyhow::anyhow!("{:?}", err))));
    }

    // Simulation succeeded
    println!("\n========== [Simulation Succeeded] ==========");
    println!("Signature: {:?}", signature);

    if let Some(units_consumed) = simulate_result.value.units_consumed {
        println!("Compute Units Consumed: {}", units_consumed);
    }

    if let Some(logs) = simulate_result.value.logs {
        println!("\n========== Transaction Logs ==========");
        for (i, log) in logs.iter().enumerate() {
            println!("{:3}. {}", i + 1, log);
        }
    }

    println!("============================================\n");

    Ok((true, signature, None))
}
