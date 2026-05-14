use crate::swqos::SwqosType;
use crate::trading::SwapParams;
use anyhow::Result;
use solana_sdk::{instruction::Instruction, signature::Signature};
/// 交易执行器trait - 定义了所有交易协议都需要实现的核心方法
#[async_trait::async_trait]
pub trait TradeExecutor: Send + Sync {
    /// 🔧 修复：返回Vec<Signature>支持多SWQOS并发交易
    /// - bool: 是否至少有一个交易成功
    /// - Vec<Signature>: 所有提交的交易签名（按SWQOS顺序）
    /// - Option<anyhow::Error>: 最后一个错误（如果全部失败）
    async fn swap(
        &self,
        params: SwapParams,
    ) -> Result<(bool, Vec<Signature>, Option<anyhow::Error>, Vec<(SwqosType, i64)>)>;
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
