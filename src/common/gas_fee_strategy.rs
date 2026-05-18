use crate::swqos::{SwqosType, TradeType};
use arc_swap::ArcSwap;
use std::collections::{HashMap, HashSet};
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

    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::LowTipHighCuPrice => "LowTipHighCuPrice",
            Self::HighTipLowCuPrice => "HighTipLowCuPrice",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GasFeeStrategyValue {
    pub cu_limit: u32,
    pub cu_price: u64,
    pub tip: f64,
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
            );
            self.set(
                swqos_type,
                TradeType::Sell,
                GasFeeStrategyType::Normal,
                sell_cu_limit,
                sell_cu_price,
                sell_tip,
            );
        }
        self.set(
            SwqosType::Default,
            TradeType::Buy,
            GasFeeStrategyType::Normal,
            buy_cu_limit,
            buy_cu_price,
            0.0,
        );
        self.set(
            SwqosType::Default,
            TradeType::Sell,
            GasFeeStrategyType::Normal,
            sell_cu_limit,
            sell_cu_price,
            0.0,
        );
    }

    /// 设置 Default/RPC 的优先费-only 策略。Default 没有 relay tip account，
    /// 但仍应携带 ComputeBudget 优先费。
    pub fn set_default_rpc_fee_strategy(
        &self,
        buy_cu_limit: u32,
        sell_cu_limit: u32,
        buy_cu_price: u64,
        sell_cu_price: u64,
    ) {
        self.set(
            SwqosType::Default,
            TradeType::Buy,
            GasFeeStrategyType::Normal,
            buy_cu_limit,
            buy_cu_price,
            0.0,
        );
        self.set(
            SwqosType::Default,
            TradeType::Sell,
            GasFeeStrategyType::Normal,
            sell_cu_limit,
            sell_cu_price,
            0.0,
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
            );
            self.set(
                *swqos_type,
                trade_type,
                GasFeeStrategyType::HighTipLowCuPrice,
                cu_limit,
                low_cu_price,
                high_tip,
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
        );
        self.set(
            swqos_type,
            trade_type,
            GasFeeStrategyType::HighTipLowCuPrice,
            cu_limit,
            low_cu_price,
            high_tip,
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
            );
            self.set(
                *swqos_type,
                TradeType::Sell,
                GasFeeStrategyType::Normal,
                cu_limit,
                cu_price,
                sell_tip,
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
        );
        self.set(
            swqos_type,
            TradeType::Sell,
            GasFeeStrategyType::Normal,
            cu_limit,
            cu_price,
            sell_tip,
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
                GasFeeStrategyValue { cu_limit, cu_price, tip },
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
        let mut swqos_types = HashSet::new();
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

    /// 动态更新买入小费（保持其他参数不变）
    /// Dynamically update buy tip (keep other parameters unchanged)
    pub fn update_buy_tip(&self, buy_tip: f64) {
        self.update_buy_tip_for_strategy(GasFeeStrategyType::Normal, buy_tip);
    }

    /// 动态更新指定买入策略的小费（保持其他参数不变）。
    /// Dynamic updates should generally target Normal only; updating all strategies would
    /// collapse the low-tip/high-tip dual-lane spread into the same tip.
    pub fn update_buy_tip_for_strategy(&self, strategy_type: GasFeeStrategyType, buy_tip: f64) {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            for ((_swqos_type, trade_type, s_type), value) in new_map.iter_mut() {
                if *trade_type == TradeType::Buy && *s_type == strategy_type {
                    value.tip = buy_tip;
                }
            }
            Arc::new(new_map)
        });
    }

    /// 动态更新卖出小费（保持其他参数不变）
    /// Dynamically update sell tip (keep other parameters unchanged)
    pub fn update_sell_tip(&self, sell_tip: f64) {
        self.update_sell_tip_for_strategy(GasFeeStrategyType::Normal, sell_tip);
    }

    /// 动态更新指定卖出策略的小费（保持其他参数不变）。
    pub fn update_sell_tip_for_strategy(&self, strategy_type: GasFeeStrategyType, sell_tip: f64) {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            for ((_swqos_type, trade_type, s_type), value) in new_map.iter_mut() {
                if *trade_type == TradeType::Sell && *s_type == strategy_type {
                    value.tip = sell_tip;
                }
            }
            Arc::new(new_map)
        });
    }

    /// 动态更新买入优先费（保持其他参数不变）
    /// Dynamically update buy compute unit price (keep other parameters unchanged)
    pub fn update_buy_cu_price(&self, buy_cu_price: u64) {
        self.update_buy_cu_price_for_strategy(GasFeeStrategyType::Normal, buy_cu_price);
    }

    /// 动态更新指定买入策略的优先费（保持其他参数不变）。
    pub fn update_buy_cu_price_for_strategy(
        &self,
        strategy_type: GasFeeStrategyType,
        buy_cu_price: u64,
    ) {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            for ((_swqos_type, trade_type, s_type), value) in new_map.iter_mut() {
                if *trade_type == TradeType::Buy && *s_type == strategy_type {
                    value.cu_price = buy_cu_price;
                }
            }
            Arc::new(new_map)
        });
    }

    /// 动态更新卖出优先费（保持其他参数不变）
    /// Dynamically update sell compute unit price (keep other parameters unchanged)
    pub fn update_sell_cu_price(&self, sell_cu_price: u64) {
        self.update_sell_cu_price_for_strategy(GasFeeStrategyType::Normal, sell_cu_price);
    }

    /// 动态更新指定卖出策略的优先费（保持其他参数不变）。
    pub fn update_sell_cu_price_for_strategy(
        &self,
        strategy_type: GasFeeStrategyType,
        sell_cu_price: u64,
    ) {
        self.strategies.rcu(|current_map| {
            let mut new_map = (**current_map).clone();
            for ((_swqos_type, trade_type, s_type), value) in new_map.iter_mut() {
                if *trade_type == TradeType::Sell && *s_type == strategy_type {
                    value.cu_price = sell_cu_price;
                }
            }
            Arc::new(new_map)
        });
    }

    /// 打印所有策略。
    /// Print all strategies
    pub fn print_all_strategies(&self) {
        if !crate::common::sdk_log::sdk_log_enabled() {
            return;
        }
        for strategy in self.get_strategies(TradeType::Buy) {
            println!("[buy] - {:?}", strategy);
        }
        for strategy in self.get_strategies(TradeType::Sell) {
            println!("[sell] - {:?}", strategy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_strategy(
        strategies: &[(SwqosType, GasFeeStrategyType, GasFeeStrategyValue)],
        swqos_type: SwqosType,
        strategy_type: GasFeeStrategyType,
    ) -> GasFeeStrategyValue {
        strategies
            .iter()
            .find(|(s, t, _)| *s == swqos_type && *t == strategy_type)
            .map(|(_, _, v)| *v)
            .expect("strategy exists")
    }

    #[test]
    fn high_low_fee_strategy_expands_two_lanes_per_swqos() {
        let strategy = GasFeeStrategy::new();

        strategy.set_high_low_fee_strategies(
            &[SwqosType::Jito, SwqosType::Helius],
            TradeType::Buy,
            100_000,
            180_000,
            400_000,
            0.002,
            0.005,
        );

        let strategies = strategy.get_strategies(TradeType::Buy);
        assert_eq!(strategies.len(), 4);

        for swqos_type in [SwqosType::Jito, SwqosType::Helius] {
            let low_tip_high_cu =
                find_strategy(&strategies, swqos_type, GasFeeStrategyType::LowTipHighCuPrice);
            assert_eq!(low_tip_high_cu.cu_limit, 100_000);
            assert_eq!(low_tip_high_cu.cu_price, 400_000);
            assert_eq!(low_tip_high_cu.tip, 0.002);

            let high_tip_low_cu =
                find_strategy(&strategies, swqos_type, GasFeeStrategyType::HighTipLowCuPrice);
            assert_eq!(high_tip_low_cu.cu_limit, 100_000);
            assert_eq!(high_tip_low_cu.cu_price, 180_000);
            assert_eq!(high_tip_low_cu.tip, 0.005);
        }
    }

    #[test]
    fn dynamic_updates_do_not_collapse_dual_lane_fees() {
        let strategy = GasFeeStrategy::new();

        strategy.set_high_low_fee_strategy(
            SwqosType::Jito,
            TradeType::Buy,
            100_000,
            180_000,
            400_000,
            0.002,
            0.005,
        );

        strategy.update_buy_tip(0.009);
        strategy.update_buy_cu_price(999_999);

        let strategies = strategy.get_strategies(TradeType::Buy);
        let low_tip_high_cu =
            find_strategy(&strategies, SwqosType::Jito, GasFeeStrategyType::LowTipHighCuPrice);
        let high_tip_low_cu =
            find_strategy(&strategies, SwqosType::Jito, GasFeeStrategyType::HighTipLowCuPrice);

        assert_eq!(low_tip_high_cu.cu_price, 400_000);
        assert_eq!(low_tip_high_cu.tip, 0.002);
        assert_eq!(high_tip_low_cu.cu_price, 180_000);
        assert_eq!(high_tip_low_cu.tip, 0.005);
    }

    #[test]
    fn default_rpc_strategy_uses_priority_fee_without_tip() {
        let strategy = GasFeeStrategy::new();

        strategy.set_default_rpc_fee_strategy(100_000, 90_000, 700_000, 800_000);

        let buy = find_strategy(
            &strategy.get_strategies(TradeType::Buy),
            SwqosType::Default,
            GasFeeStrategyType::Normal,
        );
        assert_eq!(buy.cu_limit, 100_000);
        assert_eq!(buy.cu_price, 700_000);
        assert_eq!(buy.tip, 0.0);

        let sell = find_strategy(
            &strategy.get_strategies(TradeType::Sell),
            SwqosType::Default,
            GasFeeStrategyType::Normal,
        );
        assert_eq!(sell.cu_limit, 90_000);
        assert_eq!(sell.cu_price, 800_000);
        assert_eq!(sell.tip, 0.0);
    }
}
