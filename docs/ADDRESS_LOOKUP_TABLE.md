# Address Lookup Table Guide

This guide explains how to use Address Lookup Tables (ALT) in Sol Trade SDK to optimize transaction size and reduce fees.

## 📋 What are Address Lookup Tables?

Address Lookup Tables are a Solana feature that allows you to store frequently used addresses in a compact table format. Instead of including full 32-byte addresses in transactions, you can reference addresses by their index in the lookup table, significantly reducing transaction size and cost.

## 🚀 Core Benefits

- **Transaction Size Optimization**: Reduce transaction size by using address indices instead of full addresses
- **Cost Reduction**: Lower transaction fees due to reduced transaction size
- **Performance Improvement**: Faster transaction processing and validation
- **Network Efficiency**: Reduced bandwidth usage and block space consumption

## 🛠️ Implementation

### 1. Setting up Address Lookup Table Cache

The SDK provides a global cache to manage address lookup tables:

```rust
use sol_trade_sdk::common::address_lookup_cache::AddressLookupTableCache;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

/// Setup lookup table cache
async fn setup_lookup_table_cache(
    client: Arc<SolanaRpcClient>,
    lookup_table_address: Pubkey,
) -> AnyResult<()> {
    AddressLookupTableCache::get_instance()
        .set_address_lookup_table(client, &lookup_table_address)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set address lookup table: {}", e))?;
    Ok(())
}
```

### 2. Using Lookup Tables in Trade Parameters

Include lookup tables in your trade parameters:

```rust
// Initialize lookup table
let lookup_table_key = Pubkey::from_str("your_lookup_table_address_here").unwrap();
setup_lookup_table_cache(client.rpc.clone(), lookup_table_key).await?;

// Include lookup table in trade parameters
let buy_params = sol_trade_sdk::TradeBuyParams {
    dex_type: DexType::PumpFun,
    mint: mint_pubkey,
    sol_amount: buy_sol_amount,
    slippage_basis_points: Some(100),
    recent_blockhash: recent_blockhash,
    extension_params: Box::new(PumpFunParams::from_trade(&trade_info, None)),
    lookup_table_key: Some(lookup_table_key), // Include lookup table
    wait_transaction_confirmed: true,
    create_wsol_ata: false,
    close_wsol_ata: false,
    create_mint_ata: true,
    open_seed_optimize: false,
    custom_cu_limit: None,
};

// Execute transaction
client.buy(buy_params).await?;
```

## 📊 Performance Comparison

| Aspect | Without ALT | With ALT | Improvement |
|--------|-------------|----------|-------------|
| **Transaction Size** | ~1,232 bytes | ~800 bytes | 35% reduction |
| **Address Storage** | 32 bytes per address | 1 byte per address | 97% reduction |
| **Transaction Fees** | Higher | Lower | Up to 30% savings |
| **Block Space Usage** | More | Less | Improved network efficiency |

## ⚠️ Important Notes

1. **Lookup Table Address**: Must provide a valid address lookup table address
2. **Cache Management**: SDK automatically manages lookup table cache
3. **RPC Compatibility**: Ensure your RPC provider supports lookup tables
4. **Network Specific**: Lookup tables are network-specific (mainnet/devnet/testnet)
5. **Testing**: Always test on devnet before using on mainnet

## 🔗 Related Documentation

- [Trading Parameters Reference](TRADING_PARAMETERS.md)
- [Example: Address Lookup Table](../examples/address_lookup/)

## 📚 External Resources

- [Solana Address Lookup Tables Documentation](https://docs.solana.com/developing/lookup-tables)