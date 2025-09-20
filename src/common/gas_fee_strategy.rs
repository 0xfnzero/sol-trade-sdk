use crate::swqos::{SwqosType, TradeType};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GasFeeStrategyType {
    Normal,
    LowTipHighCuPrice,
    HighTipLowCuPrice,
}

impl GasFeeStrategyType {
    pub fn values() -> Vec<Self> {
        vec![Self::Normal, Self::LowTipHighCuPrice, Self::HighTipLowCuPrice]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GasFeeStrategyValue {
    pub cu_limit: u32,
    pub cu_price: u64,
    pub tip: f64,
}

static STRATEGIES: LazyLock<
    ArcSwap<HashMap<(SwqosType, TradeType, GasFeeStrategyType), GasFeeStrategyValue>>,
> = LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

pub struct GasFeeStrategy;

impl GasFeeStrategy {
    /// 设置全局费率策略
    /// Set global fee strategy
    pub fn set_global_fee_strategy(cu_limit: u32, cu_price: u64, buy_tip: f64, sell_tip: f64) {
        for swqos_type in SwqosType::values() {
            if swqos_type.eq(&SwqosType::Default) {
                continue;
            }
            GasFeeStrategy::set(swqos_type, TradeType::Buy, cu_limit, cu_price, buy_tip);
            GasFeeStrategy::set(swqos_type, TradeType::Sell, cu_limit, cu_price, sell_tip);
        }

        GasFeeStrategy::set_normal_fee_strategy(
            SwqosType::Default,
            cu_limit,
            cu_price,
            0.0,
            0.0,
        );
    }

    /// 为多个服务类型添加高低费率策略，会移除(SwqosType,TradeType)的默认策略。
    /// Add high-low fee strategies for multiple service types, Will remove the default strategy of (SwqosType,TradeType)
    pub fn set_high_low_fee_strategies(
        swqos_types: &[SwqosType],
        trade_type: TradeType,
        cu_limit: u32,
        low_cu_price: u64,
        high_cu_price: u64,
        low_tip: f64,
        high_tip: f64,
    ) {
        for swqos_type in swqos_types {
            GasFeeStrategy::del(*swqos_type, trade_type);
            GasFeeStrategy::set(*swqos_type, trade_type, cu_limit, high_cu_price, low_tip);
            GasFeeStrategy::set(*swqos_type, trade_type, cu_limit, low_cu_price, high_tip);
        }
    }
    
    /// 为单个服务类型添加高低费率策略，会移除(SwqosType,TradeType)的默认策略。
    /// Add high-low fee strategy for a single service type, Will remove the default strategy of (SwqosType,TradeType)
    pub fn set_high_low_fee_strategy(
        swqos_type: SwqosType,
        trade_type: TradeType,
        cu_limit: u32,
        low_cu_price: u64,
        high_cu_price: u64,
        low_tip: f64,
        high_tip: f64,
    ) {
        if swqos_type.eq(&SwqosType::Default) {
            return;
        }
        GasFeeStrategy::del(swqos_type, trade_type);
        GasFeeStrategy::set(swqos_type, trade_type, cu_limit, high_cu_price, low_tip);
        GasFeeStrategy::set(swqos_type, trade_type, cu_limit, low_cu_price, high_tip);
    }

    /// 为多个服务类型添加标准费率策略，会移除(SwqosType,TradeType)的高低价策略。
    /// Add normal fee strategies for multiple service types, Will remove the high-low strategies of (SwqosType,TradeType)
    pub fn set_normal_fee_strategies(
        swqos_types: &[SwqosType],
        trade_type: TradeType,
        cu_limit: u32,
        cu_price: u64,
        tip: f64,
    ) {
        for swqos_type in swqos_types {
            GasFeeStrategy::del(*swqos_type, trade_type);
            GasFeeStrategy::set(*swqos_type, trade_type, cu_limit, cu_price, tip);
        }
    }

    pub fn set_normal_fee_strategy(
        swqos_type: SwqosType,
        cu_limit: u32,
        cu_price: u64,
        buy_tip: f64,
        sell_tip: f64,
    ) {
        GasFeeStrategy::set(swqos_type, TradeType::Buy, cu_limit, cu_price, buy_tip);
        GasFeeStrategy::set(swqos_type, TradeType::Sell, cu_limit, cu_price, sell_tip);
    }

    fn set(
        swqos_type: SwqosType,
        trade_type: TradeType,
        cu_limit: u32,
        cu_price: u64,
        tip: f64,
    ) {
        GasFeeStrategy::del(swqos_type, trade_type);
        STRATEGIES.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.insert(
                (swqos_type, trade_type, GasFeeStrategyType::Normal),
                GasFeeStrategyValue { cu_limit, cu_price, tip },
            );
            Arc::new(new_map)
        });
    }

    /// 移除指定(SwqosType,TradeType)的策略。
    /// Remove strategy for specified (SwqosType,TradeType)
    fn del(swqos_type: SwqosType, trade_type: TradeType) {
        STRATEGIES.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::Normal));
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::LowTipHighCuPrice));
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::HighTipLowCuPrice));
            Arc::new(new_map)
        });
    }

    /// 获取指定交易类型的所有策略。
    /// Get all strategies for specified trade type
    pub fn get_strategies(
        trade_type: TradeType,
    ) -> Vec<(SwqosType, GasFeeStrategyType, GasFeeStrategyValue)> {
        let strategies = STRATEGIES.load();
        let mut result = Vec::new();
        let mut swqos_types = std::collections::HashSet::new();
        for (swqos_type, t_type, _) in strategies.keys() {
            if *t_type == trade_type {
                swqos_types.insert(*swqos_type);
            }
        }
        for swqos_type in swqos_types {
            for strategy_type in GasFeeStrategyType::values() {
                if let Some(strategy_value) =
                    strategies.get(&(swqos_type, trade_type, strategy_type))
                {
                    result.push((swqos_type, strategy_type, *strategy_value));
                }
            }
        }
        result
    }

    /// 清空所有策略。
    /// Clear all strategies
    pub fn clear() {
        STRATEGIES.store(Arc::new(HashMap::new()));
    }

    /// 打印所有策略。
    /// Print all strategies
    pub fn print_all_strategies() {
        for strategy in GasFeeStrategy::get_strategies(TradeType::Buy) {
            println!("[buy] - {:?}", strategy);
        }
        for strategy in GasFeeStrategy::get_strategies(TradeType::Sell) {
            println!("[sell] - {:?}", strategy);
        }
    }
}
