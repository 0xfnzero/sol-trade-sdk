use crate::swqos::{SwqosType, TradeType};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GasFeeStrategyType {
    Default,
    LowTipHighCuPrice,
    HighTipLowCuPrice,
}

impl GasFeeStrategyType {
    pub fn values() -> Vec<Self> {
        vec![Self::Default, Self::LowTipHighCuPrice, Self::HighTipLowCuPrice]
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GasFeeStrategyValue {
    pub cu_limit: u32,
    pub cu_price: u64,
    pub tip: f64,
}

// 静态存储策略数据
static STRATEGIES: LazyLock<
    ArcSwap<HashMap<(SwqosType, TradeType, GasFeeStrategyType), GasFeeStrategyValue>>,
> = LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

pub struct GasFeeStrategy;

impl GasFeeStrategy {
    pub fn add_high_low_fee_strategies(
        swqos_types: &[SwqosType],
        trade_type: TradeType,
        cu_limit: u32,
        low_cu_price: u64,
        high_cu_price: u64,
        low_tip: f64,
        high_tip: f64,
    ) {
        for swqos_type in swqos_types {
            GasFeeStrategy::remove_strategy(*swqos_type, trade_type);
        }
        STRATEGIES.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            for swqos_type in swqos_types {
                if swqos_type.eq(&SwqosType::Default) {
                    continue;
                }
                new_map.insert(
                    (*swqos_type, trade_type, GasFeeStrategyType::LowTipHighCuPrice),
                    GasFeeStrategyValue { cu_limit, cu_price: high_cu_price, tip: low_tip },
                );
                new_map.insert(
                    (*swqos_type, trade_type, GasFeeStrategyType::HighTipLowCuPrice),
                    GasFeeStrategyValue { cu_limit, cu_price: low_cu_price, tip: high_tip },
                );
            }
            Arc::new(new_map)
        });
    }

    pub fn add_high_low_fee_strategy(
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
        GasFeeStrategy::remove_strategy(swqos_type, trade_type);
        STRATEGIES.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.insert(
                (swqos_type, trade_type, GasFeeStrategyType::LowTipHighCuPrice),
                GasFeeStrategyValue { cu_limit, cu_price: high_cu_price, tip: low_tip },
            );
            new_map.insert(
                (swqos_type, trade_type, GasFeeStrategyType::HighTipLowCuPrice),
                GasFeeStrategyValue { cu_limit, cu_price: low_cu_price, tip: high_tip },
            );
            Arc::new(new_map)
        });
    }

    pub fn add_default_fee_strategies(
        swqos_types: &[SwqosType],
        trade_type: TradeType,
        cu_price: u64,
        tip: f64,
        cu_limit: u32,
    ) {
        for swqos_type in swqos_types {
            GasFeeStrategy::remove_strategy(*swqos_type, trade_type);
        }
        STRATEGIES.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            for swqos_type in swqos_types {
                new_map.insert(
                    (*swqos_type, trade_type, GasFeeStrategyType::Default),
                    GasFeeStrategyValue { cu_limit, cu_price, tip },
                );
            }
            Arc::new(new_map)
        });
    }

    pub fn add_default_fee_strategy(
        swqos_type: SwqosType,
        trade_type: TradeType,
        cu_price: u64,
        tip: f64,
        cu_limit: u32,
    ) {
        GasFeeStrategy::remove_strategy(swqos_type, trade_type);
        STRATEGIES.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.insert(
                (swqos_type, trade_type, GasFeeStrategyType::Default),
                GasFeeStrategyValue { cu_limit, cu_price, tip },
            );
            Arc::new(new_map)
        });
    }

    pub fn remove_strategy(swqos_type: SwqosType, trade_type: TradeType) {
        STRATEGIES.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::Default));
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::LowTipHighCuPrice));
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::HighTipLowCuPrice));
            Arc::new(new_map)
        });
    }

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

    pub fn clear() {
        STRATEGIES.store(Arc::new(HashMap::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo() {
        // 给SwqosType::Default 在 Buy 时添加默认策略
        GasFeeStrategy::add_default_fee_strategy(
            SwqosType::Default,
            TradeType::Buy,
            100,
            0.0001,
            10,
        );
        // 给SwqosType::Jito 在 Buy 时添加高低价策略
        GasFeeStrategy::add_high_low_fee_strategy(
            SwqosType::Jito,
            TradeType::Buy,
            10,
            100,
            10000,
            0.001,
            0.1,
        );
        // 获取所有 Buy 策略
        let all_strategies = GasFeeStrategy::get_strategies(TradeType::Buy);
        for strategy in all_strategies {
            println!("strategy: {:?}", strategy);
        }
        println!("--------------------------------");
        // 给SwqosType::Jito 在 Buy 时添加默认策略（会删除 jito 的高低价策略）
        GasFeeStrategy::add_default_fee_strategy(SwqosType::Jito, TradeType::Buy, 100, 0.0001, 10);
        // 获取所有 Buy 策略
        let all_strategies = GasFeeStrategy::get_strategies(TradeType::Buy);
        for strategy in all_strategies {
            println!("strategy: {:?}", strategy);
        }
        // 删除SwqosType::Jito 在 Buy 时的策略
        GasFeeStrategy::remove_strategy(SwqosType::Jito, TradeType::Buy);
        // 清空策略
        GasFeeStrategy::clear();
        println!("--------------------------------");
        println!("strategy {:?}", GasFeeStrategy::get_strategies(TradeType::Buy));
    }
}
