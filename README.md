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
  - [⚙️ SWQOS Service Configuration](#️-swqos-service-configuration)
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

## ✨ Features

1. **PumpFun Trading**: Support for `buy` and `sell` operations
2. **PumpSwap Trading**: Support for PumpSwap pool trading operations
3. **Bonk Trading**: Support for Bonk trading operations
4. **Raydium CPMM Trading**: Support for Raydium CPMM (Concentrated Pool Market Maker) trading operations
5. **Raydium AMM V4 Trading**: Support for Raydium AMM V4 (Automated Market Maker) trading operations
6. **Meteora DAMM V2 Trading**: Support for Meteora DAMM V2 (Dynamic AMM) trading operations
7. **Multiple MEV Protection**: Support for Jito, Nextblock, ZeroSlot, Temporal, Bloxroute, FlashBlock, BlockRazor, Node1, Astralane and other services
8. **Concurrent Trading**: Send transactions using multiple MEV services simultaneously; the fastest succeeds while others fail
9. **Unified Trading Interface**: Use unified trading protocol enums for trading operations
10. **Middleware System**: Support for custom instruction middleware to modify, add, or remove instructions before transaction execution
11. **Shared Infrastructure**: Share expensive RPC and SWQOS clients across multiple wallets for reduced resource usage

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
sol-trade-sdk = { path = "./sol-trade-sdk", version = "3.4.0" }
```

### Use crates.io

```toml
# Add to your Cargo.toml
sol-trade-sdk = "3.4.0"
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
// Multiple SWQOS services can be configured
let swqos_configs: Vec<SwqosConfig> = vec![
    SwqosConfig::Default(rpc_url.clone()),
    SwqosConfig::Jito("your uuid".to_string(), SwqosRegion::Frankfurt, None),
    SwqosConfig::Bloxroute("your api_token".to_string(), SwqosRegion::Frankfurt, None),
];
// Create TradeConfig instance
let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);

// Optional: customize WSOL ATA and seed optimization
// let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment)
//     .with_wsol_ata_config(true, true);  // create_wsol_ata_on_startup, use_seed_optimize

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

### ⚙️ SWQOS Service Configuration

When configuring SWQOS services, note the different parameter requirements for each service:

- **Jito**: The first parameter is UUID (if no UUID, pass an empty string `""`)
- **Other MEV services**: The first parameter is the API Token

#### Custom URL Support

Each SWQOS service now supports an optional custom URL parameter:

```rust
// Using custom URL (third parameter)
let jito_config = SwqosConfig::Jito(
    "your_uuid".to_string(),
    SwqosRegion::Frankfurt, // This parameter is still required but will be ignored
    Some("https://custom-jito-endpoint.com".to_string()) // Custom URL
);

// Using default regional endpoint (third parameter is None)
let bloxroute_config = SwqosConfig::Bloxroute(
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

PumpFun and PumpSwap support **cashback** for eligible tokens: part of the trading fee can be returned to the user. When you use this SDK to execute `buy` or `sell`, the transaction is submitted as usual; if the token has cashback enabled, the protocol will credit cashback according to its rules.

- **Trading**: No change to your code—use `TradeBuyParams` / `TradeSellParams` as normal. Cashback is handled on-chain.
- **Event parsing**: If you consume chain events (e.g. via [sol-parser-sdk](https://github.com/0xfnzero/sol-parser-sdk)), trade events can expose cashback-related fields (e.g. `cashback_fee_basis_points`, `cashback`, `is_cashback_enabled`) so your strategy or analytics can be cashback-aware.

## 🛡️ MEV Protection Services

You can apply for a key through the official website: [Community Website](https://fnzero.dev/swqos)

- **Jito**: High-performance block space
- **ZeroSlot**: Zero-latency transactions
- **Temporal**: Time-sensitive transactions
- **Bloxroute**: Blockchain network acceleration
- **FlashBlock**: High-speed transaction execution with API key authentication - [Official Documentation](https://doc.flashblock.trade/)
- **BlockRazor**: High-speed transaction execution with API key authentication - [Official Documentation](https://blockrazor.gitbook.io/blockrazor/)
- **Node1**: High-speed transaction execution with API key authentication - [Official Documentation](https://node1.me/docs.html) 
- **Astralane**: Blockchain network acceleration

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

## ⚠️ Important Notes

1. Test thoroughly before using on mainnet
2. Properly configure private keys and API tokens
3. Pay attention to slippage settings to avoid transaction failures
4. Monitor balances and transaction fees
5. Comply with relevant laws and regulations
