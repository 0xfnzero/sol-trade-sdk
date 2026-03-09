<div align="center">
    <h1>🚀 Sol Trade SDK</h1>
    <h3><em>全面的 Rust SDK，用于无缝 Solana DEX 交易</em></h3>
</div>

<p align="center">
    <strong>一个面向低延迟 Solana DEX 交易机器人的高性能 Rust SDK。该 SDK 以速度和效率为核心设计，支持与 PumpFun、Pump AMM（PumpSwap）、Bonk、Meteora DAMM v2、Raydium AMM v4 以及 Raydium CPMM 进行无缝、高吞吐量的交互，适用于对延迟高度敏感的交易策略。</strong>
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
  - [⚙️ SWQoS 服务配置说明](#️-swqos-服务配置说明)
  - [Astralane QUIC（低延迟）](#astralane-quic低延迟)
  - [🔧 中间件系统说明](#-中间件系统说明)
  - [🔍 地址查找表](#-地址查找表)
  - [🔍 Nonce 缓存](#-nonce-缓存)
- [💰 Cashback 支持（PumpFun / PumpSwap）](#-cashback-支持pumpfun--pumpswap)
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
6. **Meteora DAMM V2 交易**: 支持 Meteora DAMM V2 (Dynamic AMM) 的交易操作
7. **多种 MEV 保护**: 支持 Jito、Nextblock、ZeroSlot、Temporal、Bloxroute、FlashBlock、BlockRazor、Node1、Astralane、LunarLander 等服务
8. **并发交易**: 同时使用多个 MEV 服务发送交易，最快的成功，其他失败
9. **统一交易接口**: 使用统一的交易协议枚举进行交易操作
10. **中间件系统**: 支持自定义指令中间件，可在交易执行前对指令进行修改、添加或移除
11. **共享基础设施**: 多钱包可共享同一套 RPC 与 SWQoS 客户端，降低资源占用

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
sol-trade-sdk = { path = "./sol-trade-sdk", version = "3.5.7" }
```

### 使用 crates.io

```toml
# 添加到您的 Cargo.toml
sol-trade-sdk = "3.5.7"
```

## 🛠️ 使用示例

### 📋 使用示例

#### 1. 创建 TradingClient 实例

可参考 [示例：创建 TradingClient 实例](examples/trading_client/src/main.rs)。

**方式一：简单创建（单钱包）**
```rust
// 钱包
let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
// RPC 地址
let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();
let commitment = CommitmentConfig::processed();
// 可配置多个 SWQoS 服务
let swqos_configs: Vec<SwqosConfig> = vec![
    SwqosConfig::Default(rpc_url.clone()),
    SwqosConfig::Jito("your uuid".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::Bloxroute("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    // Astralane：第4个参数 None 为 HTTP，Some(SwqosTransport::Quic) 为 QUIC；同一 API key
    SwqosConfig::Astralane("your_astralane_api_key".to_string(), SwqosRegion::Frankfurt, None, None), // HTTP
    SwqosConfig::LunarLander("your_hellomoon_api_key".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::Astralane(
        "your_astralane_api_key".to_string(),
        SwqosRegion::Frankfurt,
        None,
        Some(SwqosTransport::Quic),
    ), // QUIC
];
// 创建 TradeConfig 实例
let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);

// 可选：自定义 WSOL ATA 与 Seed 优化
// let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment)
//     .with_wsol_ata_config(true, true);  // create_wsol_ata_on_startup, use_seed_optimize

// 创建 TradingClient
let client = TradingClient::new(Arc::new(payer), trade_config).await;
```

**方式二：共享基础设施（多钱包）**

多钱包场景下可先创建一份基础设施，再复用到多个钱包。参见 [示例：共享基础设施](examples/shared_infrastructure/src/main.rs)。

```rust
// 创建一次基础设施（开销较大）
let infra_config = InfrastructureConfig::new(rpc_url, swqos_configs, commitment);
let infrastructure = Arc::new(TradingInfrastructure::new(infra_config).await);

// 基于同一基础设施创建多个客户端（开销小）
let client1 = TradingClient::from_infrastructure(Arc::new(payer1), infrastructure.clone(), true);
let client2 = TradingClient::from_infrastructure(Arc::new(payer2), infrastructure.clone(), true);
```

#### 2. 配置 Gas Fee 策略

有关 Gas Fee 策略的详细信息，请参阅 [Gas Fee 策略参考手册](docs/GAS_FEE_STRATEGY_CN.md)。

```rust
// 创建 GasFeeStrategy 实例
let gas_fee_strategy = GasFeeStrategy::new();
// 设置全局策略
gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);
```

#### 3. 构建交易参数

有关所有交易参数的详细信息，请参阅 [交易参数参考手册](docs/TRADING_PARAMETERS_CN.md)。

```rust
// 导入 DexParamEnum 用于协议特定参数
use sol_trade_sdk::trading::core::params::DexParamEnum;

let buy_params = sol_trade_sdk::TradeBuyParams {
  dex_type: DexType::PumpSwap,
  input_token_type: TradeTokenType::WSOL,
  mint: mint_pubkey,
  input_token_amount: buy_sol_amount,
  slippage_basis_points: slippage_basis_points,
  recent_blockhash: Some(recent_blockhash),
  // 使用 DexParamEnum 实现类型安全的协议参数（零开销抽象）
  extension_params: DexParamEnum::PumpSwap(params.clone()),
  address_lookup_table_account: None,
  wait_transaction_confirmed: true,
  create_input_token_ata: true,
  close_input_token_ata: true,
  create_mint_ata: true,
  durable_nonce: None,
  fixed_output_token_amount: None,  // 可选：指定精确输出数量
  gas_fee_strategy: gas_fee_strategy.clone(),  // Gas 费用策略配置
  simulate: false,  // 设为 true 仅进行模拟
  use_exact_sol_amount: None,  // 对 PumpFun/PumpSwap 使用精确 SOL 输入（默认为 true）
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
| 创建和配置 TradingClient 实例 | `cargo run --package trading_client` | [examples/trading_client](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/trading_client/src/main.rs) |
| 多钱包共享基础设施 | `cargo run --package shared_infrastructure` | [examples/shared_infrastructure](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/shared_infrastructure/src/main.rs) |
| PumpFun 代币狙击交易 | `cargo run --package pumpfun_sniper_trading` | [examples/pumpfun_sniper_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpfun_sniper_trading/src/main.rs) |
| PumpFun 代币跟单交易 | `cargo run --package pumpfun_copy_trading` | [examples/pumpfun_copy_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpfun_copy_trading/src/main.rs) |
| PumpSwap 交易操作 | `cargo run --package pumpswap_trading` | [examples/pumpswap_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpswap_trading/src/main.rs) |
| Raydium CPMM 交易操作 | `cargo run --package raydium_cpmm_trading` | [examples/raydium_cpmm_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/raydium_cpmm_trading/src/main.rs) |
| Raydium AMM V4 交易操作 | `cargo run --package raydium_amm_v4_trading` | [examples/raydium_amm_v4_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/raydium_amm_v4_trading/src/main.rs) |
| Meteora DAMM V2 交易操作 | `cargo run --package meteora_damm_v2_direct_trading` | [examples/meteora_damm_v2_direct_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/meteora_damm_v2_direct_trading/src/main.rs) |
| Bonk 代币狙击交易 | `cargo run --package bonk_sniper_trading` | [examples/bonk_sniper_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/bonk_sniper_trading/src/main.rs) |
| Bonk 代币跟单交易 | `cargo run --package bonk_copy_trading` | [examples/bonk_copy_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/bonk_copy_trading/src/main.rs) |
| 自定义指令中间件示例 | `cargo run --package middleware_system` | [examples/middleware_system](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/middleware_system/src/main.rs) |
| 地址查找表示例 | `cargo run --package address_lookup` | [examples/address_lookup](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/address_lookup/src/main.rs) |
| Nonce示例 | `cargo run --package nonce_cache` | [examples/nonce_cache](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/nonce_cache/src/main.rs) |
| SOL与WSOL相互转换示例 | `cargo run --package wsol_wrapper` | [examples/wsol_wrapper](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/wsol_wrapper/src/main.rs) |
| Seed 优化交易示例 | `cargo run --package seed_trading` | [examples/seed_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/seed_trading/src/main.rs) |
| Gas费用策略示例 | `cargo run --package gas_fee_strategy` | [examples/gas_fee_strategy](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/gas_fee_strategy/src/main.rs) |

### ⚙️ SWQoS 服务配置说明

在配置 SWQoS 服务时，需要注意不同服务的参数要求：

- **Jito**: 第一个参数为 UUID（如无 UUID 请传入空字符串 `""`）
- 其他的MEV服务，第一个参数为 API Token

#### 自定义 URL 支持

每个 SWQoS 服务现在都支持可选的自定义 URL 参数：

```rust
// 使用自定义 URL（第三个参数）
let jito_config = SwqosConfig::Jito(
    "your_uuid".to_string(),
    SwqosRegion::Frankfurt, // 这个参数仍然需要，但会被忽略
    Some("https://custom-jito-endpoint.com".to_string()) // 自定义 URL
);

// 使用默认区域端点（第三个参数为 None）
let bloxroute_config = SwqosConfig::Bloxroute(
    "your_api_token".to_string(),
    SwqosRegion::NewYork, // 将使用该区域的默认端点
    None // 没有自定义 URL，使用 SwqosRegion
);
```

**URL 优先级逻辑**：
- 如果提供了自定义 URL（`Some(url)`），将使用自定义 URL 而不是区域端点
- 如果没有提供自定义 URL（`None`），系统将使用指定 `SwqosRegion` 的默认端点
- 这提供了最大的灵活性，同时保持向后兼容性

当使用多个MEV服务时，需要使用`Durable Nonce`。你需要使用`fetch_nonce_info`函数获取最新的`nonce`值，并在交易的时候将`durable_nonce`填入交易参数。

#### Astralane QUIC（低延迟）

Astralane 支持 **HTTP** 与 **QUIC** 两种传输方式。QUIC 可减少连接开销，降低提交延迟。使用 QUIC 时，将 `SwqosConfig::Astralane` 的第四个参数设为 `Some(SwqosTransport::Quic)`。Astralane 的 QUIC 服务使用**单一端点**（无分区域端点），选 QUIC 时 SDK 会忽略 `region` 与可选自定义 URL；为与其他 SWQoS 配置一致，可传入相同 region。

```rust
use sol_trade_sdk::{SwqosConfig, SwqosRegion, SwqosTransport};

// Astralane 使用 QUIC（低延迟）；region 会被忽略（QUIC 单一端点）
let swqos_configs: Vec<SwqosConfig> = vec![
    SwqosConfig::Default(rpc_url.clone()),
    SwqosConfig::Astralane(
        "your_astralane_api_key".to_string(),
        SwqosRegion::Frankfurt,  // 与其他服务一致即可；QUIC 时会被忽略
        None,
        Some(SwqosTransport::Quic),
    ),
];
// 然后照常使用 swqos_configs 创建 TradeConfig / TradingClient
```

- **HTTP**（默认）：第四个参数为 `None` 或 `Some(SwqosTransport::Http)`；region 与可选自定义 URL 生效。
- **QUIC**：第四个参数为 `Some(SwqosTransport::Quic)`；SDK 使用单一 QUIC 端点并忽略 region。与 HTTP 使用同一 API key。

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

### 🔍 Durable Nonce

使用 Durable Nonce 来实现交易重放保护和优化交易处理。详细信息请参阅 [Nonce 使用指南](docs/NONCE_CACHE_CN.md)。

## 💰 Cashback 支持（PumpFun / PumpSwap）

PumpFun 与 PumpSwap 支持**返现（Cashback）**：部分手续费可返还给用户。SDK **必须知道**该代币是否开启返现，才能为 buy/sell 指令传入正确的账户（例如返现代币需要把 `UserVolumeAccumulator` 作为 remaining account）。

- **参数来自 RPC 时**：使用 `PumpFunParams::from_mint_by_rpc` 或 `PumpSwapParams::from_pool_address_by_rpc` / `from_mint_by_rpc` 时，SDK 会从链上读取 `is_cashback_coin`，无需额外传入。
- **参数来自事件/解析器时**：若根据交易事件（如 [sol-parser-sdk](https://github.com/0xfnzero/sol-parser-sdk)）构建参数，**必须**把返现标志传给 SDK：
  - **PumpFun**：`PumpFunParams::from_trade(..., is_cashback_coin)` 与 `PumpFunParams::from_dev_trade(..., is_cashback_coin)` 最后一个参数为 `is_cashback_coin`。从解析出的事件传入（如 sol-parser-sdk 的 `PumpFunTradeEvent.is_cashback_coin`）。
  - **PumpSwap**：`PumpSwapParams` 有字段 `is_cashback_coin`。手动构造参数（如从池/交易事件）时，从解析到的池或事件数据中设置该字段。
- **pumpfun_copy_trading**、**pumpfun_sniper_trading** 示例使用 sol-parser-sdk 订阅 gRPC 事件，并在构造参数时传入 `e.is_cashback_coin`。
- **领取返现**：使用 `client.claim_cashback_pumpfun()` 和 `client.claim_cashback_pumpswap(...)` 领取累计的返现。

#### PumpFun：Creator Rewards Sharing（creator_vault）

部分 PumpFun 代币启用了 **Creator Rewards Sharing**，链上 `creator_vault` 可能与默认推导结果不同。若在**卖出**时复用**买入**时缓存的 params，可能触发程序错误 **2006（seeds constraint violated）**。建议：

- **来自 gRPC/事件（无需 RPC）**：`creator` 与 `creator_vault` 均可从解析后的事件中直接拿到：
  - **sol-parser-sdk**：推送前会调用 `fill_trade_accounts`，从 buy/sell 指令账户补全 `creator_vault`（buy 索引 9，sell 索引 8）；`creator` 来自 TradeEvent 日志。用 `PumpFunParams::from_trade(..., e.creator, e.creator_vault, ...)` 或 `from_dev_trade(..., e.creator, e.creator_vault, ...)` 即可。
  - **solana-streamer**：指令解析时从 accounts[9]（buy）/ accounts[8]（sell）写入 `creator_vault`；`creator` 来自合并后的 CPI TradeEvent 日志。同样用事件的 `e.creator`、`e.creator_vault` 调用 `from_trade` / `from_dev_trade`。
- **RPC 后覆盖**：若通过 `PumpFunParams::from_mint_by_rpc` 得到 params，之后又从 gRPC 拿到更新的 `creator_vault`，在卖出前对 params 调用 `.with_creator_vault(latest_creator_vault)`。

SDK 不会在每次卖出时通过 RPC 拉取 creator_vault（以避免延迟）；请从 gRPC/事件中传入最新 vault。

#### PumpSwap：从事件拿 coin_creator_vault（无需 RPC）

**PumpSwap**（Pump AMM）的 buy/sell 指令需要 `coin_creator_vault_ata` 与 `coin_creator_vault_authority`，二者均可从解析事件中拿到，无需 RPC：

- **sol-parser-sdk**：指令解析从账户 17、18 写入；若事件来自日志，账户填充器也会从指令补全。用 `PumpSwapParams::from_trade(..., e.coin_creator_vault_ata, e.coin_creator_vault_authority, ...)` 即可。
- **solana-streamer**：指令解析从 `accounts.get(17)`、`accounts.get(18)` 写入。同样用事件的 `coin_creator_vault_ata`、`coin_creator_vault_authority` 调用 `from_trade`。

## 🛡️ MEV 保护服务

可以通过官网申请密钥：[社区官网](https://fnzero.dev/swqos)

- **Jito**: 高性能区块空间
- **ZeroSlot**: 零延迟交易
- **Temporal**: 时间敏感交易
- **Bloxroute**: 区块链网络加速
- **FlashBlock**: 高速交易执行，支持 API 密钥认证
- **BlockRazor**: 高速交易执行，支持 API 密钥认证
- **Node1**: 高速交易执行，支持 API 密钥认证
- **Astralane**: 区块链网络加速（支持 HTTP 与 QUIC，见上方 [Astralane QUIC](#astralane-quic低延迟)）
- **LunarLander**: HelloMoon 交易着陆服务（最低小费：0.001 SOL）

## 📁 项目结构

```
src/
├── common/           # 通用功能和工具
├── constants/        # 常量定义
├── instruction/      # 指令构建
│   └── utils/        # 指令工具函数
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

