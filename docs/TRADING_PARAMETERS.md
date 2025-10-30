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
| `dex_type` | `DexType` | ✅ | The trading protocol to use (PumpFun, PumpSwap, Bonk, RaydiumCpmm, RaydiumAmmV4, MeteoraDammV2) |
| `input_token_type` | `TradeTokenType` | ✅ | The type of input token to use (SOL, WSOL, USD1) |
| `mint` | `Pubkey` | ✅ | The public key of the token mint to purchase |
| `input_token_amount` | `u64` | ✅ | Amount of input token to spend (in smallest token units) |
| `slippage_basis_points` | `Option<u64>` | ❌ | Slippage tolerance in basis points (e.g., 100 = 1%, 500 = 5%) |
| `recent_blockhash` | `Option<Hash>` | ❌ | Recent blockhash for transaction validity |
| `extension_params` | `Box<dyn ProtocolParams>` | ✅ | Protocol-specific parameters (PumpFunParams, PumpSwapParams, etc.) |

### Advanced Configuration Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `address_lookup_table_account` | `Option<AddressLookupTableAccount>` | ❌ | Address lookup table for transaction optimization |
| `wait_transaction_confirmed` | `bool` | ✅ | Whether to wait for transaction confirmation |
| `create_input_token_ata` | `bool` | ✅ | Whether to create input token Associated Token Account |
| `close_input_token_ata` | `bool` | ✅ | Whether to close input token ATA after transaction |
| `create_mint_ata` | `bool` | ✅ | Whether to create token mint ATA |
| `open_seed_optimize` | `bool` | ✅ | Whether to use seed optimization for reduced CU consumption |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | Durable nonce information containing nonce account and current nonce value |
| `fixed_output_token_amount` | `Option<u64>` | ❌ | Optional fixed output token amount. If set, this value will be directly assigned to the output amount instead of being calculated (required for Meteora DAMM V2) |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Gas fee strategy instance for controlling transaction fees and priorities |
| `simulate` | `bool` | ✅ | Whether to simulate the transaction instead of executing it. When true, the transaction will be simulated via RPC to validate and show detailed logs, compute units consumed, and potential errors without actually submitting to the blockchain |


## TradeSellParams

The `TradeSellParams` struct contains all parameters required for executing sell orders across different DEX protocols.

### Basic Trading Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `dex_type` | `DexType` | ✅ | The trading protocol to use (PumpFun, PumpSwap, Bonk, RaydiumCpmm, RaydiumAmmV4, MeteoraDammV2) |
| `output_token_type` | `TradeTokenType` | ✅ | The type of output token to receive (SOL, WSOL, USD1) |
| `mint` | `Pubkey` | ✅ | The public key of the token mint to sell |
| `input_token_amount` | `u64` | ✅ | Amount of tokens to sell (in smallest token units) |
| `slippage_basis_points` | `Option<u64>` | ❌ | Slippage tolerance in basis points (e.g., 100 = 1%, 500 = 5%) |
| `recent_blockhash` | `Option<Hash>` | ❌ | Recent blockhash for transaction validity |
| `with_tip` | `bool` | ✅ | Whether to include tip in the transaction |
| `extension_params` | `Box<dyn ProtocolParams>` | ✅ | Protocol-specific parameters (PumpFunParams, PumpSwapParams, etc.) |

### Advanced Configuration Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `address_lookup_table_account` | `Option<Pubkey>` | ❌ | Address lookup table for transaction optimization |
| `wait_transaction_confirmed` | `bool` | ✅ | Whether to wait for transaction confirmation |
| `create_output_token_ata` | `bool` | ✅ | Whether to create output token Associated Token Account |
| `close_output_token_ata` | `bool` | ✅ | Whether to close output token ATA after transaction |
| `open_seed_optimize` | `bool` | ✅ | Whether to use seed optimization for reduced CU consumption |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | Durable nonce information containing nonce account and current nonce value |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Gas fee strategy instance for controlling transaction fees and priorities |
| `fixed_output_token_amount` | `Option<u64>` | ❌ | Optional fixed output token amount. If set, this value will be directly assigned to the output amount instead of being calculated (required for Meteora DAMM V2) |
| `simulate` | `bool` | ✅ | Whether to simulate the transaction instead of executing it. When true, the transaction will be simulated via RPC to validate and show detailed logs, compute units consumed, and potential errors without actually submitting to the blockchain |


## Parameter Categories

### 🎯 Core Trading Parameters

These parameters are essential for defining the basic trading operation:

- **dex_type**: Determines which protocol to use for trading
- **input_token_type** (buy) / **output_token_type** (sell): Specifies the base token type (SOL, WSOL, USD1)
- **mint**: Specifies the token to trade
- **input_token_amount**: Defines the trade size (for both buy and sell operations)
- **recent_blockhash**: Ensures transaction validity

### ⚙️ Transaction Control Parameters

These parameters control how the transaction is processed:

- **slippage_basis_points**: Controls acceptable price slippage
- **wait_transaction_confirmed**: Controls whether to wait for confirmation

### 🔧 Account Management Parameters

These parameters control automatic account creation and management:

- **create_input_token_ata** (buy) / **create_output_token_ata** (sell): Automatically create token accounts for input/output tokens
- **close_input_token_ata** (buy) / **close_output_token_ata** (sell): Automatically close token accounts after trading
- **create_mint_ata**: Automatically create token accounts for the traded token

### 🚀 Optimization Parameters

These parameters enable advanced optimizations:

- **address_lookup_table_account**: Use address lookup tables for reduced transaction size
- **open_seed_optimize**: Use seed-based account creation for lower CU consumption

### 🔄 Token Type Parameters

The **TradeTokenType** enum supports the following base tokens:
- **SOL**: Native Solana token (typically used with PumpFun)
- **WSOL**: Wrapped SOL token (typically used with PumpSwap, Bonk, Raydium protocols)  
- **USD1**: USD1 stablecoin (currently only supported on Bonk protocol)

### 🔄 Optional Parameters

When you need to use durable nonce, you need to fill in this parameter:
- **durable_nonce**: Durable nonce information containing nonce account and current nonce value

## Important Notes

### 🌱 Seed Optimization

When `open_seed_optimize: true`:
- ⚠️ **Warning**: Tokens purchased with seed optimization must be sold through this SDK
- ⚠️ **Warning**: Official platform selling methods may fail
- 📝 **Note**: Use `get_associated_token_address_with_program_id_fast_use_seed` to get ATA addresses

### 💰 Token Account Management

The account management parameters provide granular control:

- **Independent Control**: Create and close operations can be controlled separately
- **Batch Operations**: Create once, trade multiple times, then close
- **Rent Optimization**: Automatic rent reclamation when closing accounts

### 🔍 Address Lookup Tables

Before using `address_lookup_table_account`:
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
- **Meteora DAMM V2**: `MeteoraDammV2Params`

Refer to the respective protocol documentation for detailed parameter specifications.

### 🔍 Transaction Simulation

When `simulate: true`:
- **No Blockchain Submission**: The transaction is not actually submitted to the blockchain
- **Validation**: Validates transaction construction and execution without consuming actual tokens
- **Detailed Output**: Shows comprehensive information including:
  - Transaction logs with detailed execution steps
  - Compute units consumed (useful for optimizing CU budget)
  - Potential errors and failure reasons
  - Inner instructions for debugging
- **Use Cases**:
  - Testing transaction logic before real execution
  - Debugging failed transactions
  - Estimating compute unit consumption
  - Validating transaction parameters
- 📝 **Note**: Simulation uses RPC's `simulateTransaction` method with processed commitment level
