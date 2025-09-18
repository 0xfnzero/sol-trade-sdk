# Nonce 缓存指南

本指南介绍如何在 Sol Trade SDK 中使用 Nonce 缓存来实现交易重放保护和优化交易处理。

## 📋 什么是 Nonce 缓存？

Nonce 缓存是一个全局单例模式的缓存系统，用于管理 Solana 网络中的 durable nonce 账户。Durable nonce 是 Solana 的一项功能，允许您创建在较长时间内有效的交易，而不受最近区块哈希的 150 个区块限制。

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

### 1. 初始化 Nonce 缓存

首先需要设置 nonce 账户并初始化缓存：

```rust
use sol_trade_sdk::common::nonce_cache::NonceCache;

// 设置 nonce 账户
let nonce_account_str = "your_nonce_account_address_here";
NonceCache::get_instance().init(Some(nonce_account_str.to_string()));
```

### 2. 获取 Nonce 信息

从 RPC 获取最新的 nonce 信息：

```rust
// 获取并更新 nonce 信息
NonceCache::get_instance().fetch_nonce_info_use_rpc(&client.rpc).await?;

// 获取当前 nonce 值
let nonce_info = NonceCache::get_instance().get_nonce_info();
let current_nonce = nonce_info.current_nonce;
println!("Current nonce: {}", current_nonce);
```

### 3. 在交易中使用 Nonce

将 nonce 作为 recent_blockhash 参数传递给交易：

```rust
let buy_params = sol_trade_sdk::TradeBuyParams {
    dex_type: DexType::PumpFun,
    mint: mint_pubkey,
    sol_amount: buy_sol_amount,
    slippage_basis_points: Some(100),
    recent_blockhash: current_nonce, // 使用 nonce 作为 blockhash
    extension_params: Box::new(PumpFunParams::from_trade(&trade_info, None)),
    lookup_table_key: None,
    wait_transaction_confirmed: true,
    create_wsol_ata: false,
    close_wsol_ata: false,
    create_mint_ata: true,
    open_seed_optimize: false,
};

// 执行交易
client.buy(buy_params).await?;
```

## 🔄 Nonce 生命周期

1. **初始化**: 设置 nonce 账户地址
2. **获取**: 从 RPC 获取最新 nonce 值
4. **使用**: 在交易中作为 blockhash 使用
6. **刷新**: 下次使用前重新获取新的 nonce 值

## 🔗 相关文档

- [示例：Nonce 缓存](../examples/nonce_cache/)