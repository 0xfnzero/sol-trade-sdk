# 📋 Trading Parameters Reference

This document provides a comprehensive reference for all trading parameters used in the Sol Trade SDK.

## 📋 Table of Contents

- [TradeBuyParams](#tradebuyparams)
- [TradeSellParams](#tradesellparams)
- [Parameter Categories](#parameter-categories)
- [Important Notes](#important-notes)

## TradeBuyParams

The `TradeBuyParams` struct contains all parameters required for executing buy orders across different DEX protocols.

### Basic Trading Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `dex_type` | `DexType` | ✅ | The trading protocol to use (PumpFun, PumpSwap, Bonk, RaydiumCpmm, RaydiumAmmV4) |
| `mint` | `Pubkey` | ✅ | The public key of the token mint to purchase |
| `sol_amount` | `u64` | ✅ | Amount of SOL to spend (in lamports) |
| `slippage_basis_points` | `Option<u64>` | ❌ | Slippage tolerance in basis points (e.g., 100 = 1%, 500 = 5%) |
| `recent_blockhash` | `Option<Hash>` | ❌ | Recent blockhash for transaction validity |
| `extension_params` | `Box<dyn ProtocolParams>` | ✅ | Protocol-specific parameters (PumpFunParams, PumpSwapParams, etc.) |

### Advanced Configuration Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `lookup_table_key` | `Option<Pubkey>` | ❌ | Address lookup table key for transaction optimization |
| `wait_transaction_confirmed` | `bool` | ✅ | Whether to wait for transaction confirmation |
| `create_wsol_ata` | `bool` | ✅ | Whether to create wSOL Associated Token Account |
| `close_wsol_ata` | `bool` | ✅ | Whether to close wSOL ATA after transaction |
| `create_mint_ata` | `bool` | ✅ | Whether to create token mint ATA |
| `open_seed_optimize` | `bool` | ✅ | Whether to use seed optimization for reduced CU consumption |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | Durable nonce information containing nonce account and current nonce value |


## TradeSellParams

The `TradeSellParams` struct contains all parameters required for executing sell orders across different DEX protocols.

### Basic Trading Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `dex_type` | `DexType` | ✅ | The trading protocol to use (PumpFun, PumpSwap, Bonk, RaydiumCpmm, RaydiumAmmV4) |
| `mint` | `Pubkey` | ✅ | The public key of the token mint to sell |
| `token_amount` | `u64` | ✅ | Amount of tokens to sell (in smallest token units) |
| `slippage_basis_points` | `Option<u64>` | ❌ | Slippage tolerance in basis points (e.g., 100 = 1%, 500 = 5%) |
| `recent_blockhash` | `Option<Hash>` | ❌ | Recent blockhash for transaction validity |
| `with_tip` | `bool` | ✅ | Whether to include tip in the transaction |
| `extension_params` | `Box<dyn ProtocolParams>` | ✅ | Protocol-specific parameters (PumpFunParams, PumpSwapParams, etc.) |

### Advanced Configuration Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `lookup_table_key` | `Option<Pubkey>` | ❌ | Address lookup table key for transaction optimization |
| `wait_transaction_confirmed` | `bool` | ✅ | Whether to wait for transaction confirmation |
| `create_wsol_ata` | `bool` | ✅ | Whether to create wSOL Associated Token Account |
| `close_wsol_ata` | `bool` | ✅ | Whether to close wSOL ATA after transaction |
| `open_seed_optimize` | `bool` | ✅ | Whether to use seed optimization for reduced CU consumption |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | Durable nonce information containing nonce account and current nonce value |


## Parameter Categories

### 🎯 Core Trading Parameters

These parameters are essential for defining the basic trading operation:

- **dex_type**: Determines which protocol to use for trading
- **mint**: Specifies the token to trade
- **sol_amount** (buy) / **token_amount** (sell): Defines the trade size
- **recent_blockhash**: Ensures transaction validity

### ⚙️ Transaction Control Parameters

These parameters control how the transaction is processed:

- **slippage_basis_points**: Controls acceptable price slippage
- **wait_transaction_confirmed**: Controls whether to wait for confirmation

### 🔧 Account Management Parameters

These parameters control automatic account creation and management:

- **create_wsol_ata**: Automatically wrap SOL to wSOL when needed
- **close_wsol_ata**: Automatically unwrap wSOL to SOL after trading
- **create_mint_ata**: Automatically create token accounts

### 🚀 Optimization Parameters

These parameters enable advanced optimizations:

- **lookup_table_key**: Use address lookup tables for reduced transaction size
- **open_seed_optimize**: Use seed-based account creation for lower CU consumption

### 🔄 Optional Parameters

When you need to use durable nonce, you need to fill in this parameter:
- **durable_nonce**: Durable nonce information containing nonce account and current nonce value

## Important Notes

### 🌱 Seed Optimization

When `open_seed_optimize: true`:
- ⚠️ **Warning**: Tokens purchased with seed optimization must be sold through this SDK
- ⚠️ **Warning**: Official platform selling methods may fail
- 📝 **Note**: Use `get_associated_token_address_with_program_id_fast_use_seed` to get ATA addresses

### 💰 wSOL Account Management

The `create_wsol_ata` and `close_wsol_ata` parameters provide granular control:

- **Independent Control**: Create and close operations can be controlled separately
- **Batch Operations**: Create once, trade multiple times, then close
- **Rent Optimization**: Automatic rent reclamation when closing accounts

### 🔍 Address Lookup Tables

Before using `lookup_table_key`:
- Initialize `AddressLookupTableCache` to manage cached lookup tables
- Lookup tables reduce transaction size and improve success rates
- Particularly beneficial for complex transactions with many account references

### 📊 Slippage Configuration

Recommended slippage settings:
- **Conservative**: 100-300 basis points (1-3%)
- **Moderate**: 300-500 basis points (3-5%)
- **Aggressive**: 500-1000 basis points (5-10%)

### 🎯 Protocol-Specific Parameters

Each DEX protocol requires specific `extension_params`:
- **PumpFun**: `PumpFunParams`
- **PumpSwap**: `PumpSwapParams`
- **Bonk**: `BonkParams`
- **Raydium CPMM**: `RaydiumCpmmParams`
- **Raydium AMM V4**: `RaydiumAmmV4Params`

Refer to the respective protocol documentation for detailed parameter specifications.
