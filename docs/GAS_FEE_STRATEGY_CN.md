# 📊 Gas Fee 策略指南

本文档介绍 Sol Trade SDK 中的 Gas Fee 策略配置和使用方法。

## 基础使用

### 1. 说明

该模块支持用户配置 SwqosType 在不同 TradeType(buy/sell) 下的策略。

- normal 策略: 一个 SwqosType 发送一笔交易，指定 cu_limit、cu_price 和小费。
- 高低费率策略: 一个 SwqosType 同时发送两笔交易，一笔低小费高优先费，一笔高小费低优先费。

每个 (SwqosType, TradeType) 的组合仅可配置一个策略。后续配置的策略会覆盖之前的策略。

### 2. 使用内置策略

```rust
use sol_trade_sdk::common::gas_fee_strategy::GasFeeStrategy;
// 使用内置策略（内置了各个 SwqosType 的 normal 策略）
GasFeeStrategy::init_builtin_fee_strategies();
```

### 3. 配置单个策略

```rust
// 为 SwqosType::Jito 在 Buy 时配置 normal 策略
GasFeeStrategy::add_normal_fee_strategy(
    SwqosType::Jito,
    TradeType::Buy,
    xxxxx, // cu_limit
    xxxx,  // cu_price
    xxxxx  // tip
);
```

### 4. 配置高低费率策略

```rust
// 为 SwqosType::Jito 在 Buy 时配置高低费率策略
GasFeeStrategy::add_high_low_fee_strategy(
    SwqosType::Jito,
    TradeType::Buy,
    xxxxx, // cu_limit
    xxxxx, // low cu_price
    xxxxx, // high cu_price
    xxxxx, // low tip
    xxxxx  // high tip
);
```

### 5. 查看和清理

```rust
// 移除某个策略
GasFeeStrategy::remove_strategy(SwqosType::Jito, TradeType::Buy);
// 查看所有策略
GasFeeStrategy::print_all_strategies();
// 清空所有策略
GasFeeStrategy::clear();
```

## 🔗 相关文档

- [示例：Gas Fee 策略](../examples/gas_fee_strategy/)