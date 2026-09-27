use std::sync::Arc;

use crate::instruction::{
    bonk::BonkInstructionBuilder, meteora_damm_v2::MeteoraDammV2InstructionBuilder,
    meteora_dlmm::MeteoraDlmmInstructionBuilder, pumpfun::PumpFunInstructionBuilder,
    pumpswap::PumpSwapInstructionBuilder, raydium_amm_v4::RaydiumAmmV4InstructionBuilder,
    raydium_clmm::RaydiumClmmInstructionBuilder, raydium_cpmm::RaydiumCpmmInstructionBuilder,
    whirlpool::WhirlpoolInstructionBuilder,
};

use super::core::{executor::GenericTradeExecutor, traits::TradeExecutor};

/// 支持的交易协议
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DexType {
    PumpFun,
    PumpSwap,
    LaunchLab,
    Bonk,
    StonkFun,
    RaydiumCpmm,
    RaydiumAmmV4,
    MeteoraDammV2,
    RaydiumClmm,
    OrcaWhirlpool,
    MeteoraDlmm,
}

/// 交易工厂 - 用于创建不同协议的交易执行器
pub struct TradeFactory;

impl TradeFactory {
    /// 创建指定协议的交易执行器（零开销单例）
    pub fn create_executor(dex_type: DexType) -> Arc<dyn TradeExecutor> {
        match dex_type {
            DexType::PumpFun => Self::pumpfun_executor(),
            DexType::PumpSwap => Self::pumpswap_executor(),
            DexType::LaunchLab => Self::launchlab_executor(),
            DexType::Bonk => Self::bonk_executor(),
            DexType::StonkFun => Self::stonkfun_executor(),
            DexType::RaydiumCpmm => Self::raydium_cpmm_executor(),
            DexType::RaydiumAmmV4 => Self::raydium_amm_v4_executor(),
            DexType::MeteoraDammV2 => Self::meteora_damm_v2_executor(),
            DexType::RaydiumClmm => Self::raydium_clmm_executor(),
            DexType::OrcaWhirlpool => Self::whirlpool_executor(),
            DexType::MeteoraDlmm => Self::meteora_dlmm_executor(),
        }
    }

    #[inline]
    fn pumpfun_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(PumpFunInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "PumpFun"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn pumpswap_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(PumpSwapInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "PumpSwap"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn bonk_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(BonkInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "Bonk"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn launchlab_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(BonkInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "LaunchLab"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn stonkfun_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder =
                    Arc::new(crate::instruction::stonkfun::StonkFunInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "StonkFun"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn raydium_cpmm_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(RaydiumCpmmInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "RaydiumCpmm"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn raydium_amm_v4_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(RaydiumAmmV4InstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "RaydiumAmmV4"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn meteora_damm_v2_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(MeteoraDammV2InstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "MeteoraDammV2"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn raydium_clmm_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(RaydiumClmmInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "RaydiumClmm"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn whirlpool_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(WhirlpoolInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "OrcaWhirlpool"))
            });
        INSTANCE.clone()
    }

    #[inline]
    fn meteora_dlmm_executor() -> Arc<dyn TradeExecutor> {
        static INSTANCE: std::sync::LazyLock<Arc<dyn TradeExecutor>> =
            std::sync::LazyLock::new(|| {
                let instruction_builder = Arc::new(MeteoraDlmmInstructionBuilder);
                Arc::new(GenericTradeExecutor::new(instruction_builder, "MeteoraDlmm"))
            });
        INSTANCE.clone()
    }
}
