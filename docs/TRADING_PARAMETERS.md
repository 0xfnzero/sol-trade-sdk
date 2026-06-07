# 📋 Trading Parameters Reference

This document provides a comprehensive reference for all trading parameters used in the Sol Trade SDK.

## 📋 Table of Contents

- [SimpleBuyParams / SimpleSellParams](#simplebuyparams--simplesellparams)
- [TradeBuyParams](#tradebuyparams)
- [TradeSellParams](#tradesellparams)
- [Parameter Categories](#parameter-categories)
- [Important Notes](#important-notes)

## SimpleBuyParams / SimpleSellParams

Use `SimpleBuyParams` and `SimpleSellParams` for new integrations. They keep the public API focused on trading intent and map to the lower-level `TradeBuyParams` / `TradeSellParams` internally.

### SimpleBuyParams

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `dex_type` | `DexType` | ✅ | Protocol to trade through, for example `DexType::PumpFun`. |
| `pay_with` | `TradeTokenType` | ✅ | Quote token used to pay for the buy. Use `SOL` when the wallet spends native SOL. For PumpFun V2 SOL/WSOL quote pools, still use `SOL` if you want native SOL settlement. |
| `mint` | `Pubkey` | ✅ | Mint of the token being bought. |
| `amount` | `BuyAmount` | ✅ | Buy sizing intent. Choose one enum variant instead of combining low-level amount flags. |
| `extension_params` | `DexParamEnum` | ✅ | Protocol state from parser/RPC cache, such as `DexParamEnum::PumpFun(PumpFunParams::from_trade(...))`. |
| `recent_blockhash` | `Hash` | ✅ for `new` | Cached recent blockhash for non-nonce transactions. The SDK does not fetch this on the hot path. |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Compute unit price/limit and relay tip configuration. |
| `slippage_basis_points` | `Option<u64>` | ❌ | Optional slippage override. `100` means 1%. |
| `account_policy` | `AccountPolicy` | ❌ | ATA creation/close behavior. Default is `Auto`. |
| `address_lookup_table_account` | `Option<AddressLookupTableAccount>` | ❌ | Optional ALT to reduce transaction size. |
| `wait_tx_confirmed` | `bool` | ❌ | Whether to wait for chain confirmation before returning. Default is `false`. |
| `wait_for_all_submits` | `bool` | ❌ | Fast-submit mode only: wait for every SWQoS lane response and return all signatures. |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | Durable nonce info. Use `SimpleBuyParams::with_durable_nonce(...)`; do not combine with `recent_blockhash`. |
| `simulate` | `bool` | ❌ | Build and simulate instead of submitting. Default is `false`. |
| `grpc_recv_us` | `Option<i64>` | ❌ | Upstream receive timestamp in microseconds for latency tracing. |

### SimpleSellParams

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `dex_type` | `DexType` | ✅ | Protocol to trade through, for example `DexType::PumpFun`. |
| `receive_as` | `TradeTokenType` | ✅ | Quote token to receive from the sell. Use `SOL` when you want native SOL output. |
| `mint` | `Pubkey` | ✅ | Mint of the token being sold. |
| `amount` | `SellAmount` | ✅ | Sell sizing intent. |
| `extension_params` | `DexParamEnum` | ✅ | Protocol state from parser/RPC cache. |
| `recent_blockhash` | `Hash` | ✅ for `new` | Cached recent blockhash for non-nonce transactions. |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Compute unit price/limit and relay tip configuration. |
| `slippage_basis_points` | `Option<u64>` | ❌ | Optional slippage override. `100` means 1%. |
| `account_policy` | `AccountPolicy` | ❌ | ATA creation/close behavior. Default is `Auto`. |
| `address_lookup_table_account` | `Option<AddressLookupTableAccount>` | ❌ | Optional ALT to reduce transaction size. |
| `wait_tx_confirmed` | `bool` | ❌ | Whether to wait for chain confirmation before returning. Default is `false`. |
| `wait_for_all_submits` | `bool` | ❌ | Fast-submit mode only: wait for every SWQoS lane response and return all signatures. |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | Durable nonce info. Use `SimpleSellParams::with_durable_nonce(...)`; do not combine with `recent_blockhash`. |
| `simulate` | `bool` | ❌ | Build and simulate instead of submitting. Default is `false`. |
| `with_tip` | `bool` | ❌ | Whether sells include relay tips. Default is `true`; set with `.with_tip(false)`. |
| `grpc_recv_us` | `Option<i64>` | ❌ | Upstream receive timestamp in microseconds for latency tracing. |

### Amount Selection

| Variant | Meaning | Low-level mapping |
|---------|---------|-------------------|
| `BuyAmount::ExactInput(amount)` | Spend exactly this quote amount; slippage protects minimum token output. | `input_token_amount = amount`, `use_exact_sol_amount = Some(true)` |
| `BuyAmount::WithMaxInput { quote_amount }` | Regular PumpFun/PumpSwap buy. The SDK estimates output and applies slippage to max quote cost. | `input_token_amount = quote_amount`, `use_exact_sol_amount = Some(false)` |
| `BuyAmount::ExactOutput { output_amount, max_input_amount }` | Buy an exact token amount while limiting max quote input. | `fixed_output_token_amount = Some(output_amount)`, `input_token_amount = max_input_amount` |
| `SellAmount::ExactInput(amount)` | Sell exactly this token amount; slippage protects minimum quote output. | `input_token_amount = amount` |
| `SellAmount::ExactOutput { output_amount, max_input_amount }` | Receive an exact quote amount while limiting token input, where supported. | `fixed_output_token_amount = Some(output_amount)`, `input_token_amount = max_input_amount` |

### AccountPolicy

| Variant | Behavior | Use when |
|---------|----------|----------|
| `Auto` | SDK creates practical ATAs when needed. Buy creates the target mint ATA; sell creates the output ATA for non-SOL outputs. | Normal apps and manual trading tools. |
| `HotPathMinimal` | No ATA create/close instructions in the trade transaction. | Bots, sniping, arbitrage, and any path sensitive to transaction size. |
| `CreateMissing` | Include ATA creation where possible. | Convenience matters more than smallest transaction size. |
| `AssumePrepared` | Do not create or close token accounts; caller prepared everything. | Advanced deterministic flows. |

## TradeBuyParams

`TradeBuyParams` is the advanced low-level buy API. New integrations should prefer `SimpleBuyParams` unless they need direct control over individual ATA flags.

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
| `wait_tx_confirmed` | `bool` | ✅ | Whether to wait for transaction confirmation |
| `create_input_token_ata` | `bool` | ✅ | Whether to create input token Associated Token Account |
| `close_input_token_ata` | `bool` | ✅ | Whether to close input token ATA after transaction |
| `create_mint_ata` | `bool` | ✅ | Whether to create token mint ATA |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | Durable nonce information containing nonce account and current nonce value |
| `fixed_output_token_amount` | `Option<u64>` | ❌ | Optional fixed output token amount. On exact-out capable DEXes, this uses the exact-out instruction and treats input_token_amount as the max input budget (required for Meteora DAMM V2) |
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
| `wait_tx_confirmed` | `bool` | ✅ | Whether to wait for transaction confirmation |
| `create_output_token_ata` | `bool` | ✅ | Whether to create output token Associated Token Account |
| `close_output_token_ata` | `bool` | ✅ | Whether to close output token ATA after transaction |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | Durable nonce information containing nonce account and current nonce value |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Gas fee strategy instance for controlling transaction fees and priorities |
| `fixed_output_token_amount` | `Option<u64>` | ❌ | Optional fixed output token amount. On exact-out capable DEXes, this uses the exact-out instruction and treats input_token_amount as the max input budget (required for Meteora DAMM V2) |
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
- **wait_tx_confirmed**: Controls whether to wait for confirmation

### 🔧 Account Management Parameters

These parameters control automatic account creation and management:

- **create_input_token_ata** (buy) / **create_output_token_ata** (sell): Automatically create token accounts for input/output tokens
- **close_input_token_ata** (buy) / **close_output_token_ata** (sell): Automatically close token accounts after trading
- **create_mint_ata**: Automatically create token accounts for the traded token

### 🚀 Optimization Parameters

These parameters enable advanced optimizations:

- **address_lookup_table_account**: Use address lookup tables for reduced transaction size

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

Seed optimization is now configured globally in `TradeConfig` when creating the `SolanaTrade` instance:

```rust
// Enable seed optimization globally (default: true)
let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment)
    .with_wsol_ata_config(
        true,  // create_wsol_ata_on_startup: Check and create WSOL ATA on startup (default: true)
        true   // use_seed_optimize: Enable seed optimization for all ATA operations (default: true)
    );
```

When seed optimization is enabled:
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
