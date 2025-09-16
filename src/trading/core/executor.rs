use anyhow::Result;
use solana_sdk::signature::Signature;
use std::{sync::Arc, time::Instant};

use crate::trading::core::parallel::{buy_parallel_execute, sell_parallel_execute};

use super::{
    params::{InternalBuyParams, InternalSellParams},
    traits::{InstructionBuilder, TradeExecutor},
};

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
    async fn buy_with_tip(&self, params: InternalBuyParams) -> Result<Signature> {
        let start = Instant::now();

        // Build instructions directly from params to avoid unnecessary cloning
        let instructions = self.instruction_builder.build_buy_instructions(&params).await?;
        let final_instructions = match &params.middleware_manager {
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name.to_string(),
                    true,
                )?,
            None => instructions,
        };

        println!("Building buy transaction instructions time cost: {:?}", start.elapsed());

        // Execute transactions in parallel
        buy_parallel_execute(params, final_instructions, self.protocol_name).await
    }

    async fn sell_with_tip(&self, params: InternalSellParams) -> Result<Signature> {
        let start = Instant::now();

        // Build instructions directly from params to avoid unnecessary cloning
        let instructions = self.instruction_builder.build_sell_instructions(&params).await?;
        let final_instructions = match &params.middleware_manager {
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name.to_string(),
                    false,
                )?,
            None => instructions,
        };

        println!("Building sell transaction instructions time cost: {:?}", start.elapsed());

        // Execute transactions in parallel
        sell_parallel_execute(params, final_instructions, self.protocol_name).await
    }

    fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }
}
