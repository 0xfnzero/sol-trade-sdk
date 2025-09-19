<div align="center">
    <h1>🚀 Sol Trade SDK</h1>
    <h3><em>全面的 Rust SDK，用于无缝 Solana DEX 交易</em></h3>
</div>

<p align="center">
    <strong>将 PumpFun、PumpSwap、Bonk 和 Raydium 交易功能集成到您的应用程序中，提供强大的工具和统一的接口。</strong>
</p>

<p align="center">
    <a href="https://crates.io/crates/sol-trade-sdk">
        <img src="https://img.shields.io/crates/v/sol-trade-sdk.svg" alt="Crates.io">
    </a>
    <a href="https://docs.rs/sol-trade-sdk">
        <img src="https://docs.rs/sol-trade-sdk/badge.svg" alt="Documentation">
    </a>
    <a href="https://github.com/0xfnzero/sol-trade-sdk/blob/main/LICENSE">
        <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License">
    </a>
    <a href="https://github.com/0xfnzero/sol-trade-sdk">
        <img src="https://img.shields.io/github/stars/0xfnzero/sol-trade-sdk?style=social" alt="GitHub stars">
    </a>
    <a href="https://github.com/0xfnzero/sol-trade-sdk/network">
        <img src="https://img.shields.io/github/forks/0xfnzero/sol-trade-sdk?style=social" alt="GitHub forks">
    </a>
</p>

<p align="center">
    <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
    <img src="https://img.shields.io/badge/Solana-9945FF?style=for-the-badge&logo=solana&logoColor=white" alt="Solana">
    <img src="https://img.shields.io/badge/DEX-4B8BBE?style=for-the-badge&logo=bitcoin&logoColor=white" alt="DEX Trading">
</p>

<p align="center">
    <a href="https://github.com/0xfnzero/sol-trade-sdk/blob/main/README_CN.md">中文</a> |
    <a href="https://github.com/0xfnzero/sol-trade-sdk/blob/main/README.md">English</a> |
    <a href="https://fnzero.dev/">Website</a> |
    <a href="https://t.me/fnzero_group">Telegram</a> |
    <a href="https://discord.gg/vuazbGkqQE">Discord</a>
</p>

## 📋 目录

- [✨ 项目特性](#-项目特性)
- [📦 安装](#-安装)
- [🛠️ 使用示例](#️-使用示例)
  - [📋 使用示例](#-使用示例)
  - [⚡ 交易参数](#-交易参数)
  - [📊 使用示例汇总表格](#-使用示例汇总表格)
  - [⚙️ SWQOS 服务配置说明](#️-swqos-服务配置说明)
  - [🔧 中间件系统说明](#-中间件系统说明)
  - [🔍 地址查找表](#-地址查找表)
  - [🔍 Nonce 缓存](#-nonce-缓存)
- [🛡️ MEV 保护服务](#️-mev-保护服务)
- [📁 项目结构](#-项目结构)
- [📄 许可证](#-许可证)
- [💬 联系方式](#-联系方式)
- [⚠️ 重要注意事项](#️-重要注意事项)

---

## ✨ 项目特性

1. **PumpFun 交易**: 支持`购买`、`卖出`功能
2. **PumpSwap 交易**: 支持 PumpSwap 池的交易操作
3. **Bonk 交易**: 支持 Bonk 的交易操作
4. **Raydium CPMM 交易**: 支持 Raydium CPMM (Concentrated Pool Market Maker) 的交易操作
5. **Raydium AMM V4 交易**: 支持 Raydium AMM V4 (Automated Market Maker) 的交易操作
6. **事件订阅**: SDK 集成了 solana-streamer SDK，支持引用该 SDK 订阅 PumpFun、PumpSwap、Bonk、Raydium CPMM 和 Raydium AMM V4 程序的交易事件，该 SDK 的说明可以查阅：[solana-streamer SDK](https://github.com/0xfnzero/solana-streamer)。
7. **多种 MEV 保护**: 支持 Jito、Nextblock、ZeroSlot、Temporal、Bloxroute、FlashBlock、BlockRazor、Node1、Astralane 等服务
8. **并发交易**: 同时使用多个 MEV 服务发送交易，最快的成功，其他失败
9. **统一交易接口**: 使用统一的交易协议枚举进行交易操作
10. **中间件系统**: 支持自定义指令中间件，可在交易执行前对指令进行修改、添加或移除

## 📦 安装

### 直接克隆

将此项目克隆到您的项目目录：

```bash
cd your_project_root_directory
git clone https://github.com/0xfnzero/sol-trade-sdk
```

在您的`Cargo.toml`中添加依赖：

```toml
# 添加到您的 Cargo.toml
sol-trade-sdk = { path = "./sol-trade-sdk", version = "1.0.1" }
```

### 使用 crates.io

```toml
# 添加到您的 Cargo.toml
sol-trade-sdk = "1.0.1"
```

## 🛠️ 使用示例

### 📋 使用示例

#### 1. 创建 SolanaTrade 实例

可以参考 [示例：创建 SolanaTrade 实例](examples/trading_client/src/main.rs)。

```rust
// 钱包
let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
// RPC 地址
let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();
let commitment = CommitmentConfig::processed();
// 可以配置多个SWQOS服务
let swqos_configs: Vec<SwqosConfig> = vec![
    SwqosConfig::Default(rpc_url.clone()),
    SwqosConfig::Jito("your uuid".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::NextBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::Bloxroute("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::ZeroSlot("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::Temporal("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::FlashBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::Node1("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::BlockRazor("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::Astralane("your api_token".to_string(), SwqosRegion::Frankfurt, None),
];
// 创建 TradeConfig 实例
let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);
// 创建 SolanaTrade 客户端
let client = SolanaTrade::new(Arc::new(payer), trade_config).await;
```

#### 2. 配置 Gas Fee 策略

有关 Gas Fee 策略的详细信息，请参阅 [Gas Fee 策略参考手册](docs/GAS_FEE_STRATEGY_CN.md)。
```rust
// 设置全局策略
sol_trade_sdk::common::set_global_gas_fee_strategy(150000, 500000, 0.001, 0.001);
```

#### 3. 构建交易参数

有关所有交易参数的详细信息，请参阅 [交易参数参考手册](docs/TRADING_PARAMETERS_CN.md)。

```rust
let buy_params = sol_trade_sdk::TradeBuyParams {
  dex_type: DexType::PumpSwap,
  mint: mint_pubkey,
  sol_amount: buy_sol_amount,
  slippage_basis_points: slippage_basis_points,
  recent_blockhash: recent_blockhash,
  extension_params: Box::new(params.clone()),
  lookup_table_key: None,
  wait_transaction_confirmed: true,
  create_wsol_ata: true,
  close_wsol_ata: true,
  create_mint_ata: true,
  open_seed_optimize: false,
};
```

#### 4. 执行交易

```rust
client.buy(buy_params).await?;
```

### ⚡ 交易参数

有关所有交易参数（包括 `TradeBuyParams` 和 `TradeSellParams`）的详细信息，请参阅专门的 [交易参数参考手册](docs/TRADING_PARAMETERS_CN.md)。

#### 关于shredstream

当你使用 shred 订阅事件时，由于 shred 的特性，你无法获取到交易事件的完整信息。
请你在使用时，确保你的交易逻辑依赖的参数，在shred中都能获取到。

### 📊 使用示例汇总表格

| 描述 | 运行命令 | 源码路径 |
|------|---------|----------|
| 监听代币交易事件 | `cargo run --package event_subscription` | [examples/event_subscription](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/event_subscription/src/main.rs) |
| 创建和配置 SolanaTrade 实例 | `cargo run --package trading_client` | [examples/trading_client](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/trading_client/src/main.rs) |
| PumpFun 代币狙击交易 | `cargo run --package pumpfun_sniper_trading` | [examples/pumpfun_sniper_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpfun_sniper_trading/src/main.rs) |
| PumpFun 代币跟单交易 | `cargo run --package pumpfun_copy_trading` | [examples/pumpfun_copy_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpfun_copy_trading/src/main.rs) |
| PumpSwap 交易操作 | `cargo run --package pumpswap_trading` | [examples/pumpswap_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpswap_trading/src/main.rs) |
| Raydium CPMM 交易操作 | `cargo run --package raydium_cpmm_trading` | [examples/raydium_cpmm_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/raydium_cpmm_trading/src/main.rs) |
| Raydium AMM V4 交易操作 | `cargo run --package raydium_amm_v4_trading` | [examples/raydium_amm_v4_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/raydium_amm_v4_trading/src/main.rs) |
| Bonk 代币狙击交易 | `cargo run --package bonk_sniper_trading` | [examples/bonk_sniper_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/bonk_sniper_trading/src/main.rs) |
| Bonk 代币跟单交易 | `cargo run --package bonk_copy_trading` | [examples/bonk_copy_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/bonk_copy_trading/src/main.rs) |
| 自定义指令中间件示例 | `cargo run --package middleware_system` | [examples/middleware_system](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/middleware_system/src/main.rs) |
| 地址查找表示例 | `cargo run --package address_lookup` | [examples/address_lookup](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/address_lookup/src/main.rs) |
| Nonce示例 | `cargo run --package nonce_cache` | [examples/nonce_cache](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/nonce_cache/src/main.rs) |
| SOL与WSOL相互转换示例 | `cargo run --package wsol_wrapper` | [examples/wsol_wrapper](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/wsol_wrapper/src/main.rs) |
| Seed 优化交易示例 | `cargo run --package seed_trading` | [examples/seed_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/seed_trading/src/main.rs) |
| Gas费用策略示例 | `cargo run --package gas_fee_strategy` | [examples/gas_fee_strategy](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/gas_fee_strategy/src/main.rs) |

### ⚙️ SWQOS 服务配置说明

在配置 SWQOS 服务时，需要注意不同服务的参数要求：

- **Jito**: 第一个参数为 UUID（如无 UUID 请传入空字符串 `""`）
- 其他的MEV服务，第一个参数为 API Token

#### 自定义 URL 支持

每个 SWQOS 服务现在都支持可选的自定义 URL 参数：

```rust
// 使用自定义 URL（第三个参数）
let jito_config = SwqosConfig::Jito(
    "your_uuid".to_string(),
    SwqosRegion::Frankfurt, // 这个参数仍然需要，但会被忽略
    Some("https://custom-jito-endpoint.com".to_string()) // 自定义 URL
);

// 使用默认区域端点（第三个参数为 None）
let nextblock_config = SwqosConfig::NextBlock(
    "your_api_token".to_string(),
    SwqosRegion::NewYork, // 将使用该区域的默认端点
    None // 没有自定义 URL，使用 SwqosRegion
);
```

**URL 优先级逻辑**：
- 如果提供了自定义 URL（`Some(url)`），将使用自定义 URL 而不是区域端点
- 如果没有提供自定义 URL（`None`），系统将使用指定 `SwqosRegion` 的默认端点
- 这提供了最大的灵活性，同时保持向后兼容性

当使用多个MEV服务时，需要使用`Durable Nonce`。你需要初始化`NonceCache`类（或者自行写一个管理nonce的类），获取最新的`nonce`值，并在交易的时候将`nonce_account`和`current_nonce`填入交易参数。

---

### 🔧 中间件系统说明

SDK 提供了强大的中间件系统，允许您在交易执行前对指令进行修改、添加或移除。中间件按照添加顺序依次执行：

```rust
let middleware_manager = MiddlewareManager::new()
    .add_middleware(Box::new(FirstMiddleware))   // 第一个执行
    .add_middleware(Box::new(SecondMiddleware))  // 第二个执行
    .add_middleware(Box::new(ThirdMiddleware));  // 最后执行
```

### 🔍 地址查找表

地址查找表 (ALT) 允许您通过将经常使用的地址存储在紧凑的表格格式中来优化交易大小并降低费用。详细信息请参阅 [地址查找表指南](docs/ADDRESS_LOOKUP_TABLE_CN.md)。

### 🔍 Nonce 缓存

使用 Nonce 缓存来实现交易重放保护和优化交易处理。详细信息请参阅 [Nonce 缓存指南](docs/NONCE_CACHE_CN.md)。

## 🛡️ MEV 保护服务

可以通过官网申请密钥：[社区官网](https://fnzero.dev/swqos)

- **Jito**: 高性能区块空间
- **NextBlock**: 快速交易执行
- **ZeroSlot**: 零延迟交易
- **Temporal**: 时间敏感交易
- **Bloxroute**: 区块链网络加速
- **FlashBlock**: 高速交易执行，支持 API 密钥认证 - [官方文档](https://doc.flashblock.trade/)
- **BlockRazor**: 高速交易执行，支持 API 密钥认证 - [官方文档](https://blockrazor.gitbook.io/blockrazor/)
- **Node1**: 高速交易执行，支持 API 密钥认证 - [官方文档](https://node1.me/docs.html)
- **Astralane**: 高速交易执行，支持 API 密钥认证

## 📁 项目结构

```
src/
├── common/           # 通用功能和工具
├── constants/        # 常量定义
├── instruction/      # 指令构建
│   └── utils/        # 指令工具函数
├── protos/           # gRPC 协议定义
├── swqos/            # MEV 服务客户端
├── trading/          # 统一交易引擎
│   ├── common/       # 通用交易工具
│   ├── core/         # 核心交易引擎
│   ├── middleware/   # 中间件系统
│   └── factory.rs    # 交易工厂
├── utils/            # 工具函数
│   ├── calc/         # 数量计算工具
│   └── price/        # 价格计算工具
└── lib.rs            # 主库文件
```

## 📄 许可证

MIT 许可证

## 💬 联系方式

- 官方网站: https://fnzero.dev/
- 项目仓库: https://github.com/0xfnzero/sol-trade-sdk
- Telegram 群组: https://t.me/fnzero_group
- Discord: https://discord.gg/vuazbGkqQE

## ⚠️ 重要注意事项

1. 在主网使用前请充分测试
2. 正确设置私钥和 API 令牌
3. 注意滑点设置避免交易失败
4. 监控余额和交易费用
5. 遵循相关法律法规

