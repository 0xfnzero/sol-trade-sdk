# 地址查找表指南

本指南介绍如何在 Sol Trade SDK 中使用地址查找表 (ALT) 来优化交易大小并降低费用。

## 📋 什么是地址查找表？

地址查找表是 Solana 的一项功能，允许您将经常使用的地址以紧凑的表格格式存储。您可以通过查找表中的索引来引用地址，而不是在交易中包含完整的 32 字节地址，从而显著减少交易大小和成本。

## 🚀 核心优势

- **交易大小优化**: 使用地址索引而非完整地址来减少交易大小
- **成本降低**: 由于交易大小减小而降低交易费用
- **性能提升**: 更快的交易处理和验证速度
- **网络效率**: 减少带宽使用和区块空间消耗

## 🛠️ 实现方法

### 1. 设置地址查找表缓存

SDK 提供了一个全局缓存来管理地址查找表：

```rust
use sol_trade_sdk::common::address_lookup_cache::AddressLookupTableCache;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// 设置查找表缓存
async fn setup_lookup_table_cache(
    client: Arc<SolanaRpcClient>,
    lookup_table_address: Pubkey,
) -> AnyResult<()> {
    AddressLookupTableCache::get_instance()
        .set_address_lookup_table(client, &lookup_table_address)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set address lookup table: {}", e))?;
    Ok(())
}
```

### 2. 在交易参数中使用查找表

在您的交易参数中包含查找表：

```rust
// 初始化查找表
let lookup_table_key = Pubkey::from_str("your_lookup_table_address_here").unwrap();
setup_lookup_table_cache(client.rpc.clone(), lookup_table_key).await?;

// 在交易参数中包含查找表
let buy_params = sol_trade_sdk::TradeBuyParams {
    dex_type: DexType::PumpFun,
    mint: mint_pubkey,
    sol_amount: buy_sol_amount,
    slippage_basis_points: Some(100),
    recent_blockhash: Some(recent_blockhash),
    extension_params: Box::new(PumpFunParams::from_trade(&trade_info, None)),
    lookup_table_key: Some(lookup_table_key), // 包含查找表
    wait_transaction_confirmed: true,
    create_wsol_ata: false,
    close_wsol_ata: false,
    create_mint_ata: true,
    open_seed_optimize: false,
};

// 执行交易
client.buy(buy_params).await?;
```

## 📊 性能对比

| 方面 | 不使用 ALT | 使用 ALT | 改进幅度 |
|------|-----------|----------|----------|
| **交易大小** | ~1,232 字节 | ~800 字节 | 减少 35% |
| **地址存储** | 每个地址 32 字节 | 每个地址 1 字节 | 减少 97% |
| **交易费用** | 更高 | 更低 | 节省高达 30% |
| **区块空间使用** | 更多 | 更少 | 提高网络效率 |

## ⚠️ 重要注意事项

1. **查找表地址**: 必须提供有效的地址查找表地址
2. **缓存管理**: SDK 自动管理查找表缓存
3. **RPC 兼容性**: 确保您的 RPC 提供商支持查找表
4. **网络**: 查找表是特定于网络的（主网/开发网/测试网）
5. **测试**: 在主网使用前请务必在开发网测试

## 🔗 相关文档

- [交易参数参考手册](TRADING_PARAMETERS_CN.md)
- [示例：地址查找表](../examples/address_lookup/)

## 📚 外部资源

- [Solana 地址查找表文档](https://docs.solana.com/developing/lookup-tables)