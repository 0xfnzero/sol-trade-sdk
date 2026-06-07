<div align="center">
    <h1>🚀 Sol Trade SDK</h1>
    <h3><em>A comprehensive Rust SDK for seamless Solana DEX trading</em></h3>
</div>

<p align="center">
    <strong>A high-performance Rust SDK for low-latency Solana DEX trading bots. Built for speed and efficiency, it enables seamless, high-throughput interaction with PumpFun, Pump AMM (PumpSwap), Bonk, Meteora DAMM v2, Raydium AMM v4, and Raydium CPMM for latency-critical trading strategies.</strong>
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

> ☕ **Support This Project**
>
> This SDK is completely free and open source. However, maintaining and continuously updating it requires significant AI computing resources and token consumption. If this SDK helps with your trading development, consider making a monthly SOL donation — any amount is appreciated and helps keep this project alive!
>
> **Donation Wallet:** `6oW7AXz1yRb57pYSxysuXnMs2aR1ha5rzGzReZ1MjPV8`

## 📋 Table of Contents

- [✨ Features](#-features)
- [📦 Installation](#-installation)
- [🛠️ Usage Examples](#️-usage-examples)
  - [📋 Example Usage](#-example-usage)
  - [⚡ Trading Parameters](#-trading-parameters)
  - [📊 Usage Examples Summary Table](#-usage-examples-summary-table)
  - [⚙️ SWQoS Service Configuration](#️-swqos-service-configuration)
  - [Astralane (Binary / Plain / QUIC)](#astralane-binary--plain--quic)
  - [🔧 Middleware System](#-middleware-system)
  - [🔍 Address Lookup Tables](#-address-lookup-tables)
  - [🔍 Nonce Cache](#-nonce-cache)
- [💰 Cashback Support (PumpFun / PumpSwap)](#-cashback-support-pumpfun--pumpswap)
- [🔄 PumpFun V1 vs V2 Instructions](#-pumpfun-v1-vs-v2-instructions)
- [🛡️ MEV Protection Services](#️-mev-protection-services)
- [📁 Project Structure](#-project-structure)
- [📄 License](#-license)
- [💬 Contact](#-contact)
- [⚠️ Important Notes](#️-important-notes)

---

## 📦 SDK Versions

This SDK is available in multiple languages:

| Language | Repository | Description |
|----------|------------|-------------|
| **Rust** | [sol-trade-sdk](https://github.com/0xfnzero/sol-trade-sdk) | Ultra-low latency with zero-copy optimization |
| **Node.js** | [sol-trade-sdk-nodejs](https://github.com/0xfnzero/sol-trade-sdk-nodejs) | TypeScript/JavaScript for Node.js |
| **Python** | [sol-trade-sdk-python](https://github.com/0xfnzero/sol-trade-sdk-python) | Async/await native support |
| **Go** | [sol-trade-sdk-golang](https://github.com/0xfnzero/sol-trade-sdk-golang) | Concurrent-safe with goroutine support |

## 🔖 Current Release

**Rust crate:** `sol-trade-sdk = "4.0.17"`

This release refreshes PumpFun V2 WSOL quote-pool handling, keeps the default RPC submit lane active alongside SWQoS lanes, restores the fast-submit result window to 5 seconds, and aligns Raydium CPMM fixed-output swaps with the on-chain `swap_base_out` instruction. Trade execution requires a caller-supplied `recent_blockhash` or durable nonce; hot-path execution does not query RPC for blockhash, account, or balance data.

## ✨ Features

1. **PumpFun Trading**: Unified SDK-side `buy`, `sell`, and `buy_exact_quote_in` flow, selecting legacy or V2 on-chain instructions as needed (SOL/WSOL + USDC)
2. **PumpSwap Trading**: Support for PumpSwap pool trading operations
3. **Bonk Trading**: Support for Bonk trading operations
4. **Raydium CPMM Trading**: Support for Raydium CPMM (Concentrated Pool Market Maker) trading operations
5. **Raydium AMM V4 Trading**: Support for Raydium AMM V4 (Automated Market Maker) trading operations
6. **Meteora DAMM V2 Trading**: Support for Meteora DAMM V2 (Dynamic AMM) trading operations
7. **Multiple MEV Protection**: Support for Jito, Nextblock, ZeroSlot, Temporal, Bloxroute, FlashBlock, BlockRazor, Node1, Astralane and other services
8. **Concurrent Trading**: Submit through every configured SWQoS provider plus the default RPC lane; the first accepted result can return early while slower routes continue submitting
9. **Unified Trading Interface**: Use unified trading protocol enums for trading operations
10. **Middleware System**: Support for custom instruction middleware to modify, add, or remove instructions before transaction execution
11. **Shared Infrastructure**: Share expensive RPC and SWQoS clients across multiple wallets for reduced resource usage
12. **Hot-Path RPC Boundary**: Trade execution uses caller-supplied blockhash or durable nonce and never queries RPC for blockhash, account, or balance data

## 📦 Installation

### Direct Clone

Clone this project to your project directory:

```bash
cd your_project_root_directory
git clone https://github.com/0xfnzero/sol-trade-sdk
```

Add the dependency to your `Cargo.toml`:

```toml
# Add to your Cargo.toml
sol-trade-sdk = { path = "./sol-trade-sdk", version = "4.0.17" }
```

### Use crates.io

```toml
# Add to your Cargo.toml
sol-trade-sdk = "4.0.17"
```

## 🛠️ Usage Examples

### 📋 Example Usage

#### 1. Create TradingClient Instance

You can refer to [Example: Create TradingClient Instance](examples/trading_client/src/main.rs).

**Method 1: Simple (single wallet)**
```rust
// Wallet
let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
// RPC URL
let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();
let commitment = CommitmentConfig::processed();
// Multiple SWQoS services can be configured
let swqos_configs: Vec<SwqosConfig> = vec![
    SwqosConfig::Default(rpc_url.clone()),
    SwqosConfig::Jito("your uuid".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::Temporal("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::FlashBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::BlockRazor("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    // Astralane: 4th param = AstralaneTransport — Binary (default), Plain (/iris), or Quic
    SwqosConfig::Astralane("your_astralane_api_key".to_string(), SwqosRegion::Frankfurt, None, None), // Binary HTTP /irisb
    SwqosConfig::SpeedLanding("your api_token".to_string(), SwqosRegion::Frankfurt, None),
];
// Create TradeConfig instance
let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
    // .create_wsol_ata_on_startup(true)  // default: true  - check & create WSOL ATA on init
    // .use_seed_optimize(true)            // default: true  - seed optimization for ATA ops
    // .log_enabled(true)                  // default: true  - SDK timing / SWQOS logs
    // .check_min_tip(false)               // default: false - filter SWQOS below min tip
    // .swqos_cores_from_end(false)        // default: false - bind SWQOS to last N CPU cores
    // .mev_protection(false)              // default: false - MEV (Astralane QUIC :9000 or HTTP mev-protect / BlockRazor)
    .build();

// Create TradingClient
let client = TradingClient::new(Arc::new(payer), trade_config).await;
```

**Method 2: Shared infrastructure (multiple wallets)**

For multi-wallet scenarios, create the infrastructure once and share it across wallets.
See [Example: Shared Infrastructure](examples/shared_infrastructure/src/main.rs).

```rust
// Create infrastructure once (expensive)
let infra_config = InfrastructureConfig::new(rpc_url, swqos_configs, commitment);
let infrastructure = Arc::new(TradingInfrastructure::new(infra_config).await);

// Create multiple clients sharing the same infrastructure (fast)
let client1 = TradingClient::from_infrastructure(Arc::new(payer1), infrastructure.clone(), true);
let client2 = TradingClient::from_infrastructure(Arc::new(payer2), infrastructure.clone(), true);
```

#### 2. Configure Gas Fee Strategy

For detailed information about Gas Fee Strategy, see the [Gas Fee Strategy Reference](docs/GAS_FEE_STRATEGY.md).

```rust
// Create GasFeeStrategy instance
let gas_fee_strategy = GasFeeStrategy::new();
// Set global strategy
gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);
```

#### 3. Build Trading Parameters

For detailed information about all trading parameters, see the [Trading Parameters Reference](docs/TRADING_PARAMETERS.md).

```rust
use sol_trade_sdk::{
    AccountPolicy, BuyAmount, DexType, SimpleBuyParams, TradeTokenType,
    trading::core::params::DexParamEnum,
};

let buy_params = SimpleBuyParams::new(
    DexType::PumpFun,
    // Token used to pay. For PumpFun V2 SOL/WSOL quote pools, keep this as SOL
    // when you want to spend native SOL; the SDK will still use V2 accounts.
    TradeTokenType::SOL,
    // Mint of the meme/token you want to buy.
    mint_pubkey,
    // Regular PumpFun/PumpSwap buy. The SDK estimates token output and applies
    // slippage to the maximum quote cost.
    BuyAmount::WithMaxInput { quote_amount: buy_sol_amount },
    // Protocol state from parser/RPC cache, for example PumpFunParams::from_trade(...).
    DexParamEnum::PumpFun(pumpfun_params),
    // Pass a cached recent blockhash; the SDK does not fetch it on the hot path.
    recent_blockhash,
    gas_fee_strategy.clone(),
)
// 300 = 3%.
.slippage_basis_points(300)
// For bots/sniping: assume ATAs are already prepared and keep the tx small.
.account_policy(AccountPolicy::HotPathMinimal);
```

#### 4. Execute Trading

```rust
client.buy_simple(buy_params).await?;
```

### ⚡ Trading Parameters

Use `SimpleBuyParams` / `SimpleSellParams` for new integrations. They describe trading intent and hide low-level ATA flags. Most users only choose:

- `pay_with` / `receive_as`: quote token direction. Use `SOL` when the wallet spends or receives native SOL. For PumpFun V2 SOL-paired pools whose quote mint is WSOL, still use `SOL` if you want native SOL settlement.
- `amount`: trade sizing intent. Pick one enum variant instead of combining `input_token_amount`, `fixed_output_token_amount`, and `use_exact_sol_amount`.
- `account_policy`: account creation behavior. Bots usually use `HotPathMinimal`; normal apps can keep the default `Auto`.

| Parameter | Meaning | Recommendation |
|---|---|---|
| `BuyAmount::ExactInput(amount)` | Spend exactly this quote amount; slippage protects minimum output. | Normal swaps |
| `BuyAmount::WithMaxInput { quote_amount }` | Regular PumpFun/PumpSwap buy with slippage applied to max quote cost. | Sniping/arbitrage |
| `BuyAmount::ExactOutput { output_amount, max_input_amount }` | Buy an exact token amount with a max quote budget. | Exact-output workflows |
| `SellAmount::ExactInput(amount)` | Sell exactly this token amount. | Normal sells |
| `SellAmount::ExactOutput { output_amount, max_input_amount }` | Receive an exact quote amount while limiting token input, where the DEX supports it. | Exact-output sells |
| `AccountPolicy::Auto` | SDK creates practical ATAs when needed. | General usage |
| `AccountPolicy::HotPathMinimal` | Avoid ATA create/close instructions in the trade tx. | Bots, sniping, latency-sensitive flows |
| `AccountPolicy::CreateMissing` | Include ATA creation instructions where possible. | Convenience over transaction size |
| `AccountPolicy::AssumePrepared` | Caller prepared every required ATA. | Deterministic advanced flows |

Optional builder methods:

| Method | Meaning |
|---|---|
| `.slippage_basis_points(300)` | Set slippage. `300` means 3%. |
| `.address_lookup_table_account(alt)` | Attach an ALT to reduce transaction size. Useful for large PumpFun V2 transactions. |
| `.wait_tx_confirmed(true)` | Return only after confirmation. Usually disabled for fastest submit paths. |
| `.wait_for_all_submits(true)` | In fast-submit mode, wait for all SWQoS lane responses and return all signatures. |
| `.simulate(true)` | Build and simulate the transaction instead of sending it. |
| `.grpc_recv_us(ts)` | Attach upstream receive timestamp for latency tracing. |
| `.durable_nonce(nonce_info)` | Use durable nonce and clear `recent_blockhash`. Recommended when you start from `SimpleBuyParams::new(...)` / `SimpleSellParams::new(...)`. |
| `SimpleBuyParams::with_durable_nonce(...)` / `SimpleSellParams::with_durable_nonce(...)` | Construct params directly with durable nonce instead of `recent_blockhash`. |
| `SimpleSellParams::with_tip(false)` | Disable relay tips for sells. Buys use the gas fee strategy/tip settings. |

`TradeBuyParams` and `TradeSellParams` remain available as advanced low-level APIs. See the dedicated [Trading Parameters Reference](docs/TRADING_PARAMETERS.md).

#### About ShredStream

When using shred to subscribe to events, due to the nature of shreds, you cannot get complete information about transaction events.
Please ensure that the parameters your trading logic depends on are available in shreds when using them.

### 📊 Usage Examples Summary Table

| Description | Run Command | Source Code |
|-------------|-------------|-------------|
| Simple buy/sell parameter API | `cargo run --package simple_trading` | [examples/simple_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/simple_trading/src/main.rs) |
| Create and configure TradingClient instance | `cargo run --package trading_client` | [examples/trading_client](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/trading_client/src/main.rs) |
| Share infrastructure across multiple wallets | `cargo run --package shared_infrastructure` | [examples/shared_infrastructure](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/shared_infrastructure/src/main.rs) |
| PumpFun token sniping trading | `cargo run --package pumpfun_sniper_trading` | [examples/pumpfun_sniper_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpfun_sniper_trading/src/main.rs) |
| PumpFun token copy trading | `cargo run --package pumpfun_copy_trading` | [examples/pumpfun_copy_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpfun_copy_trading/src/main.rs) |
| PumpSwap trading operations | `cargo run --package pumpswap_trading` | [examples/pumpswap_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/pumpswap_trading/src/main.rs) |
| Raydium CPMM trading operations | `cargo run --package raydium_cpmm_trading` | [examples/raydium_cpmm_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/raydium_cpmm_trading/src/main.rs) |
| Raydium AMM V4 trading operations | `cargo run --package raydium_amm_v4_trading` | [examples/raydium_amm_v4_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/raydium_amm_v4_trading/src/main.rs) |
| Meteora DAMM V2 trading operations | `cargo run --package meteora_damm_v2_direct_trading` | [examples/meteora_damm_v2_direct_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/meteora_damm_v2_direct_trading/src/main.rs) |
| Bonk token sniping trading | `cargo run --package bonk_sniper_trading` | [examples/bonk_sniper_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/bonk_sniper_trading/src/main.rs) |
| Bonk token copy trading | `cargo run --package bonk_copy_trading` | [examples/bonk_copy_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/bonk_copy_trading/src/main.rs) |
| Custom instruction middleware example | `cargo run --package middleware_system` | [examples/middleware_system](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/middleware_system/src/main.rs) |
| Address lookup table example | `cargo run --package address_lookup` | [examples/address_lookup](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/address_lookup/src/main.rs) |
| Nonce example | `cargo run --package nonce_cache` | [examples/nonce_cache](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/nonce_cache/src/main.rs) |
| Wrap/unwrap SOL to/from WSOL example | `cargo run --package wsol_wrapper` | [examples/wsol_wrapper](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/wsol_wrapper/src/main.rs) |
| Seed trading example | `cargo run --package seed_trading` | [examples/seed_trading](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/seed_trading/src/main.rs) |
| Gas fee strategy example | `cargo run --package gas_fee_strategy` | [examples/gas_fee_strategy](https://github.com/0xfnzero/sol-trade-sdk/tree/main/examples/gas_fee_strategy/src/main.rs) |

### ⚙️ SWQoS Service Configuration

When configuring SWQoS services, note the different parameter requirements for each service:

- **Jito**: The first parameter is UUID (if no UUID, pass an empty string `""`)
- **Other MEV services**: The first parameter is the API Token

#### Custom URL Support

Each SWQoS service now supports an optional custom URL parameter:

```rust
// Using custom URL (third parameter)
let jito_config = SwqosConfig::Jito(
    "your_uuid".to_string(),
    SwqosRegion::Frankfurt, // This parameter is still required but will be ignored
    Some("https://custom-jito-endpoint.com".to_string()) // Custom URL
);

// Using default regional endpoint (third parameter is None)
let temporal_config = SwqosConfig::Temporal(
    "your_api_token".to_string(),
    SwqosRegion::NewYork, // Will use the default endpoint for this region
    None // No custom URL, uses SwqosRegion
);
```

**URL Priority Logic**:
- If a custom URL is provided (`Some(url)`), it will be used instead of the regional endpoint
- If no custom URL is provided (`None`), the system will use the default endpoint for the specified `SwqosRegion`
- This allows for maximum flexibility while maintaining backward compatibility 

When using multiple MEV services, you need to use `Durable Nonce`. Fetch the latest nonce value and attach it to the high-level buy/sell params:

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
    recent_blockhash, // will be cleared by `.durable_nonce(...)`
    gas_fee_strategy.clone(),
)
.durable_nonce(nonce_info)
.account_policy(AccountPolicy::HotPathMinimal);

client.buy_simple(buy_params).await?;
```

#### Astralane (Binary / Plain HTTP / QUIC)

Astralane supports **Binary** HTTP (`/irisb`), **Plain** HTTP (`/iris`), and **QUIC** (`host:7000`, or `:9000` when global `mev_protection` is true). Pass `Some(AstralaneTransport::Plain)`, `Some(AstralaneTransport::Quic)`, or use `None` / omit for **Binary** (default). Global `mev_protection` adds `mev-protect=true` on HTTP or selects QUIC port 9000.

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
// Then create TradeConfig / TradingClient as usual with swqos_configs
```

- **Binary** (default): `None` or `Some(AstralaneTransport::Binary)` — `/irisb`, bincode body.
- **Plain**: `Some(AstralaneTransport::Plain)` — `/iris`.
- **QUIC**: `Some(AstralaneTransport::Quic)` — regional `host:7000` / `:9000` (MEV); same API key.

---

### 🔧 Middleware System

The SDK provides a powerful middleware system that allows you to modify, add, or remove instructions before transaction execution. Middleware executes in the order they are added:

```rust
let middleware_manager = MiddlewareManager::new()
    .add_middleware(Box::new(FirstMiddleware))   // Executes first
    .add_middleware(Box::new(SecondMiddleware))  // Executes second
    .add_middleware(Box::new(ThirdMiddleware));  // Executes last
```

### 🔍 Address Lookup Tables

Address Lookup Tables (ALT) allow you to optimize transaction size and reduce fees by storing frequently used addresses in a compact table format. For detailed information, see the [Address Lookup Tables Guide](docs/ADDRESS_LOOKUP_TABLE.md).

### 🔍 Durable Nonce

Use Durable Nonce to implement transaction replay protection and optimize transaction processing. For detailed information, see the [Durable Nonce Guide](docs/NONCE_CACHE.md).

## 💰 Cashback Support (PumpFun / PumpSwap)

PumpFun and PumpSwap support **cashback** for eligible tokens: part of the trading fee can be returned to the user. The SDK **must know** whether the token has cashback enabled so that buy/sell instructions include the correct accounts (e.g. `UserVolumeAccumulator` as remaining account for cashback coins).

- **When params come from RPC**: If you use `PumpFunParams::from_mint_by_rpc` or `PumpSwapParams::from_pool_address_by_rpc` / `from_mint_by_rpc`, the SDK reads `is_cashback_coin` from chain—no extra step.
- **When params come from event/parser**: If you build params from trade events (e.g. [sol-parser-sdk](https://github.com/0xfnzero/sol-parser-sdk)), you **must** pass the cashback flag into the SDK:
  - **PumpFun**: `PumpFunParams::from_trade(..., mint, quote_mint, creator, ..., is_cashback_coin, mayhem_mode)` and `PumpFunParams::from_dev_trade(..., is_cashback_coin)` take an `is_cashback_coin` parameter. Set it from the parsed event (e.g. CreateEvent’s `is_cashback_enabled` or BondingCurve’s `is_cashback_coin`).
  - **PumpSwap**: `PumpSwapParams` has a field `is_cashback_coin`. When constructing params manually (e.g. from pool/trade events), set it from the parsed pool or event data.
- The **pumpfun_copy_trading** and **pumpfun_sniper_trading** examples use sol-parser-sdk for gRPC subscription and pass `e.is_cashback_coin` when building params.
- **Claim**: Use `client.claim_cashback_pumpfun()` and `client.claim_cashback_pumpswap(...)` to claim accumulated cashback.

#### PumpFun: troubleshooting (on-chain errors)

For **Anchor 2006 / `NotAuthorized` (6000) / wrong token program / BuyZeroAmount (6020) / slippage (6042)** and related issues, see **[docs/PUMP_ERRORS_AND_TROUBLESHOOTING_CN.md](docs/PUMP_ERRORS_AND_TROUBLESHOOTING_CN.md)** (Chinese). An English appendix may be added later.

#### PumpFun: Creator Rewards Sharing (creator_vault)

Some PumpFun coins use **Creator Rewards Sharing**, so the on-chain `creator_vault` can differ from the default derivation. If you reuse cached params from a **buy** when **selling**, you may see program error **2006 (seeds constraint violated)**. To avoid this:

- **From gRPC/events (no RPC needed)**: You can get both `creator` and `creator_vault` from parsed transaction events:
  - **sol-parser-sdk**: Before pushing events, the pipeline calls `fill_trade_accounts`, which fills `creator_vault` from the buy/sell instruction accounts (buy index 9, sell index 8). `creator` comes from the TradeEvent log. Use `PumpFunParams::from_trade(..., e.creator, e.creator_vault, ...)` or `from_dev_trade(..., e.creator, e.creator_vault, ...)` with the event `e`.
  - **solana-streamer**: Instruction parsers set `creator_vault` from accounts[9] (buy) or accounts[8] (sell); `creator` comes from the merged CPI TradeEvent log. Use the same `from_trade` / `from_dev_trade` with `e.creator` and `e.creator_vault`.
- **Override after RPC**: If you get params via `PumpFunParams::from_mint_by_rpc` but later receive a newer `creator_vault` from gRPC, call `.with_creator_vault(latest_creator_vault)` on the params before selling.

The SDK does not fetch creator_vault from RPC on every sell (to avoid latency); pass the up-to-date vault from gRPC/events when available.

#### PumpFun Unified Buy/Sell With V1/V2 Instructions

PumpFun has two instruction sets for bonding-curve trading:

| | V1 (default) | V2 (opt-in) |
|---|---|---|
| Instructions | `buy` / `buy_exact_sol_in` / `sell` | `buy_v2` / `buy_exact_quote_in_v2` / `sell_v2` |
| Account metas | 18 | 27 |
| Quote mint | SOL only (legacy) | SOL or USDC (via `quote_mint` field) |
| Transaction size | Smaller (fits `PACKET_DATA_SIZE` without LUT) | Larger (requires LUT for most transactions) |

The SDK-side builder is version-neutral: callers use the normal buy/sell flow, and `quote_mint` selects the correct on-chain discriminator and account layout internally. There is no user-facing V2 switch required.

**Default: V1**. When `quote_mint` is `Pubkey::default()` or the Solscan SOL sentinel (`So11111111111111111111111111111111111111111`), the SDK uses V1 instructions which produce smaller transactions that fit within the 1232-byte `PACKET_DATA_SIZE` limit without requiring an Address Lookup Table. Passing `WSOL_TOKEN_ACCOUNT` selects SOL V2; passing USDC selects USDC V2.

**Key changes in v2 instructions:**
- `quote_mint` parameter — pass wrapped SOL for SOL-paired, or USDC mint for USDC-paired
- 27 fixed accounts (buy) / 26 fixed accounts (sell) — **no optional accounts**
- `buyback_fee_recipient`, `sharing_config`, and 6 `associated_quote_*` ATAs are now mandatory
- Same pricing and cost as legacy instructions for SOL-paired coins
- USDC-paired coins must be bought with USDC and sell back to USDC. The SDK rejects SOL input for USDC quote pools before transaction submission.

**Pass `quote_mint` into `PumpFunParams::from_trade`**:

When using event/parser data, pass the event's `quote_mint` right after `mint`. `Pubkey::default()` and Solscan SOL (`So11111111111111111111111111111111111111111`) mean the legacy SOL layout; `WSOL_TOKEN_ACCOUNT` means SOL V2; USDC means USDC V2.

```rust
// quote_mint is not a PDA. It is the quote SPL mint carried by parser/gRPC events:
// - Legacy SOL pool: Pubkey::default() or Solscan SOL from parser data
// - SOL V2 pool: WSOL_TOKEN_ACCOUNT
// - USDC V2 pool: USDC mint
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
```

For USDC-paired coins, pass `USDC_TOKEN_ACCOUNT` as the buy `input_mint` and sell `output_mint`; SOL/WSOL is only valid for SOL-paired PumpFun curves.
When consuming parser events, map `quoteMint`, `virtualQuoteReserves`, and `realQuoteReserves` into `PumpFunParams::from_trade(...)`; USDC pools use `4_292_000_000` as the initial virtual quote reserve.
For legacy SOL events where `quote_mint` is `Pubkey::default()` or Solscan SOL, use `virtual_sol_reserves` / `real_sol_reserves` when the quote-reserve fields are absent or zero.

> **Note**: V2 transactions with ATA creation + durable nonce may exceed `PACKET_DATA_SIZE`. Enable an Address Lookup Table (`address_lookup_table_account`) when using V2.

#### PumpSwap: coin_creator_vault from events (no RPC)

For **PumpSwap** (Pump AMM), `coin_creator_vault_ata` and `coin_creator_vault_authority` are required in buy/sell instructions. Both are available from parsed events without RPC:

- **sol-parser-sdk**: Instruction parser sets them from accounts 17 and 18; the account filler also fills them when the event comes from logs. Use `PumpSwapParams::from_trade(..., e.coin_creator_vault_ata, e.coin_creator_vault_authority, ...)` with the buy/sell event `e`.
- **solana-streamer**: Instruction parser sets them from `accounts.get(17)` and `accounts.get(18)`. Use the same `from_trade` with the event's `coin_creator_vault_ata` and `coin_creator_vault_authority`.

## 🛡️ MEV Protection Services

You can apply for a key through the official website: [Community Website](https://fnzero.dev/swqos)

- **Jito**: High-performance block space
- **Temporal**: Time-sensitive transactions
- **FlashBlock**: High-speed transaction execution with API key authentication
- **BlockRazor**: High-speed transaction execution with API key authentication
- **Astralane**: Blockchain network acceleration (Binary/Plain HTTP and QUIC)
- **SpeedLanding**: High-speed transaction execution with API key authentication

## 📁 Project Structure

```
src/
├── common/           # Common functionality and tools
├── constants/        # Constant definitions
├── instruction/      # Instruction building
│   └── utils/        # Instruction utilities
├── swqos/            # MEV service clients
├── trading/          # Unified trading engine
│   ├── common/       # Common trading tools
│   ├── core/         # Core trading engine
│   ├── middleware/   # Middleware system
│   └── factory.rs    # Trading factory
├── utils/            # Utility functions
│   ├── calc/         # Amount calculation utilities
│   └── price/        # Price calculation utilities
└── lib.rs            # Main library file
```

## 📄 License

MIT License

## 💬 Contact

- Official Website: https://fnzero.dev/
- Project Repository: https://github.com/0xfnzero/sol-trade-sdk
- Telegram Group: https://t.me/fnzero_group
- Discord: https://discord.gg/vuazbGkqQE

## ⏱️ Timing metrics (v3.5.0+)

When `log_enabled` and SDK log are on, the executor prints `[SDK] Buy/Sell timing(...)`. **Semantics changed in v3.5.0**: `submit` is now only the send to SWQOS/RPC; `confirm` is separate; `start_to_submit` (when `grpc_recv_us` is set) is **end-to-end from gRPC event to submit**, so it is larger than in-process timings. See [docs/TIMING_METRICS.md](docs/TIMING_METRICS.md) for definitions and how to compare with older versions.

## ⚠️ Important Notes

1. Test thoroughly before using on mainnet
2. Properly configure private keys and API tokens
3. Pay attention to slippage settings to avoid transaction failures
4. Monitor balances and transaction fees
5. Comply with relevant laws and regulations
