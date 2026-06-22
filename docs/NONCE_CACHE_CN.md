# Nonce 使用指南

本指南介绍如何在 Sol Trade SDK 中使用 Durable Nonce 来实现交易重放保护和优化交易处理。

## 📋 什么是 Durable Nonce？

Durable Nonce 是 Solana 的一项功能，允许您创建在较长时间内有效的交易，而不受最近区块哈希的 150 个区块限制。

## 🚀 核心优势

- **交易重放保护**: 防止相同交易被重复执行
- **时间窗口扩展**: 交易可在更长时间内保持有效
- **网络性能优化**: 减少对最新区块哈希的依赖
- **交易确定性**: 提供一致的交易处理体验
- **离线交易支持**: 支持预签名交易的离线处理

## 🛠️ 实现方法

### 前提：

需要先创建你 payer 账号使用的 nonce 账户。
参考资料： https://solana.com/zh/developers/guides/advanced/introduction-to-durable-nonces

### 1. 获取 Nonce 信息

从 RPC 直接获取 nonce 信息：

```rust
use sol_trade_sdk::common::nonce_cache::fetch_nonce_info;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

// 设置 nonce 账户
let nonce_account = Pubkey::from_str("your_nonce_account_address_here")?;

// 获取 nonce 信息
let durable_nonce = fetch_nonce_info(&client.rpc, nonce_account).await;
```

### 2. 在交易中使用 Nonce

设置 nonce 参数：durable_nonce

```rust
let buy_params = sol_trade_sdk::TradeBuyParams {
    dex_type: DexType::PumpFun,
    mint: mint_pubkey,
    sol_amount: buy_sol_amount,
    slippage_basis_points: Some(100),
    recent_blockhash: Some(recent_blockhash),
    extension_params: Box::new(PumpFunParams::from_trade(&trade_info, None)),
    wait_transaction_confirmed: true,
    create_wsol_ata: false,
    close_wsol_ata: false,
    create_mint_ata: true,
    open_seed_optimize: false,
    durable_nonce: durable_nonce, // 设置 durable nonce
};

// 执行交易
client.buy(buy_params).await?;
```

## 🔄 Nonce 使用流程

1. **获取**: 从 RPC 获取最新 nonce 值
2. **使用**: 在交易中设置 nonce 参数
3. **刷新**: 下次使用前重新调用 `fetch_nonce_info` 获取新的 nonce 值

## 🔗 相关文档

- [示例：Durable Nonce](../examples/nonce_cache/)
