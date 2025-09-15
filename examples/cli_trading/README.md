# SOL Trade CLI

A command-line tool for trading tokens on Solana, supporting multiple DEX trading and wallet management features.

## Features

This CLI tool supports the following operation modes:

### 🚀 **Trading Features**

- **Buy Tokens** - Purchase tokens with SOL, supporting multiple DEXs
- **Sell Tokens** - Sell tokens for SOL, supporting specified amounts or sell all
- **Multi-DEX Support** - pumpfun, pumpswap, bonk, raydium_v4, raydium_cpmm

### 💼 **Wallet Management**

- **Wallet Status** - View SOL and WSOL balances
- **SOL Wrapping** - Wrap SOL to WSOL
- **WSOL Closing** - Close WSOL account and retrieve SOL

### 🎛️ **Usage Modes**

- **Interactive Mode** - Continuous command-line interface
- **Direct Command Mode** - Single command execution

## Build

```bash
cd examples/cli_trading
cargo build --release
```

## Usage

### 1. Interactive Mode (Recommended)

Run the program directly to enter interactive mode:

```bash
cargo run
# or explicitly specify
cargo run -- interactive
```

Then enter commands in the interactive command line:

```
sol-trade> help                                                             # View help
sol-trade> wallet                                                           # Check wallet status
sol-trade> buy xxxxxxxxxxxxxx pumpfun 1.0     # Buy with 1.0 SOL
sol-trade> buy xxxxxxxxxxxxxx pumpfun 1.0 500 # Buy with 500 slippage
sol-trade> sell xxxxxxxxxxxxxx pumpfun        # Sell all tokens
sol-trade> sell xxxxxxxxxxxxxx pumpfun 100.0  # Sell 100 tokens
sol-trade> raydium_v4_buy <mint> <amm_address> 1.0                          # Raydium V4 buy
sol-trade> raydium_cpmm_buy <mint> <pool_address> 1.0                       # Raydium CPMM buy
sol-trade> wrap_sol 2.5                                                     # Wrap 2.5 SOL
sol-trade> close_wsol                                                       # Close WSOL account
sol-trade> quit                                                             # Exit
```

### 2. Direct Command Line Mode ⭐️ **New Feature**

Now supports executing single commands directly via command line arguments without entering interactive mode:

#### 📋 View Help

```bash
cargo run -- --help              # General help
cargo run -- buy --help          # Buy command help
cargo run -- sell --help         # Sell command help
```

#### 💰 Buy Tokens

```bash
# Basic buy
cargo run -- buy <mint_address> pumpfun --amount 1.0

# Buy with slippage
cargo run -- buy <mint_address> pumpfun --amount 1.0 --slippage 500

# Raydium V4 buy
cargo run -- buy <mint_address> raydium_v4 --amount 1.0 --amm <amm_address>

# Raydium CPMM buy
cargo run -- buy <mint_address> raydium_cpmm --amount 1.0 --pool <pool_address>
```

#### 💸 Sell Tokens

```bash
# Sell all tokens
cargo run -- sell <mint_address> pumpfun

# Sell specified amount
cargo run -- sell <mint_address> pumpfun --amount 100.0

# Sell with slippage
cargo run -- sell <mint_address> pumpfun --amount 100.0 --slippage 500

# Raydium V4 sell
cargo run -- sell <mint_address> raydium_v4 --amm <amm_address>

# Raydium CPMM sell
cargo run -- sell <mint_address> raydium_cpmm --pool <pool_address>
```

#### 🔄 Wallet Operations

```bash
# Wrap SOL to WSOL
cargo run -- wrap-sol --amount 1.0

# Close WSOL account
cargo run -- close-wsol

# Check wallet status
cargo run -- wallet
```

## Supported DEXs

| DEX              | Status          | Buy | Sell | Special Parameters  |
| ---------------- | --------------- | --- | ---- | ------------------- |
| **PumpFun**      | ✅ Full Support | ✅  | ✅   | None                |
| **PumpSwap**     | ✅ Full Support | ✅  | ✅   | None                |
| **Bonk**         | ✅ Full Support | ✅  | ✅   | None                |
| **Raydium V4**   | ✅ Full Support | ✅  | ✅   | Requires `--amm`    |
| **Raydium CPMM** | ✅ Full Support | ✅  | ✅   | Requires `--pool`   |

## Feature Status

✅ **Fully Implemented Features:**

- **Multi-DEX Trading** - Supports PumpFun, PumpSwap, Bonk, Raydium V4, Raydium CPMM
- **Buy/Sell** - Complete token trading functionality (real blockchain transactions)
- **Wallet Management** - SOL wrapping, WSOL closing, balance queries
- **Dual Mode Operation** - Interactive mode and direct command line mode
- **Parameter Validation** - Smart checking of required parameters (e.g., Raydium AMM/Pool addresses)
- **Complete Help System** - Detailed help documentation for every command

## Quick Start Examples

```bash
# 1. View help
cargo run -- --help

# 2. Check wallet status
cargo run -- wallet

# 3. Direct token purchase (PumpFun)
cargo run -- buy xxxxxxxxxxxxxx pumpfun --amount 0.1

# 4. Buy using Raydium V4
cargo run -- buy <mint> raydium_v4 --amount 0.1 --amm <amm_address>

# 5. Sell tokens
cargo run -- sell xxxxxxxxxxxxxx pumpfun

# 6. Start interactive mode
cargo run
```

## Feature Highlights

- 🎯 **Smart Parameter Validation** - Automatically checks required special parameters for each DEX
- 🔄 **Dual Mode Operation** - Supports both interactive and command-line usage
- 📊 **Real-time Status Display** - Wallet balances and transaction status updated in real-time
- 🛡️ **Secure Trading** - All transactions have complete error handling and confirmation
- 📝 **Detailed Logging** - Each transaction displays signature for tracking

## Security Reminders

⚠️ **Important Security Reminders:**

- This tool performs **real blockchain transactions**
- All transactions consume real SOL as transaction fees
- Ensure wallet private keys are secure and never share them
- Transactions are irreversible, please operate carefully
- Recommend testing with small amounts first
