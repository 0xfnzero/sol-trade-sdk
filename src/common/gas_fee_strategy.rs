use crate::swqos::{SwqosType, TradeType};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

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

#[derive(Debug)]
pub struct GasFeeStrategy {
    strategies: ArcSwap<HashMap<(SwqosType, TradeType, GasFeeStrategyType), GasFeeStrategyValue>>,
    enabled_types: ArcSwap<HashMap<TradeType, Vec<GasFeeStrategyType>>>,
    swqos_disabled_types: ArcSwap<HashMap<(SwqosType, TradeType), Vec<GasFeeStrategyType>>>,
}

static INSTANCE: OnceLock<Arc<GasFeeStrategy>> = OnceLock::new();

impl GasFeeStrategy {
    pub fn instance() -> Arc<GasFeeStrategy> {
        INSTANCE.get_or_init(|| Arc::new(GasFeeStrategy::new())).clone()
    }

    fn new() -> Self {
        Self {
            strategies: ArcSwap::new(Arc::new(HashMap::new())),
            enabled_types: ArcSwap::new(Arc::new(HashMap::new())),
            swqos_disabled_types: ArcSwap::new(Arc::new(HashMap::new())),
        }
    }

    pub fn add_high_low_fee_strategies(
        &self,
        swqos_types: &[SwqosType],
        trade_type: TradeType,
        cu_limit: u32,
        low_cu_price: u64,
        high_cu_price: u64,
        low_tip: f64,
        high_tip: f64,
    ) -> &Self {
        self.strategies.rcu(|current_map| {
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
        self
    }

    pub fn add_high_low_fee_strategy(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
        cu_limit: u32,
        low_cu_price: u64,
        high_cu_price: u64,
        low_tip: f64,
        high_tip: f64,
    ) -> &Self {
        if swqos_type.eq(&SwqosType::Default) {
            return self;
        }
        self.strategies.rcu(|current_map| {
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
        self
    }

    pub fn add_default_fee_strategies(
        &self,
        swqos_types: &[SwqosType],
        trade_type: TradeType,
        cu_price: u64,
        tip: f64,
        cu_limit: u32,
    ) -> &Self {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            for swqos_type in swqos_types {
                new_map.insert(
                    (*swqos_type, trade_type, GasFeeStrategyType::Default),
                    GasFeeStrategyValue { cu_limit, cu_price, tip },
                );
            }
            Arc::new(new_map)
        });
        self
    }

    pub fn add_default_fee_strategy(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
        cu_price: u64,
        tip: f64,
        cu_limit: u32,
    ) -> &Self {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.insert(
                (swqos_type, trade_type, GasFeeStrategyType::Default),
                GasFeeStrategyValue { cu_limit, cu_price, tip },
            );
            Arc::new(new_map)
        });
        self
    }

    pub fn remove_default_fee_strategy(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
    ) -> &Self {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::Default));
            Arc::new(new_map)
        });
        self
    }

    pub fn remove_high_low_fee_strategy(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
    ) -> &Self {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::LowTipHighCuPrice));
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::HighTipLowCuPrice));
            Arc::new(new_map)
        });
        self
    }

    pub fn get_all_strategies(
        &self,
    ) -> HashMap<(SwqosType, TradeType, GasFeeStrategyType), GasFeeStrategyValue> {
        (**self.strategies.load()).clone()
    }

    pub fn get_strategies(
        &self,
        trade_type: TradeType,
    ) -> Vec<(SwqosType, GasFeeStrategyType, GasFeeStrategyValue)> {
        let strategies = self.strategies.load();
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

    pub fn get_available_strategies(
        &self,
        trade_type: TradeType,
    ) -> Vec<(SwqosType, GasFeeStrategyType, GasFeeStrategyValue)> {
        let strategies = self.strategies.load();
        let enabled_types = self.get_enabled_strategy_types(trade_type);
        let mut result = Vec::new();
        let mut swqos_types = std::collections::HashSet::new();
        for (swqos_type, t_type, _) in strategies.keys() {
            if *t_type == trade_type {
                swqos_types.insert(*swqos_type);
            }
        }
        for swqos_type in swqos_types {
            let disabled_types = self.get_swqos_disabled_strategy_types(swqos_type, trade_type);
            for strategy_type in &enabled_types {
                if disabled_types.contains(strategy_type) {
                    continue;
                }
                if let Some(strategy_value) =
                    strategies.get(&(swqos_type, trade_type, *strategy_type))
                {
                    result.push((swqos_type, *strategy_type, *strategy_value));
                }
            }
        }
        result
    }

    pub fn set_enabled_strategy_types(
        &self,
        trade_type: TradeType,
        types: &[GasFeeStrategyType],
    ) -> &Self {
        self.enabled_types.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.insert(trade_type, types.to_vec());
            Arc::new(new_map)
        });
        self
    }

    pub fn get_enabled_strategy_types(&self, trade_type: TradeType) -> Vec<GasFeeStrategyType> {
        let strategies = self.enabled_types.load();
        (**strategies).get(&trade_type).cloned().unwrap_or_default()
    }

    pub fn set_swqos_disabled_strategy_types(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
        types: &[GasFeeStrategyType],
    ) -> &Self {
        self.swqos_disabled_types.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.insert((swqos_type, trade_type), types.to_vec());
            Arc::new(new_map)
        });
        self
    }

    pub fn get_swqos_disabled_strategy_types(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
    ) -> Vec<GasFeeStrategyType> {
        let disabled_strategies = self.swqos_disabled_types.load();
        (**disabled_strategies).get(&(swqos_type, trade_type)).cloned().unwrap_or_default()
    }

    pub fn clear(&self) -> &Self {
        self.strategies.store(Arc::new(HashMap::new()));
        self.enabled_types.store(Arc::new(HashMap::new()));
        self.swqos_disabled_types.store(Arc::new(HashMap::new()));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo() {
        GasFeeStrategy::instance()
            // 给SwqosType::Default在 Buy 时添加默认策略
            .add_default_fee_strategy(SwqosType::Default, TradeType::Buy, 100, 0.0001, 10)
            // 给SwqosType::Jito在 Buy 时添加高低价策略
            .add_high_low_fee_strategy(SwqosType::Jito, TradeType::Buy, 10, 100, 10000, 0.001, 0.1)
            // 给SwqosType::Jito在 Buy 时添加默认策略
            .add_default_fee_strategy(SwqosType::Jito, TradeType::Buy, 100, 0.0001, 10)
            // 设置在 Buy 时启用策略 - 启用两个高低价策略、默认策略
            .set_enabled_strategy_types(
                TradeType::Buy,
                &[
                    GasFeeStrategyType::HighTipLowCuPrice,
                    GasFeeStrategyType::LowTipHighCuPrice,
                    GasFeeStrategyType::Default,
                ],
            )
            // 设置SwqosType::Jito在 Buy 时禁用策略 - 禁用 （默认策略）
            .set_swqos_disabled_strategy_types(
                SwqosType::Jito,
                TradeType::Buy,
                &[GasFeeStrategyType::Default],
            );
        // 获取在 Buy 时可用的策略
        let strategies = GasFeeStrategy::instance().get_available_strategies(TradeType::Buy);
        println!("strategies: {:?}", strategies);
        // 获取所有 Buy 策略（包括禁用的）
        let all_strategies = GasFeeStrategy::instance().get_strategies(TradeType::Buy);
        println!("all_strategies: {:?}", all_strategies);
    }
}
