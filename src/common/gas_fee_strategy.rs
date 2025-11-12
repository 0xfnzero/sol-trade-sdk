use crate::swqos::{SwqosType, TradeType};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::Arc;

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
    pub data_size_limit: u32,
}

#[derive(Clone)]
pub struct GasFeeStrategy {
    strategies:
        Arc<ArcSwap<HashMap<(SwqosType, TradeType, GasFeeStrategyType), GasFeeStrategyValue>>>,
}

impl GasFeeStrategy {
    pub fn new() -> Self {
        Self { strategies: Arc::new(ArcSwap::from_pointee(HashMap::new())) }
    }

    /// 设置全局费率策略
    /// Set global fee strategy
    pub fn set_global_fee_strategy(
        &self,
        buy_cu_limit: u32,
        sell_cu_limit: u32,
        buy_cu_price: u64,
        sell_cu_price: u64,
        buy_tip: f64,
        sell_tip: f64,
        buy_data_size_limit: u32,
        sell_data_size_limit: u32,
    ) {
        for swqos_type in SwqosType::values() {
            if swqos_type.eq(&SwqosType::Default) {
                continue;
            }
            self.set(
                swqos_type,
                TradeType::Buy,
                GasFeeStrategyType::Normal,
                buy_cu_limit,
                buy_cu_price,
                buy_tip,
                buy_data_size_limit,
            );
            self.set(
                swqos_type,
                TradeType::Sell,
                GasFeeStrategyType::Normal,
                sell_cu_limit,
                sell_cu_price,
                sell_tip,
                sell_data_size_limit,
            );
        }
        self.set(
            SwqosType::Default,
            TradeType::Buy,
            GasFeeStrategyType::Normal,
            buy_cu_limit,
            buy_cu_price,
            0.0,
            buy_data_size_limit,
        );
        self.set(
            SwqosType::Default,
            TradeType::Sell,
            GasFeeStrategyType::Normal,
            sell_cu_limit,
            sell_cu_price,
            0.0,
            sell_data_size_limit,
        );
    }

    /// 为多个服务类型添加高低费率策略，会移除(SwqosType,TradeType)的默认策略。
    /// Add high-low fee strategies for multiple service types, Will remove the default strategy of (SwqosType,TradeType)
    pub fn set_high_low_fee_strategies(
        &self,
        swqos_types: &[SwqosType],
        trade_type: TradeType,
        cu_limit: u32,
        low_cu_price: u64,
        high_cu_price: u64,
        low_tip: f64,
        high_tip: f64,
        data_size_limit: u32,
    ) {
        for swqos_type in swqos_types {
            self.del(*swqos_type, trade_type, GasFeeStrategyType::Normal);
            self.set(
                *swqos_type,
                trade_type,
                GasFeeStrategyType::LowTipHighCuPrice,
                cu_limit,
                high_cu_price,
                low_tip,
                data_size_limit,
            );
            self.set(
                *swqos_type,
                trade_type,
                GasFeeStrategyType::HighTipLowCuPrice,
                cu_limit,
                low_cu_price,
                high_tip,
                data_size_limit,
            );
        }
    }

    /// 为单个服务类型添加高低费率策略，会移除(SwqosType,TradeType)的默认策略。
    /// Add high-low fee strategy for a single service type, Will remove the default strategy of (SwqosType,TradeType)
    pub fn set_high_low_fee_strategy(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
        cu_limit: u32,
        low_cu_price: u64,
        high_cu_price: u64,
        low_tip: f64,
        high_tip: f64,
        data_size_limit: u32,
    ) {
        if swqos_type.eq(&SwqosType::Default) {
            return;
        }
        self.del(swqos_type, trade_type, GasFeeStrategyType::Normal);
        self.set(
            swqos_type,
            trade_type,
            GasFeeStrategyType::LowTipHighCuPrice,
            cu_limit,
            high_cu_price,
            low_tip,
            data_size_limit,
        );
        self.set(
            swqos_type,
            trade_type,
            GasFeeStrategyType::HighTipLowCuPrice,
            cu_limit,
            low_cu_price,
            high_tip,
            data_size_limit,
        );
    }

    /// 为多个服务类型添加标准费率策略，会移除(SwqosType,TradeType)的高低价策略。
    /// Add normal fee strategies for multiple service types, Will remove the high-low strategies of (SwqosType,TradeType)
    pub fn set_normal_fee_strategies(
        &self,
        swqos_types: &[SwqosType],
        cu_limit: u32,
        cu_price: u64,
        buy_tip: f64,
        sell_tip: f64,
        buy_data_size_limit: u32,
        sell_data_size_limit: u32,
    ) {
        for swqos_type in swqos_types {
            self.del_all(*swqos_type, TradeType::Buy);
            self.del_all(*swqos_type, TradeType::Sell);
            self.set(
                *swqos_type,
                TradeType::Buy,
                GasFeeStrategyType::Normal,
                cu_limit,
                cu_price,
                buy_tip,
                buy_data_size_limit,
            );
            self.set(
                *swqos_type,
                TradeType::Sell,
                GasFeeStrategyType::Normal,
                cu_limit,
                cu_price,
                sell_tip,
                sell_data_size_limit,
            );
        }
    }

    pub fn set_normal_fee_strategy(
        &self,
        swqos_type: SwqosType,
        cu_limit: u32,
        cu_price: u64,
        buy_tip: f64,
        sell_tip: f64,
        buy_data_size_limit: u32,
        sell_data_size_limit: u32,
    ) {
        self.del_all(swqos_type, TradeType::Buy);
        self.del_all(swqos_type, TradeType::Sell);
        self.set(
            swqos_type,
            TradeType::Buy,
            GasFeeStrategyType::Normal,
            cu_limit,
            cu_price,
            buy_tip,
            buy_data_size_limit,
        );
        self.set(
            swqos_type,
            TradeType::Sell,
            GasFeeStrategyType::Normal,
            cu_limit,
            cu_price,
            sell_tip,
            sell_data_size_limit,
        );
    }

    pub fn set(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
        strategy_type: GasFeeStrategyType,
        cu_limit: u32,
        cu_price: u64,
        tip: f64,
        data_size_limit: u32,
    ) {
        if strategy_type == GasFeeStrategyType::Normal {
            self.del(swqos_type, trade_type, GasFeeStrategyType::HighTipLowCuPrice);
            self.del(swqos_type, trade_type, GasFeeStrategyType::LowTipHighCuPrice);
        } else {
            self.del(swqos_type, trade_type, GasFeeStrategyType::Normal);
        }
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.insert(
                (swqos_type, trade_type, strategy_type),
                GasFeeStrategyValue { cu_limit, cu_price, tip, data_size_limit },
            );
            Arc::new(new_map)
        });
    }

    /// 移除指定(SwqosType,TradeType)的策略。
    /// Remove strategy for specified (SwqosType,TradeType)
    pub fn del_all(&self, swqos_type: SwqosType, trade_type: TradeType) {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::Normal));
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::LowTipHighCuPrice));
            new_map.remove(&(swqos_type, trade_type, GasFeeStrategyType::HighTipLowCuPrice));
            Arc::new(new_map)
        });
    }

    /// 移除指定(SwqosType,TradeType,GasFeeStrategyType)的策略。
    /// Remove strategy for specified (SwqosType,TradeType,GasFeeStrategyType)
    pub fn del(
        &self,
        swqos_type: SwqosType,
        trade_type: TradeType,
        strategy_type: GasFeeStrategyType,
    ) {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            new_map.remove(&(swqos_type, trade_type, strategy_type));
            Arc::new(new_map)
        });
    }

    /// 获取指定交易类型的所有策略。
    /// Get all strategies for specified trade type
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

    /// 清空所有策略。
    /// Clear all strategies
    pub fn clear(&self) {
        self.strategies.store(Arc::new(HashMap::new()));
    }

    /// 打印所有策略。
    /// Print all strategies
    pub fn print_all_strategies(&self) {
        for strategy in self.get_strategies(TradeType::Buy) {
            println!("[buy] - {:?}", strategy);
        }
        for strategy in self.get_strategies(TradeType::Sell) {
            println!("[sell] - {:?}", strategy);
        }
    }
}
