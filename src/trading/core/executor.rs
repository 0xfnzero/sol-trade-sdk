use anyhow::{anyhow, Result};
use std::sync::Arc;

use super::{
    parallel::parallel_execute_with_tips,
    params::{BuyParams, BuyWithTipParams, SellParams, SellWithTipParams},
    timer::TradeTimer,
    traits::{InstructionBuilder, TradeExecutor},
};
use crate::{
    swqos::TradeType,
    trading::common::{build_rpc_transaction, build_sell_transaction},
};

const MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT: u32 = 256 * 1024;

/// 通用交易执行器实现
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
    async fn buy(&self, mut params: BuyParams) -> Result<()> {
        if params.data_size_limit == 0 {
            params.data_size_limit = MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT;
        }
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();
        let mut timer = TradeTimer::new("构建买入交易指令");
        // Build buy transaction instructions
        // 构建指令
        let instructions = self
            .instruction_builder
            .build_buy_instructions(&params)
            .await?;
        timer.stage("构建rpc交易指令");
        // Build RPC transaction instructions

        // 构建交易
        // Build transaction
        let transaction = build_rpc_transaction(
            params.payer.clone(),
            &params.priority_fee,
            instructions,
            params.lookup_table_key,
            params.recent_blockhash,
            params.data_size_limit,
        )
        .await?;
        timer.stage("rpc提交确认");
        // RPC submit confirmation

        // 发送交易
        // Send transaction
        rpc.send_and_confirm_transaction(&transaction).await?;
        timer.finish();

        Ok(())
    }

    async fn buy_with_tip(&self, mut params: BuyWithTipParams) -> Result<()> {
        if params.data_size_limit == 0 {
            params.data_size_limit = MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT;
        }
        let timer = TradeTimer::new("构建买入交易指令");
        // Build buy transaction instructions

        // 验证参数 - 转换为BuyParams进行验证
        // Validate parameters - convert to BuyParams for validation
        let buy_params = BuyParams {
            rpc: params.rpc,
            payer: params.payer.clone(),
            mint: params.mint,
            creator: params.creator,
            sol_amount: params.sol_amount,
            slippage_basis_points: params.slippage_basis_points,
            priority_fee: params.priority_fee.clone(),
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            data_size_limit: params.data_size_limit,
            protocol_params: params.protocol_params.clone(),
        };

        // 构建指令
        // Build instructions
        let instructions = self
            .instruction_builder
            .build_buy_instructions(&buy_params)
            .await?;

        timer.finish();

        // 并行执行交易
        // Execute transaction in parallel
        parallel_execute_with_tips(
            params.swqos_clients,
            params.payer,
            instructions,
            params.priority_fee,
            params.lookup_table_key,
            params.recent_blockhash,
            params.data_size_limit,
            TradeType::Buy,
        )
        .await?;

        Ok(())
    }

    async fn sell(&self, params: SellParams) -> Result<()> {
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();
        let mut timer = TradeTimer::new("构建卖出交易指令");
        // Build sell transaction instructions

        // 构建指令
        // Build instructions
        let instructions = self
            .instruction_builder
            .build_sell_instructions(&params)
            .await?;
        timer.stage("卖出交易指令");
        // Sell transaction instructions

        // 构建交易
        // Build transaction
        let transaction = build_sell_transaction(
            params.payer.clone(),
            &params.priority_fee,
            instructions,
            params.lookup_table_key,
            params.recent_blockhash,
        )
        .await?;
        timer.stage("卖出交易签名");
        // Sell transaction signing

        // 发送交易
        // Send transaction
        rpc.send_and_confirm_transaction(&transaction).await?;
        timer.finish();

        Ok(())
    }

    async fn sell_with_tip(&self, params: SellWithTipParams) -> Result<()> {
        let timer = TradeTimer::new("构建卖出交易指令");
        // Build sell transaction instructions

        // 转换为SellParams进行指令构建
        // Convert to SellParams for instruction building
        let sell_params = SellParams {
            rpc: params.rpc,
            payer: params.payer.clone(),
            mint: params.mint,
            creator: params.creator,
            token_amount: params.token_amount,
            slippage_basis_points: params.slippage_basis_points,
            priority_fee: params.priority_fee.clone(),
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            protocol_params: params.protocol_params.clone(),
        };

        // 构建指令
        // Build instructions
        let instructions = self
            .instruction_builder
            .build_sell_instructions(&sell_params)
            .await?;

        timer.finish();

        // 并行执行交易
        // Execute transaction in parallel
        parallel_execute_with_tips(
            params.swqos_clients,
            params.payer,
            instructions,
            params.priority_fee,
            params.lookup_table_key,
            params.recent_blockhash,
            0,
            TradeType::Sell,
        )
        .await?;

        Ok(())
    }

    fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }
}
