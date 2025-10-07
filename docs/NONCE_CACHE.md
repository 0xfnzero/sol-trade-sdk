# Nonce Cache Guide

This guide explains how to use Nonce Cache in Sol Trade SDK to implement transaction replay protection and optimize transaction processing.

## 📋 What is Nonce Cache?

Nonce Cache is a global singleton cache system for managing durable nonce accounts in the Solana network. Durable nonce is a Solana feature that allows you to create transactions that remain valid for extended periods, beyond the 150-block limitation of recent block hashes.

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

### 1. Initialize Nonce Cache

First, set up the nonce account and initialize the cache:

```rust
use sol_trade_sdk::common::nonce_cache::NonceCache;

// Set up nonce account
let nonce_account_str = "your_nonce_account_address_here";
NonceCache::get_instance().init(Some(nonce_account_str.to_string()));
```

### 2. Fetch Nonce Information

Get the latest nonce information from RPC:

```rust
// Fetch and update nonce information
NonceCache::get_instance().fetch_nonce_info_use_rpc(&client.rpc).await?;
// Or manually manage nonce
// NonceCache::get_instance().update_nonce_info_partial(nonce_account, current_nonce, used);
let durable_nonce = NonceCache::get_durable_nonce_info();
```

### 3. Use Nonce in Transactions

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
    durable_nonce: Some(durable_nonce), // Set durable nonce
};

// Execute transaction
client.buy(buy_params).await?;
```

## 🔄 Nonce Lifecycle

1. **Initialize**: Set nonce account address
2. **Fetch**: Get the latest nonce value from RPC
3. **Use**: Set nonce parameters in transactions
4. **Refresh**: Fetch new nonce value before next use

## 🔗 Related Documentation

- [Example: Nonce Cache](../examples/nonce_cache/)
