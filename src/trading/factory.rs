use anyhow::{anyhow, Result};
use std::sync::Arc;

use crate::instruction::{
    bonk::BonkInstructionBuilder, pumpfun::PumpFunInstructionBuilder,
    pumpswap::PumpSwapInstructionBuilder, raydium_cpmm::RaydiumCpmmInstructionBuilder,
};

use super::core::{executor::GenericTradeExecutor, traits::TradeExecutor};

/// 支持的交易协议
/// Supported trading protocols
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DexType {
    PumpFun,
    PumpSwap,
    Bonk,
    RaydiumCpmm,
}

impl std::fmt::Display for DexType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DexType::PumpFun => write!(f, "PumpFun"),
            DexType::PumpSwap => write!(f, "PumpSwap"),
            DexType::Bonk => write!(f, "Bonk"),
            DexType::RaydiumCpmm => write!(f, "RaydiumCpmm"),
        }
    }
}

impl std::str::FromStr for DexType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pumpfun" => Ok(DexType::PumpFun),
            "pumpswap" => Ok(DexType::PumpSwap),
            "bonk" => Ok(DexType::Bonk),
            "raydiumcpmm" => Ok(DexType::RaydiumCpmm),
            _ => Err(anyhow!("Unsupported protocol: {}", s)),
        }
    }
}

/// 交易工厂 - 用于创建不同协议的交易执行器
/// Trade factory - used to create trade executors for different protocols
pub struct TradeFactory;

impl TradeFactory {
    /// 创建指定协议的交易执行器
    /// Create trade executor for specified protocol
    pub fn create_executor(dex_type: DexType) -> Arc<dyn TradeExecutor> {
        match dex_type {
            DexType::PumpFun => {
                let instruction_builder = Arc::new(PumpFunInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "PumpFun"))
            }
            DexType::PumpSwap => {
                let instruction_builder = Arc::new(PumpSwapInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "PumpSwap"))
            }
            DexType::Bonk => {
                let instruction_builder = Arc::new(BonkInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "Bonk"))
            }
            DexType::RaydiumCpmm => {
                let instruction_builder = Arc::new(RaydiumCpmmInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(
                    instruction_builder,
                    "RaydiumCpmm",
                ))
            }
        }
    }

    /// 获取所有支持的协议
    /// Get all supported protocols
    pub fn supported_dex_types() -> Vec<DexType> {
        vec![DexType::PumpFun, DexType::PumpSwap, DexType::Bonk, DexType::RaydiumCpmm]
    }

    /// 检查协议是否支持
    /// Check if protocol is supported
    pub fn is_supported(dex_type: &DexType) -> bool {
        Self::supported_dex_types().contains(dex_type)
    }
}
