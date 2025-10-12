use anyhow::Result;
use solana_sdk::signature::Signature;
use std::{sync::Arc, time::Instant};

use crate::trading::core::{
    parallel::{buy_parallel_execute, sell_parallel_execute},
    traits::TradeExecutor,
};

use super::{params::SwapParams, traits::InstructionBuilder};

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
    async fn swap(&self, params: SwapParams) -> Result<(bool, Signature)> {
        let start = Instant::now();
        // 暂时支持这三种。后续重构扩展builder 支持所有的 swap
        let is_buy = params.input_mint == crate::constants::SOL_TOKEN_ACCOUNT
            || params.input_mint == crate::constants::WSOL_TOKEN_ACCOUNT
            || params.input_mint == crate::constants::USDC_TOKEN_ACCOUNT
            || (params.input_mint == crate::constants::USD1_TOKEN_ACCOUNT
                && params.output_mint != crate::constants::WSOL_TOKEN_ACCOUNT);
        // Build instructions directly from params to avoid unnecessary cloning
        let instructions = if is_buy {
            self.instruction_builder.build_buy_instructions(&params).await?
        } else {
            self.instruction_builder.build_sell_instructions(&params).await?
        };
        let final_instructions = match &params.middleware_manager {
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name.to_string(),
                    is_buy,
                )?,
            None => instructions,
        };
        println!("Building swap transaction instructions time cost: {:?}", start.elapsed());
        // Execute transactions in parallel
        if is_buy {
            buy_parallel_execute(params, final_instructions, self.protocol_name).await
        } else {
            sell_parallel_execute(params, final_instructions, self.protocol_name).await
        }
    }

    fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }
}
