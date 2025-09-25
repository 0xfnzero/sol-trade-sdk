use crate::trading::SwapParams;
use anyhow::Result;
use solana_sdk::{instruction::Instruction, signature::Signature};

/// 交易执行器trait - 定义了所有交易协议都需要实现的核心方法
#[async_trait::async_trait]
pub trait TradeExecutor: Send + Sync {
    async fn swap(&self, params: SwapParams) -> Result<(bool, Signature)>;
    /// 获取协议名称
    fn protocol_name(&self) -> &'static str;
}

/// 指令构建器trait - 负责构建协议特定的交易指令
#[async_trait::async_trait]
pub trait InstructionBuilder: Send + Sync {
    /// 构建买入指令
    async fn build_buy_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>>;

    /// 构建卖出指令
    async fn build_sell_instructions(&self, params: &SwapParams) -> Result<Vec<Instruction>>;
}

/// 协议特定参数trait - 允许每个协议定义自己的参数
pub trait ProtocolParams: Send + Sync {
    /// 将参数转换为Any以便向下转型
    fn as_any(&self) -> &dyn std::any::Any;

    /// 克隆参数
    fn clone_box(&self) -> Box<dyn ProtocolParams>;
}

impl Clone for Box<dyn ProtocolParams> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
