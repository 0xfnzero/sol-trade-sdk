use anyhow::Result;
use solana_sdk::instruction::Instruction;
use super::params::{BuyParams, BuyWithTipParams, SellParams, SellWithTipParams};

/// 交易执行器trait - 定义了所有交易协议都需要实现的核心方法
/// Trade executor trait - defines core methods that all trading protocols need to implement
#[async_trait::async_trait]
pub trait TradeExecutor: Send + Sync {
    /// 执行买入交易
    /// Execute buy transaction
    async fn buy(&self, params: BuyParams) -> Result<()>;

    /// 使用MEV服务执行买入交易
    /// Execute buy transaction using MEV service
    async fn buy_with_tip(&self, params: BuyWithTipParams) -> Result<()>;

    /// 执行卖出交易
    /// Execute sell transaction
    async fn sell(&self, params: SellParams) -> Result<()>;

    /// 使用MEV服务执行卖出交易
    /// Execute sell transaction using MEV service
    async fn sell_with_tip(&self, params: SellWithTipParams) -> Result<()>;

    /// 获取协议名称
    /// Get protocol name
    fn protocol_name(&self) -> &'static str;
}

/// 指令构建器trait - 负责构建协议特定的交易指令
/// Instruction builder trait - responsible for building protocol-specific transaction instructions
#[async_trait::async_trait]
pub trait InstructionBuilder: Send + Sync {
    /// 构建买入指令
    /// Build buy instructions
    async fn build_buy_instructions(&self, params: &BuyParams) -> Result<Vec<Instruction>>;

    /// 构建卖出指令
    /// Build sell instructions
    async fn build_sell_instructions(&self, params: &SellParams) -> Result<Vec<Instruction>>;
}

/// 协议特定参数trait - 允许每个协议定义自己的参数
/// Protocol-specific parameters trait - allows each protocol to define its own parameters
pub trait ProtocolParams: Send + Sync {
    /// 将参数转换为Any以便向下转型
    /// Convert parameters to Any for downcasting
    fn as_any(&self) -> &dyn std::any::Any;

    /// 克隆参数
    /// Clone parameters
    fn clone_box(&self) -> Box<dyn ProtocolParams>;
}

impl Clone for Box<dyn ProtocolParams> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}
