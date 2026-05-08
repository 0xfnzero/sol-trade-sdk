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

## ✨ Features

1. **PumpFun Trading**: Support for `buy`, `sell`, `buy_exact_sol_in`, and the new unified `buy_v2`/`sell_v2`/`buy_exact_quote_in_v2` instructions (SOL + USDC)
2. **PumpSwap Trading**: Support for PumpSwap pool trading operations
3. **Bonk Trading**: Support for Bonk trading operations
4. **Raydium CPMM Trading**: Support for Raydium CPMM (Concentrated Pool Market Maker) trading operations
5. **Raydium AMM V4 Trading**: Support for Raydium AMM V4 (Automated Market Maker) trading operations
6. **Meteora DAMM V2 Trading**: Support for Meteora DAMM V2 (Dynamic AMM) trading operations
7. **Multiple MEV Protection**: Support for Jito, Nextblock, ZeroSlot, Temporal, Bloxroute, FlashBlock, BlockRazor, Node1, Astralane and other services
8. **Concurrent Trading**: Send transactions using multiple MEV services simultaneously; the fastest succeeds while others fail
9. **Unified Trading Interface**: Use unified trading protocol enums for trading operations
10. **Middleware System**: Support for custom instruction middleware to modify, add, or remove instructions before transaction execution
11. **Shared Infrastructure**: Share expensive RPC and SWQoS clients across multiple wallets for reduced resource usage

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
sol-trade-sdk = { path = "./sol-trade-sdk", version = "4.0.7" }
```

### Use crates.io

```toml
# Add to your Cargo.toml
sol-trade-sdk = "4.0.7"
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
// Import DexParamEnum for protocol-specific parameters
use sol_trade_sdk::trading::core::params::DexParamEnum;

let buy_params = sol_trade_sdk::TradeBuyParams {
  dex_type: DexType::PumpSwap,
  input_token_type: TradeTokenType::WSOL,
  mint: mint_pubkey,
  input_token_amount: buy_sol_amount,
  slippage_basis_points: slippage_basis_points,
  recent_blockhash: Some(recent_blockhash),
  // Use DexParamEnum for type-safe protocol parameters (zero-overhead abstraction)
  extension_params: DexParamEnum::PumpSwap(params.clone()),
  address_lookup_table_account: None,
  wait_transaction_confirmed: true,
  create_input_token_ata: true,
  close_input_token_ata: true,
  create_mint_ata: true,
  durable_nonce: None,
  fixed_output_token_amount: None,  // Optional: specify exact output amount
  gas_fee_strategy: gas_fee_strategy.clone(),  // Gas fee strategy configuration
  simulate: false,  // Set to true for simulation only
  use_exact_sol_amount: None,  // Use exact SOL input for PumpFun/PumpSwap (defaults to true)
};
```

#### 4. Execute Trading

```rust
client.buy(buy_params).await?;
```

### ⚡ Trading Parameters

For comprehensive information about all trading parameters including `TradeBuyParams` and `TradeSellParams`, see the dedicated [Trading Parameters Reference](docs/TRADING_PARAMETERS.md).

#### About ShredStream

When using shred to subscribe to events, due to the nature of shreds, you cannot get complete information about transaction events.
Please ensure that the parameters your trading logic depends on are available in shreds when using them.

### 📊 Usage Examples Summary Table

| Description | Run Command | Source Code |
|-------------|-------------|-------------|
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

When using multiple MEV services, you need to use `Durable Nonce`. You need to use the `fetch_nonce_info` function to get the latest `nonce` value, and use it as the `durable_nonce` when trading.

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
  - **PumpFun**: `PumpFunParams::from_trade(..., is_cashback_coin)` and `PumpFunParams::from_dev_trade(..., is_cashback_coin)` take an `is_cashback_coin` parameter. Set it from the parsed event (e.g. CreateEvent’s `is_cashback_enabled` or BondingCurve’s `is_cashback_coin`).
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

#### PumpSwap: coin_creator_vault from events (no RPC)

For **PumpSwap** (Pump AMM), `coin_creator_vault_ata` and `coin_creator_vault_authority` are required in buy/sell instructions. Both are available from parsed events without RPC:

- **sol-parser-sdk**: Instruction parser sets them from accounts 17 and 18; the account filler also fills them when the event comes from logs. Use `PumpSwapParams::from_trade(..., e.coin_creator_vault_ata, e.coin_creator_vault_authority, ...)` with the buy/sell event `e`.
- **solana-streamer**: Instruction parser sets them from `accounts.get(17)` and `accounts.get(18)`. Use the same `from_trade` with the event’s `coin_creator_vault_ata` and `coin_creator_vault_authority`.

### Pump.fun Bonding Curve v2 (buy_v2 / sell_v2 / buy_exact_quote_in_v2)

Pump.fun has upgraded the Bonding Curve contract with **unified v2 instructions** that support both SOL-paired and USDC-paired coins through a single fixed account layout. The legacy `buy`/`sell`/`buy_exact_sol_in` instructions continue to work for SOL-paired coins and remain the default.

**Key changes in v2 instructions:**
- `quote_mint` parameter — pass wrapped SOL (`So11111111111111111111111111111111111111112`) for SOL-paired, or USDC mint for USDC-paired
- 27 fixed accounts (buy) / 26 fixed accounts (sell) — **no optional accounts**
- `buyback_fee_recipient`, `sharing_config`, and 6 `associated_quote_*` ATAs are now mandatory
- Same pricing and cost as legacy instructions for SOL-paired coins

**Using v2 instructions:**

Set `quote_mint` on your `PumpFunParams`. The SDK automatically switches to `buy_v2`/`sell_v2`/`buy_exact_quote_in_v2` discriminators with the 27/26-account layout:

```rust
use sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
use sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT;

// SOL-paired coin — use wrapped SOL mint
let params = PumpFunParams::from_trade(/* ... */)
    .with_quote_mint(WSOL_TOKEN_ACCOUNT);

// USDC-paired coin (coming soon — requires v2)
let params = PumpFunParams::from_trade(/* ... */)
    .with_quote_mint(USDC_TOKEN_ACCOUNT);

// Then trade as usual
client.buy(buy_params).await?;
client.sell(sell_params).await?;
```

| Quote Mint | `use_v2_ix` | Instruction Used | Notes |
|-----------|-------------|-----------------|-------|
| Not set (default) | `false` | Legacy `buy`/`sell`/`buy_exact_sol_in` | Backward compatible, SOL-only |
| `WSOL_TOKEN_ACCOUNT` | `true` | `buy_v2`/`sell_v2`/`buy_exact_quote_in_v2` | SOL-paired, unified layout |
| `USDC_TOKEN_ACCOUNT` | `true` | `buy_v2`/`sell_v2`/`buy_exact_quote_in_v2` | USDC-paired (requires v2) |

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
