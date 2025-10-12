# Durable Nonce Guide

This guide explains how to use Durable Nonce in Sol Trade SDK to implement transaction replay protection and optimize transaction processing.

## 📋 What is Durable Nonce?

Durable Nonce is a Solana feature that allows you to create transactions that remain valid for extended periods, beyond the 150-block limitation of recent block hashes.

## 🚀 Core Benefits

- **Transaction Replay Protection**: Prevents identical transactions from being executed multiple times
- **Extended Time Window**: Transactions can remain valid for longer periods
- **Network Performance Optimization**: Reduces dependency on the latest block hash
- **Transaction Determinism**: Provides consistent transaction processing experience
- **Offline Transaction Support**: Supports offline processing of pre-signed transactions

## 🛠️ Implementation

### Prerequisites:

You need to create a nonce account for your payer account first.
Reference: https://solana.com/developers/guides/advanced/introduction-to-durable-nonces

### 1. Fetch Nonce Information

Directly fetch nonce information from RPC:

```rust
use sol_trade_sdk::common::nonce_cache::fetch_nonce_info;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

// Set up nonce account
let nonce_account = Pubkey::from_str("your_nonce_account_address_here")?;

// Fetch nonce information
let durable_nonce = fetch_nonce_info(&client.rpc, nonce_account).await;
```

### 2. Use Nonce in Transactions

Set nonce parameters: durable_nonce

```rust
let buy_params = sol_trade_sdk::TradeBuyParams {
    dex_type: DexType::PumpFun,
    mint: mint_pubkey,
    sol_amount: buy_sol_amount,
    slippage_basis_points: Some(100),
    recent_blockhash: Some(recent_blockhash),
    extension_params: Box::new(PumpFunParams::from_trade(&trade_info, None)),
    address_lookup_table_account: None,
    wait_transaction_confirmed: true,
    create_wsol_ata: false,
    close_wsol_ata: false,
    create_mint_ata: true,
    open_seed_optimize: false,
    durable_nonce: durable_nonce, // Set durable nonce
};

// Execute transaction
client.buy(buy_params).await?;
```

## 🔄 Nonce Usage Flow

1. **Fetch**: Get the latest nonce value from RPC
2. **Use**: Set nonce parameters in transactions
3. **Refresh**: Call `fetch_nonce_info` again before next use to get new nonce value

## 🔗 Related Documentation

- [Example: Durable Nonce](../examples/nonce_cache/)
