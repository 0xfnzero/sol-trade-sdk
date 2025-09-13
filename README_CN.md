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
    <a href="https://t.me/fnzero_group">Telegram</a>
</p>

## 📋 目录

- [✨ 项目特性](#-项目特性)
- [📦 安装](#-安装)
- [🛠️ 使用示例](#️-使用示例)
  - [📋 重要说明](#-重要说明)
  - [📊 使用示例汇总表格](#-使用示例汇总表格)
  - [⚙️ SWQOS 服务配置说明](#️-swqos-服务配置说明)
  - [🔧 中间件系统说明](#-中间件系统说明)
  - [⚡ 自定义优先费用配置](#-自定义优先费用配置)
- [🏪 支持的交易平台](#-支持的交易平台)
- [🛡️ MEV 保护服务](#️-mev-保护服务)
- [💰 价格计算工具](#-价格计算工具)
- [🧮 数量计算工具](#-数量计算工具)
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
6. **事件订阅**: 订阅 PumpFun、PumpSwap、Bonk、Raydium CPMM 和 Raydium AMM V4 程序的交易事件
7. **Yellowstone gRPC**: 使用 Yellowstone gRPC 订阅程序事件
8. **ShredStream 支持**: 使用 ShredStream 订阅程序事件
9. **多种 MEV 保护**: 支持 Jito、Nextblock、ZeroSlot、Temporal、Bloxroute、FlashBlock、BlockRazor、Node1、Astralane 等服务
10. **并发交易**: 同时使用多个 MEV 服务发送交易，最快的成功，其他失败
11. **统一交易接口**: 使用统一的交易协议枚举进行交易操作
12. **中间件系统**: 支持自定义指令中间件，可在交易执行前对指令进行修改、添加或移除

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
sol-trade-sdk = { path = "./sol-trade-sdk", version = "0.6.10" }
```

### 使用 crates.io

```toml
# 添加到您的 Cargo.toml
sol-trade-sdk = "0.6.10"
```

## 🛠️ 使用示例

### 📋 重要说明

#### 🌱 open_seed_optimize 参数

`open_seed_optimize` ，用于指定是否使用 seed 优化交易 CU 消耗。

- **用途**：当 `open_seed_optimize: true` 时，SDK 会在交易时使用 createAccountWithSeed 优化来创建代币 ata 账户。
- **注意**：开启 `open_seed_optimize` 后创建的交易，需要通过该 SDK 卖出，使用官网提供的方法卖出可能会失败。
- **注意**：开启 `open_seed_optimize` 后，获取代币 ata 地址需要通过 `get_associated_token_address_with_program_id_fast_use_seed` 方法获取。

#### 💰 create_wsol_ata 和 close_wsol_ata、 create_mint_ata 参数

在 PumpSwap、Bonk、Raydium 交易中，`create_wsol_ata` 和 `close_wsol_ata`、 `create_mint_ata` 参数提供对 wSOL（Wrapped SOL）账户管理的精细控制：

- **create_wsol_ata**：
  - 当 `create_wsol_ata: true` 时，SDK 会在交易前自动创建并将 SOL 包装为 wSOL
  - 买入时：自动将 SOL 包装为 wSOL 进行交易

- **close_wsol_ata**：
  - 当 `close_wsol_ata: true` 时，SDK 会在交易后自动关闭 wSOL 账户并解包装为 SOL
  - 卖出时：自动将获得的 wSOL 解包装为 SOL 并回收租金

- **create_mint_ata**：
  - 当 `create_mint_ata: true` 时，SDK 会在交易时创建代币ata账户

- **分离参数的优势**：
  - 允许独立控制 wSOL 账户的创建和关闭
  - 适用于批量操作，可以创建一次，在多次交易后再关闭
  - 为高级交易策略提供灵活性

#### 🔍 lookup_table_key 参数

`lookup_table_key` 参数是一个可选的 `Pubkey`，用于指定地址查找表以优化交易。在使用前你需要通过`AddressLookupTableCache`来管理缓存地址查找表。

- **用途**：地址查找表可以通过存储常用地址来减少交易大小并提高执行速度
- **使用方法**：
  - 可以在 `buy()` 和 `sell()` 方法中按交易覆盖
  - 如果不提供，默认为 `None`
- **优势**：
  - 通过从查找表引用地址来减少交易大小
  - 提高交易成功率和速度
  - 特别适用于具有许多账户引用的复杂交易

#### ⚡ priority_fee 参数

`priority_fee` 参数是一个可选的 `PriorityFee`，允许您为单个交易覆盖默认的优先级费用设置：

- **用途**：为每个交易提供对交易优先级费用的细粒度控制
- **使用方法**：
  - 可以传递给 `buy()` 和 `sell()` 方法来覆盖全局优先级费用设置
  - 如果不提供，默认为 `None` 并使用 `TradeConfig` 中的优先级费用设置
  - 当提供时，`buy_tip_fees` 数组将自动填充以匹配 SWQOS 客户端的数量
- **优势**：
  - 允许根据市场条件动态调整优先级费用
  - 为不同类型的交易启用不同的费用策略
  - 为高频交易场景提供灵活性

#### 关于shredstream

当你使用 shred 订阅事件时，由于 shred 的特性，你无法获取到交易事件的完整信息。
请你在使用时，确保你的交易逻辑依赖的参数，在shred中都能获取到。

### 📊 使用示例汇总表格

| 功能类型 | 示例包名 | 描述 | 运行命令 | 源码路径 |
|---------|---------|------|---------|----------|
| 事件订阅 | `event_subscription` | 监听代币交易事件 | `cargo run --package event_subscription` | [examples/event_subscription](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/event_subscription/src/main.rs) |
| 交易客户端 | `trading_client` | 创建和配置 SolanaTrade 实例 | `cargo run --package trading_client` | [examples/trading_client](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/trading_client/src/main.rs) |
| PumpFun 狙击 | `pumpfun_sniper_trading` | PumpFun 代币狙击交易 | `cargo run --package pumpfun_sniper_trading` | [examples/pumpfun_sniper_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpfun_sniper_trading/src/main.rs) |
| PumpFun 跟单 | `pumpfun_copy_trading` | PumpFun 代币跟单交易 | `cargo run --package pumpfun_copy_trading` | [examples/pumpfun_copy_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpfun_copy_trading/src/main.rs) |
| PumpSwap | `pumpswap_trading` | PumpSwap 交易操作 | `cargo run --package pumpswap_trading` | [examples/pumpswap_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpswap_trading/src/main.rs) |
| Raydium CPMM | `raydium_cpmm_trading` | Raydium CPMM 交易操作 | `cargo run --package raydium_cpmm_trading` | [examples/raydium_cpmm_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/raydium_cpmm_trading/src/main.rs) |
| Raydium AMM V4 | `raydium_amm_v4_trading` | Raydium AMM V4 交易操作 | `cargo run --package raydium_amm_v4_trading` | [examples/raydium_amm_v4_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/raydium_amm_v4_trading/src/main.rs) |
| Bonk 狙击 | `bonk_sniper_trading` | Bonk 代币狙击交易 | `cargo run --package bonk_sniper_trading` | [examples/bonk_sniper_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/bonk_sniper_trading/src/main.rs) |
| Bonk 跟单 | `bonk_copy_trading` | Bonk 代币跟单交易 | `cargo run --package bonk_copy_trading` | [examples/bonk_copy_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/bonk_copy_trading/src/main.rs) |
| 中间件系统 | `middleware_system` | 自定义指令中间件示例 | `cargo run --package middleware_system` | [examples/middleware_system](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/middleware_system/src/main.rs) |
| 地址查找表 | `address_lookup` | 地址查找表示例 | `cargo run --package address_lookup` | [examples/address_lookup](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/address_lookup/src/main.rs) |
| Nonce    | `nonce_cache` | Nonce示例 | `cargo run --package nonce_cache` | [examples/nonce_cache](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/nonce_cache/src/main.rs) |
| WSOL 包装器 | `wsol_wrapper` | SOL与WSOL相互转换示例 | `cargo run --package wsol_wrapper` | [examples/wsol_wrapper](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/wsol_wrapper/src/main.rs) |
| Seed 优化 | `seed_trading` | Seed 优化交易示例 | `cargo run --package seed_trading` | [examples/seed_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/seed_trading/src/main.rs) |

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

当使用多个MEV服务时，需要使用`Durable Nonce`。你需要初始化`NonceCache`类（或者自行写一个管理nonce的类），获取最新的`nonce`值，并在交易的时候作为`blockhash`使用。

---

### 🔧 中间件系统说明

SDK 提供了强大的中间件系统，允许您在交易执行前对指令进行修改、添加或移除。中间件按照添加顺序依次执行：

```rust
let middleware_manager = MiddlewareManager::new()
    .add_middleware(Box::new(FirstMiddleware))   // 第一个执行
    .add_middleware(Box::new(SecondMiddleware))  // 第二个执行
    .add_middleware(Box::new(ThirdMiddleware));  // 最后执行
```

### ⚡ 自定义优先费用配置

```rust
use sol_trade_sdk::common::PriorityFee;

// 自定义优先费用配置
let priority_fee = PriorityFee {
    tip_unit_limit: 190000,
    tip_unit_price: 1000000,
    rpc_unit_limit: 500000,
    rpc_unit_price: 500000,
    buy_tip_fee: 0.001,
    buy_tip_fees: vec![0.001, 0.002],
    sell_tip_fee: 0.0001,
};

// 在TradeConfig中使用自定义优先费用
let trade_config = TradeConfig {
    rpc_url: rpc_url.clone(),
    commitment: CommitmentConfig::confirmed(),
    priority_fee, // 使用自定义优先费用
    swqos_configs,
};
```

## 🏪 支持的交易平台

- **PumpFun**: 主要的 meme 币交易平台
- **PumpSwap**: PumpFun 的交换协议
- **Bonk**: 代币发行平台（letsbonk.fun）
- **Raydium CPMM**: Raydium 的集中流动性做市商协议
- **Raydium AMM V4**: Raydium 的自动做市商 V4 协议

## 🛡️ MEV 保护服务

- **Jito**: 高性能区块空间
- **NextBlock**: 快速交易执行
- **ZeroSlot**: 零延迟交易
- **Temporal**: 时间敏感交易
- **Bloxroute**: 区块链网络加速
- **FlashBlock**: 高速交易执行，支持 API 密钥认证 - [官方文档](https://doc.flashblock.trade/)
- **BlockRazor**: 高速交易执行，支持 API 密钥认证 - [官方文档](https://blockrazor.gitbook.io/blockrazor/)
- **Node1**: 高速交易执行，支持 API 密钥认证 - [官方文档](https://node1.me/docs.html)
- **Astralane**: 高速交易执行，支持 API 密钥认证


## 💰 价格计算工具

SDK 包含所有支持协议的价格计算工具，位于 `src/utils/price/` 目录。

## 🧮 数量计算工具

SDK 提供各种协议的交易数量计算功能，位于 `src/utils/calc/` 目录：

- **通用计算函数**: 提供通用的手续费计算和除法运算工具
- **协议特定计算**: 针对每个协议的特定计算逻辑
  - **PumpFun**: 基于联合曲线的代币购买/销售数量计算
  - **PumpSwap**: 支持多种交易对的数量计算
  - **Raydium AMM V4**: 自动做市商池的数量和手续费计算
  - **Raydium CPMM**: 恒定乘积做市商的数量计算
  - **Bonk**: 专门的 Bonk 代币计算逻辑

主要功能包括：
- 根据输入金额计算输出数量
- 手续费计算和分配
- 滑点保护计算
- 流动性池状态计算

## 📁 项目结构

```
src/
├── common/           # 通用功能和工具
├── constants/        # 常量定义
├── instruction/      # 指令构建
├── swqos/            # MEV服务客户端
├── trading/          # 统一交易引擎
│   ├── common/       # 通用交易工具
│   ├── core/         # 核心交易引擎
│   ├── middleware/   # 中间件系统
│   │   ├── builtin.rs    # 内置中间件实现
│   │   ├── traits.rs     # 中间件 trait 定义
│   │   └── mod.rs        # 中间件模块
│   ├── bonk/         # Bonk交易实现
│   ├── pumpfun/      # PumpFun交易实现
│   ├── pumpswap/     # PumpSwap交易实现
│   ├── raydium_cpmm/ # Raydium CPMM交易实现
│   ├── raydium_amm_v4/ # Raydium AMM V4交易实现
│   └── factory.rs    # 交易工厂
├── utils/            # 工具函数
│   ├── price/        # 价格计算工具
│   │   ├── common.rs       # 通用价格函数
│   │   ├── bonk.rs         # Bonk 价格计算
│   │   ├── pumpfun.rs      # PumpFun 价格计算
│   │   ├── pumpswap.rs     # PumpSwap 价格计算
│   │   ├── raydium_cpmm.rs # Raydium CPMM 价格计算
│   │   ├── raydium_clmm.rs # Raydium CLMM 价格计算
│   │   └── raydium_amm_v4.rs # Raydium AMM V4 价格计算
│   └── calc/         # 数量计算工具
│       ├── common.rs       # 通用计算函数
│       ├── bonk.rs         # Bonk 数量计算
│       ├── pumpfun.rs      # PumpFun 数量计算
│       ├── pumpswap.rs     # PumpSwap 数量计算
│       ├── raydium_cpmm.rs # Raydium CPMM 数量计算
│       └── raydium_amm_v4.rs # Raydium AMM V4 数量计算
├── lib.rs            # 主库文件
└── main.rs           # 示例程序
```

## 📄 许可证

MIT 许可证

## 💬 联系方式

- 官方网站: https://fnzero.dev/
- 项目仓库: https://github.com/0xfnzero/sol-trade-sdk
- Telegram 群组: https://t.me/fnzero_group

## ⚠️ 重要注意事项

1. 在主网使用前请充分测试
2. 正确设置私钥和 API 令牌
3. 注意滑点设置避免交易失败
4. 监控余额和交易费用
5. 遵循相关法律法规

