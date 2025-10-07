# 📊 Gas Fee Strategy Guide

This document introduces the Gas Fee strategy configuration and usage methods in Sol Trade SDK.

## Basic Usage

### 1. Overview

This module supports users to configure strategies for SwqosType under different TradeType (buy/sell).

- Normal strategy: One SwqosType sends one transaction, specifying cu_limit, cu_price, and tip.
- High-low fee strategy: One SwqosType sends two transactions simultaneously, one with low tip and high priority fee, another with high tip and low priority fee.

Each (SwqosType, TradeType) combination can only configure one strategy. Subsequent strategy configurations will override previous ones.

### 2. Create GasFeeStrategy Instance

```rust
use sol_trade_sdk::common::GasFeeStrategy;

// Create a new GasFeeStrategy instance
let gas_fee_strategy = GasFeeStrategy::new();
```

### 3. Set Global Strategy (can also be configured individually)

```rust
// Set global strategy (normal strategy)
gas_fee_strategy.set_global_fee_strategy(
    150000, // cu_limit
    500000, // cu_price
    0.001,  // buy tip
    0.001   // sell tip
);
```

### 4. Configuring Single Strategy

```rust
// Configure normal strategy for SwqosType::Jito
gas_fee_strategy.set_normal_fee_strategy(
    SwqosType::Jito,
    xxxxx, // cu_limit
    xxxx,  // cu_price
    xxxxx, // buy_tip
    xxxxx  // sell_tip
);
```

### 5. Configuring High-Low Fee Strategy

```rust
// Configure high-low fee strategy for SwqosType::Jito during Buy
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

### 6. Using in Trading Parameters

```rust
use sol_trade_sdk::TradeBuyParams;

let buy_params = TradeBuyParams {
    // ... other parameters
    gas_fee_strategy: gas_fee_strategy.clone(),
};
```

### 7. Viewing and Cleanup

```rust
// Remove a specific strategy
gas_fee_strategy.del_all(SwqosType::Jito, TradeType::Buy);
// View all strategies
gas_fee_strategy.print_all_strategies();
// Clear all strategies
gas_fee_strategy.clear();
```

## 🔗 Related Documents

- [Example: Gas Fee Strategy](../examples/gas_fee_strategy/)
