# 📊 Gas Fee 策略指南

本文档介绍 Sol Trade SDK 中的 Gas Fee 策略配置和使用方法。

## 基础使用

### 1. 说明

该模块支持用户配置 SwqosType 在不同 TradeType(buy/sell) 下的策略。

- normal 策略: 一个 SwqosType 发送一笔交易，指定 cu_limit、cu_price 和小费。
- 高低费率策略: 一个 SwqosType 同时发送两笔交易，一笔低小费高优先费，一笔高小费低优先费。

每个 (SwqosType, TradeType) 的组合仅可配置一个策略。后续配置的策略会覆盖之前的策略。

### 2. 创建 GasFeeStrategy 实例

```rust
use sol_trade_sdk::common::GasFeeStrategy;

// 创建一个新的 GasFeeStrategy 实例
let gas_fee_strategy = GasFeeStrategy::new();
```

### 3. 设置全局策略(也可以不设置,单独去配置单个策略)

```rust
// 设置全局策略(normal 策略)
gas_fee_strategy.set_global_fee_strategy(
    150000, // cu_limit
    500000, // cu_price
    0.001,  // buy tip
    0.001   // sell tip
);
```

### 4. 配置单个策略

```rust
// 为 SwqosType::Jito 配置 normal 策略
gas_fee_strategy.set_normal_fee_strategy(
    SwqosType::Jito,
    xxxxx, // cu_limit
    xxxx,  // cu_price
    xxxxx, // buy_tip
    xxxxx  // sell_tip
);
```

### 5. 配置高低费率策略

```rust
// 为 SwqosType::Jito 在 Buy 时配置高低费率策略
gas_fee_strategy.set_high_low_fee_strategy(
    SwqosType::Jito,
    TradeType::Buy,
    xxxxx, // cu_limit
    xxxxx, // low cu_price
    xxxxx, // high cu_price
    xxxxx, // low tip
    xxxxx  // high tip
);
```

### 6. 在交易参数中使用

```rust
use sol_trade_sdk::TradeBuyParams;

let buy_params = TradeBuyParams {
    // ... 其他参数
    gas_fee_strategy: gas_fee_strategy.clone(),
};
```

### 7. 查看和清理

```rust
// 移除某个策略
gas_fee_strategy.del_all(SwqosType::Jito, TradeType::Buy);
// 查看所有策略
gas_fee_strategy.print_all_strategies();
// 清空所有策略
gas_fee_strategy.clear();
```

## 🔗 相关文档

- [示例：Gas Fee 策略](../examples/gas_fee_strategy/)

## 最小小费

启用 `with_tip` 和 `check_min_tip` 时，各 SWQOS 通道按客户端 `min_tip_sol()` 检查费用策略。低于阈值的策略会被跳过，SDK 不会自动提高配置的小费。当前 Node1 默认值在 `src/constants/swqos.rs` 中为 `0.0001 SOL`；配置前应核对服务商的当前要求。没有可用策略时返回错误。
