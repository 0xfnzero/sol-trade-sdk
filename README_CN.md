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

> ☕ **支持本项目**
>
> 本 SDK 完全免费且开源。但维护和持续更新需要消耗大量 AI 算力与 Token。如果这个 SDK 对您的开发有帮助，欢迎每月捐赠任意数量的 SOL，您的支持将帮助这个项目持续运行！
>
> **捐赠钱包：** `6oW7AXz1yRb57pYSxysuXnMs2aR1ha5rzGzReZ1MjPV8`

## 📋 目录

- [✨ 项目特性](#-项目特性)
- [📦 安装](#-安装)
- [🛠️ 使用示例](#️-使用示例)
  - [📋 使用示例](#-使用示例)
  - [⚡ 交易参数](#-交易参数)
  - [📊 使用示例汇总表格](#-使用示例汇总表格)
  - [⚙️ SWQoS 服务配置说明](#️-swqos-服务配置说明)
  - [Astralane（Binary / Plain / QUIC）](#astralanebinary--plain--quic)
  - [🔧 中间件系统说明](#-中间件系统说明)
  - [🔍 地址查找表](#-地址查找表)
  - [🔍 Nonce 缓存](#-nonce-缓存)
- [💰 Cashback 支持（PumpFun / PumpSwap）](#-cashback-支持pumpfun--pumpswap)
  - [Pump.fun 常见链上错误与排错（文档）](docs/PUMP_ERRORS_AND_TROUBLESHOOTING_CN.md)
- [🛡️ MEV 保护服务](#️-mev-保护服务)
- [📁 项目结构](#-项目结构)
- [📄 许可证](#-许可证)
- [💬 联系方式](#-联系方式)
- [⚠️ 重要注意事项](#️-重要注意事项)

---

## 📦 SDK 版本

本 SDK 提供多种语言版本：

| 语言 | 仓库 | 描述 |
|------|------|------|
| **Rust** | [sol-trade-sdk](https://github.com/0xfnzero/sol-trade-sdk) | 超低延迟，零拷贝优化 |
| **Node.js** | [sol-trade-sdk-nodejs](https://github.com/0xfnzero/sol-trade-sdk-nodejs) | TypeScript/JavaScript，Node.js 支持 |
| **Python** | [sol-trade-sdk-python](https://github.com/0xfnzero/sol-trade-sdk-python) | 原生 async/await 支持 |
| **Go** | [sol-trade-sdk-golang](https://github.com/0xfnzero/sol-trade-sdk-golang) | 并发安全，goroutine 支持 |

## 这个 SDK 适合什么场景

`sol-trade-sdk` 是 FnZero Solana 交易 SDK 系列的 Rust 版本，重点服务于低延迟交易构建和提交，适合 Solana DEX 交易机器人、跟单系统、狙击机器人、套利策略和私有交易基础设施。

| 方向 | 覆盖范围 |
|------|----------|
| DEX 协议 | PumpFun、PumpSwap、Bonk、Meteora DAMM v2、Raydium AMM v4、Raydium CPMM |
| 提交通道 | 默认 Solana RPC，以及 Jito、Nextblock、ZeroSlot、Temporal、Bloxroute、FlashBlock、BlockRazor、Node1、Astralane、SpeedLanding 等 SWQoS 服务 |
| 交易流程 | 买入/卖出、精确输入/输出、跟单交易、狙击交易、地址查找表、durable nonce、中间件、共享基础设施 |
| 热路径设计 | 调用方传入 recent blockhash 或 durable nonce；交易执行阶段不再查询 RPC 获取 blockhash、账户或余额 |

## 🔖 当前版本

**Rust crate:** `sol-trade-sdk = "4.0.21"`

本版本刷新 PumpFun native SOL quote 处理逻辑，SOL/WSOL sentinel 默认优先走更小的 V1 热路径，确保默认 RPC 提交通道会和 SWQoS 通道一起发出，快速提交结果等待窗口恢复为 5 秒，并将 Raydium CPMM fixed-output 交易对齐到链上 `swap_base_out` 指令。交易执行必须由调用方传入 `recent_blockhash` 或 durable nonce；热路径不会查询 RPC 获取 blockhash、账户或余额数据。

## ✨ 项目特性

1. **PumpFun 交易**: SDK 侧统一为 `buy`、`sell`、`buy_exact_quote_in` 流程，native SOL 优先走 V1，USDC/非 SOL quote 或显式 WSOL 结算才走 V2
2. **PumpSwap 交易**: 支持 PumpSwap 池的交易操作
3. **Bonk 交易**: 支持 Bonk 的交易操作
4. **Raydium CPMM 交易**: 支持 Raydium CPMM (Concentrated Pool Market Maker) 的交易操作
5. **Raydium AMM V4 交易**: 支持 Raydium AMM V4 (Automated Market Maker) 的交易操作
6. **Meteora DAMM V2 交易**: 支持 Meteora DAMM V2 (Dynamic AMM) 的交易操作
7. **多种 MEV 保护**: 支持 Jito、Temporal、FlashBlock、BlockRazor、Astralane、SpeedLanding 等服务
8. **并发交易**: 所有已配置的 SWQoS 通道和默认 RPC 通道都会发出提交；首个成功只影响返回，较慢通道会继续提交
9. **统一交易接口**: 使用统一的交易协议枚举进行交易操作
10. **中间件系统**: 支持自定义指令中间件，可在交易执行前对指令进行修改、添加或移除
11. **共享基础设施**: 多钱包可共享同一套 RPC 与 SWQoS 客户端，降低资源占用
12. **热路径 RPC 边界**: 交易执行使用调用方传入的 blockhash 或 durable nonce，不在热路径查询 blockhash、账户或余额

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
sol-trade-sdk = { path = "./sol-trade-sdk", version = "4.0.21" }
```

### 使用 crates.io

```toml
# 添加到您的 Cargo.toml
sol-trade-sdk = "4.0.21"
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
    SwqosConfig::Temporal("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::FlashBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::BlockRazor("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    // Astralane：第4个参数为 AstralaneTransport — Binary（默认）、Plain（/iris）或 Quic
    SwqosConfig::Astralane("your_astralane_api_key".to_string(), SwqosRegion::Frankfurt, None, None), // Binary /irisb
    SwqosConfig::SpeedLanding("your api_token".to_string(), SwqosRegion::Frankfurt, None),
];
// 创建 TradeConfig 实例
let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
    // .create_wsol_ata_on_startup(true)  // 默认: true  - 初始化时检查并创建 WSOL ATA
    // .use_seed_optimize(true)            // 默认: true  - ATA 操作启用 seed 优化
    // .log_enabled(true)                  // 默认: true  - SDK 计时 / SWQOS 日志
    // .check_min_tip(false)               // 默认: false - 过滤低于最低小费的 SWQOS
    // .swqos_cores_from_end(false)        // 默认: false - 将 SWQOS 绑定到末尾 N 个 CPU 核心
    // .mev_protection(false)              // 默认: false - MEV（Astralane QUIC :9000 或 HTTP mev-protect / BlockRazor）
    .build();

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
use sol_trade_sdk::{
    AccountPolicy, BuyAmount, DexType, SimpleBuyParams, TradeTokenType,
    trading::core::params::DexParamEnum,
};

let buy_params = SimpleBuyParams::new(
    DexType::PumpFun,
    // 支付币种。PumpFun V2 的 SOL/WSOL quote 池，如果你想花原生 SOL，
    // 这里仍然传 SOL；SDK 内部会按 V2 账户布局处理。
    TradeTokenType::SOL,
    // 要买入的 meme/token mint。
    mint_pubkey,
    // 常规 PumpFun/PumpSwap buy。SDK 先估算能买到多少 token，
    // 再把滑点应用到最大 quote 成本上。
    BuyAmount::WithMaxInput { quote_amount: buy_sol_amount },
    // 协议状态参数，通常来自 parser/RPC 缓存，例如 PumpFunParams::from_trade(...)。
    DexParamEnum::PumpFun(pumpfun_params),
    // 传入外部缓存的 recent_blockhash；SDK 不在热路径里临时获取。
    recent_blockhash,
    gas_fee_strategy.clone(),
)
// 300 = 3%。
.slippage_basis_points(300)
// Bot/狙击推荐：假设 ATA 已提前准备好，交易内不创建/关闭 ATA，体积更小。
.account_policy(AccountPolicy::HotPathMinimal);
```

#### 4. 执行交易

```rust
client.buy_simple(buy_params).await?;
```

### ⚡ 交易参数

新接入建议优先使用 `SimpleBuyParams` / `SimpleSellParams`。它们描述交易意图，SDK 内部处理底层 ATA 参数。多数用户只需要选择：

- `pay_with` / `receive_as`：买入时用什么 quote 支付，卖出时收什么 quote。钱包实际花/收原生 SOL 就传 `SOL`。PumpFun V2 的 SOL 配对池虽然 `quote_mint` 是 WSOL，但你想用原生 SOL 结算时这里仍传 `SOL`。
- `amount`：交易数量语义。用一个枚举表达意图，不再同时理解 `input_token_amount`、`fixed_output_token_amount`、`use_exact_sol_amount`。
- `account_policy`：账户创建策略。Bot 通常用 `HotPathMinimal`；普通应用可以保留默认 `Auto`。

| 参数 | 含义 | 推荐场景 |
|---|---|---|
| `BuyAmount::ExactInput(amount)` | 精确花费指定 quote 数量；滑点保护最小买到数量。 | 普通买入 |
| `BuyAmount::WithMaxInput { quote_amount }` | PumpFun/PumpSwap 常规 buy，滑点作用在最大 quote 成本上。 | 狙击、套利 |
| `BuyAmount::ExactOutput { output_amount, max_input_amount }` | 精确买到指定 token 数量，并限制最大 quote 成本。 | 精确输出 |
| `SellAmount::ExactInput(amount)` | 精确卖出指定 token 数量。 | 普通卖出 |
| `SellAmount::ExactOutput { output_amount, max_input_amount }` | 精确收到指定 quote 数量，并限制最多卖出多少 token；取决于 DEX 是否支持。 | 精确输出卖出 |
| `AccountPolicy::Auto` | SDK 按交易路径创建必要 ATA。 | 普通用户 |
| `AccountPolicy::HotPathMinimal` | 交易内避免创建/关闭 ATA。 | Bot、狙击、低延迟 |
| `AccountPolicy::CreateMissing` | 尽量在交易内创建缺失 ATA。 | 优先方便，不追求最小交易体积 |
| `AccountPolicy::AssumePrepared` | 调用方保证所有 ATA 已准备好。 | 高级确定性流程 |

可选 builder 方法：

| 方法 | 含义 |
|---|---|
| `.slippage_basis_points(300)` | 设置滑点。`300` 表示 3%。 |
| `.address_lookup_table_account(alt)` | 传入 ALT 以减少交易体积。PumpFun V2 交易较大时很有用。 |
| `.wait_tx_confirmed(true)` | 等链上确认后再返回。追求最快提交时通常关闭。 |
| `.wait_for_all_submits(true)` | 等待所有 SWQoS 通道返回，并拿到已提交签名。recent blockhash 多路交易不互斥；durable nonce 多路交易互斥。 |
| `.simulate(true)` | 只构建并模拟交易，不真正发送。 |
| `.grpc_recv_us(ts)` | 传入上游收到事件的微秒时间戳，用于延迟追踪。 |
| `.durable_nonce(nonce_info)` | 使用 durable nonce，并清空 `recent_blockhash`。如果你从 `SimpleBuyParams::new(...)` / `SimpleSellParams::new(...)` 开始构造，推荐用这个。 |
| `SimpleBuyParams::with_durable_nonce(...)` / `SimpleSellParams::with_durable_nonce(...)` | 直接用 durable nonce 构造参数，不使用 `recent_blockhash`。 |
| `SimpleSellParams::with_tip(false)` | 关闭卖出交易 relay tip。买入的 tip 使用 gas fee strategy 控制。 |

`TradeBuyParams` 和 `TradeSellParams` 仍保留为高级低层接口。详细说明见 [交易参数参考手册](docs/TRADING_PARAMETERS_CN.md)。

#### 关于shredstream

当你使用 shred 订阅事件时，由于 shred 的特性，你无法获取到交易事件的完整信息。
请你在使用时，确保你的交易逻辑依赖的参数，在shred中都能获取到。

客户端预热、blockhash/nonce、交易模式、事件状态新鲜度和 6040 重报价规则见 [低延迟 Bot 集成清单](docs/LOW_LATENCY_BOTS_CN.md)。

### 📊 使用示例汇总表格

完整双语索引和安全分类见 [`examples/README_CN.md`](examples/README_CN.md)。

| 描述 | 运行命令 | 使用说明 |
|------|---------|----------|
| 简化买卖参数 API | `cargo run --package simple_trading` | [README](examples/simple_trading/README_CN.md) |
| 创建和配置 TradingClient | `cargo run --package trading_client` | [README](examples/trading_client/README_CN.md) |
| 多钱包共享基础设施 | `cargo run --package shared_infrastructure` | [README](examples/shared_infrastructure/README_CN.md) |
| PumpFun 狙击 | `cargo run --package pumpfun_sniper_trading` | [README](examples/pumpfun_sniper_trading/README_CN.md) |
| PumpFun 跟单 | `cargo run --package pumpfun_copy_trading` | [README](examples/pumpfun_copy_trading/README_CN.md) |
| PumpSwap 低延迟事件流 | `cargo run --package pumpswap_trading` | [README](examples/pumpswap_trading/README_CN.md) |
| PumpSwap 直接 RPC 流程 | `cargo run --package pumpswap_direct_trading` | [README](examples/pumpswap_direct_trading/README_CN.md) |
| Raydium CPMM | `cargo run --package raydium_cpmm_trading` | [README](examples/raydium_cpmm_trading/README_CN.md) |
| Raydium AMM V4 | `cargo run --package raydium_amm_v4_trading` | [README](examples/raydium_amm_v4_trading/README_CN.md) |
| Meteora DAMM V2 | `cargo run --package meteora_damm_v2_direct_trading` | [README](examples/meteora_damm_v2_direct_trading/README_CN.md) |
| Bonk 狙击 | `cargo run --package bonk_sniper_trading` | [README](examples/bonk_sniper_trading/README_CN.md) |
| Bonk 跟单 | `cargo run --package bonk_copy_trading` | [README](examples/bonk_copy_trading/README_CN.md) |
| 指令中间件 | `cargo run --package middleware_system` | [README](examples/middleware_system/README_CN.md) |
| 地址查找表 | `cargo run --package address_lookup` | [README](examples/address_lookup/README_CN.md) |
| Durable nonce | `cargo run --package nonce_cache` | [README](examples/nonce_cache/README_CN.md) |
| SOL/WSOL 包装与解包 | `cargo run --package wsol_wrapper` | [README](examples/wsol_wrapper/README_CN.md) |
| Seed 优化 | `cargo run --package seed_trading` | [README](examples/seed_trading/README_CN.md) |
| Gas fee 策略 | `cargo run --package gas_fee_strategy` | [README](examples/gas_fee_strategy/README_CN.md) |
| 多 DEX CLI 模板 | `cargo run --package cli_trading` | [README](examples/cli_trading/README_CN.md) |

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
let temporal_config = SwqosConfig::Temporal(
    "your_api_token".to_string(),
    SwqosRegion::NewYork, // 将使用该区域的默认端点
    None // 没有自定义 URL，使用 SwqosRegion
);
```

**URL 优先级逻辑**：
- 如果提供了自定义 URL（`Some(url)`），将使用自定义 URL 而不是区域端点
- 如果没有提供自定义 URL（`None`），系统将使用指定 `SwqosRegion` 的默认端点
- 这提供了最大的灵活性，同时保持向后兼容性

当使用多个 MEV 服务时，需要使用 `Durable Nonce`。先获取最新 nonce，再挂到新的 buy/sell 参数上：

```rust
use sol_trade_sdk::{fetch_nonce_info, AccountPolicy, BuyAmount, SimpleBuyParams};

let nonce_info = fetch_nonce_info(&client.infrastructure.rpc, nonce_account)
    .await
    .expect("nonce account must be initialized");

let buy_params = SimpleBuyParams::new(
    DexType::PumpFun,
    TradeTokenType::SOL,
    mint_pubkey,
    BuyAmount::WithMaxInput { quote_amount: buy_sol_amount },
    DexParamEnum::PumpFun(pumpfun_params),
    recent_blockhash, // 会被 `.durable_nonce(...)` 清空
    gas_fee_strategy.clone(),
)
.durable_nonce(nonce_info)
.account_policy(AccountPolicy::HotPathMinimal);

client.buy_simple(buy_params).await?;
```

#### Astralane（Binary / Plain / QUIC）

Astralane 支持 **Binary** HTTP（`/irisb`）、**Plain** HTTP（`/iris`）与 **QUIC**（`host:7000`，全局 `mev_protection` 为 true 时用 `:9000`）。第四个参数：`Some(AstralaneTransport::Plain)`、`Some(AstralaneTransport::Quic)`，或 `None` 表示 **Binary**（默认）。全局 `mev_protection` 会在 HTTP 上附加 `mev-protect=true`，或为 QUIC 选择 9000 端口。

```rust
use sol_trade_sdk::{SwqosConfig, SwqosRegion, AstralaneTransport};

let swqos_configs: Vec<SwqosConfig> = vec![
    SwqosConfig::Default(rpc_url.clone()),
    SwqosConfig::Astralane(
        "your_astralane_api_key".to_string(),
        SwqosRegion::Frankfurt,
        None,
        Some(AstralaneTransport::Quic),
    ),
];
// 然后照常使用 swqos_configs 创建 TradeConfig / TradingClient
```

- **Binary**（默认）：`None` 或 `Some(AstralaneTransport::Binary)` — `/irisb`，bincode 正文。
- **Plain**：`Some(AstralaneTransport::Plain)` — `/iris`。
- **QUIC**：`Some(AstralaneTransport::Quic)` — 按区域的 `host:7000` / `:9000`（MEV）；同一 API key。

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
  - **PumpFun**：`PumpFunParams::from_trade(..., mint, quote_mint, creator, ..., is_cashback_coin, mayhem_mode)` 与 `PumpFunParams::from_dev_trade(..., is_cashback_coin)` 都需要传入 `is_cashback_coin`。从解析出的事件传入（如 sol-parser-sdk 的 `PumpFunTradeEvent.is_cashback_coin`）。
  - **PumpSwap**：`PumpSwapParams` 有字段 `is_cashback_coin`。手动构造参数（如从池/交易事件）时，从解析到的池或事件数据中设置该字段。
- **pumpfun_copy_trading**、**pumpfun_sniper_trading** 示例使用 sol-parser-sdk 订阅 gRPC 事件，并在构造参数时传入 `e.is_cashback_coin`。
- **领取返现**：使用 `client.claim_cashback_pumpfun()` 和 `client.claim_cashback_pumpswap(...)` 领取累计的返现。

#### PumpFun：常见错误与排错思路

实盘集成时若遇 **Anchor 2006、`NotAuthorized`(6000)、Token program 不匹配、6020/6042** 等，请参阅专门文档：**[Pump.fun 常见链上错误与处理思路（中文）](docs/PUMP_ERRORS_AND_TROUBLESHOOTING_CN.md)**。

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

### Pump.fun Bonding Curve 统一买卖入口与 v2 指令

Pump.fun 已升级 Bonding Curve 合约，推出**统一化 v2 指令**，通过固定账户布局同时支持 SOL 和 USDC 配对币。旧版 `buy`/`sell`/`buy_exact_sol_in` 仍可用于 SOL 配对币，且保持为默认选项。

SDK 侧调用入口保持统一：正常使用 `buy` / `sell` 流程即可，SDK 会根据 `quote_mint` 和买/卖的结算 mint 自动选择正确的链上 discriminator 和账户布局。能用 V1 的 native SOL 池会优先用 V1。

**v2 指令关键变化：**
- 新增 `quote_mint` 参数 — native SOL 配对可能表现为默认值、Solscan SOL sentinel（`So11111111111111111111111111111111111111111`）或 WSOL sentinel（`So11111111111111111111111111111111111111112`）；USDC/其他真实 quote mint 才选择 V2
- 27 个固定账户（buy）/ 26 个固定账户（sell）— **无可选账户**
- `buyback_fee_recipient`、`sharing_config` 和 6 个 `associated_quote_*` ATA 变为强制账户
- SOL 配对币的报价和成本与旧版一致，无额外开销

**使用方式：**

把事件里的 `quote_mint` 传给 `PumpFunParams::from_trade`。`quote_mint` 不是 PDA，它就是 quote SPL mint 或 native SOL sentinel；`Pubkey::default()`、Solscan SOL（`So11111111111111111111111111111111111111111`）和 `WSOL_TOKEN_ACCOUNT` 都表示 native SOL 配对，正常用 SOL 结算时默认走旧版 V1；USDC 表示 USDC V2：

```rust
// native SOL 池：可能是 Pubkey::default()、Solscan SOL sentinel 或 WSOL sentinel
// USDC / 非 SOL 池：就是实际 quote SPL mint
let quote_mint = e.quote_mint;

let params = PumpFunParams::from_trade(
    e.bonding_curve,
    e.associated_bonding_curve,
    e.mint,
    quote_mint,
    e.creator,
    e.creator_vault,
    e.virtual_token_reserves,
    e.virtual_quote_reserves,
    e.real_token_reserves,
    e.real_quote_reserves,
    close_token_account_when_sell,
    e.fee_recipient,
    e.token_program,
    e.is_cashback_coin,
    Some(e.mayhem_mode),
);

// 之后正常交易
client.buy(buy_params).await?;
client.sell(sell_params).await?;
```

USDC 配对币必须用 USDC 买入、卖出也结算为 USDC；SOL/WSOL 只适用于 SOL 配对的 PumpFun 曲线。SOL 配对的普通热路径请传 `SOL`，SDK 会用 V1；只有你明确传 `WSOL` 作为买入输入或卖出输出、希望通过已有 WSOL ATA 结算时，才会选择 V2。
SDK 会在提交前拒绝 USDC quote 池的 SOL 输入，避免链上 6063 失败。
消费 parser 事件时，需要把 `quoteMint`、`virtualQuoteReserves`、`realQuoteReserves` 传进 `PumpFunParams::from_trade(...)`；USDC 池初始虚拟 quote reserve 是 `4_292_000_000`。
legacy SOL 事件里如果 `quote_mint` 是默认值或 Solscan SOL，并且 quote reserve 字段缺失/为 0，应回退使用 `virtual_sol_reserves` / `real_sol_reserves`。

| quote_mint | 实际使用的指令 | 说明 |
|-----------|---------|------|
| 未设置（默认）/ `SOL_TOKEN_ACCOUNT` (`So111...11111`) / `WSOL_TOKEN_ACCOUNT` (`So111...11112`) | 优先旧版 `buy`/`sell`/`buy_exact_sol_in` | native SOL 配对；普通 SOL 结算走 V1，显式 WSOL 结算才走 V2 |
| `USDC_TOKEN_ACCOUNT` | `buy_v2`/`sell_v2`/`buy_exact_quote_in_v2` | USDC 配对（必须使用 v2） |

## 🛡️ MEV 保护服务

可以通过官网申请密钥：[社区官网](https://fnzero.dev/swqos)

- **Jito**: 高性能区块空间
- **Temporal**: 时间敏感交易
- **FlashBlock**: 高速交易执行，支持 API 密钥认证
- **BlockRazor**: 高速交易执行，支持 API 密钥认证
- **Astralane**: 区块链网络加速（Binary/Plain HTTP 与 QUIC）
- **SpeedLanding**: 高速交易执行，支持 API 密钥认证

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
