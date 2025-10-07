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

Include lookup tables in your trade parameters:

```rust
let lookup_table_key = Pubkey::from_str("use_your_lookup_table_key_here").unwrap();
let address_lookup_table_account = fetch_address_lookup_table_account(&client.rpc, &lookup_table_key).await.ok();

// Include lookup table in trade parameters
let buy_params = sol_trade_sdk::TradeBuyParams {
    dex_type: DexType::PumpFun,
    mint: mint_pubkey,
    sol_amount: buy_sol_amount,
    slippage_basis_points: Some(100),
    recent_blockhash: Some(recent_blockhash),
    extension_params: Box::new(PumpFunParams::from_trade(&trade_info, None)),
    address_lookup_table_account: address_lookup_table_account, // Include lookup table
    wait_transaction_confirmed: true,
    create_wsol_ata: false,
    close_wsol_ata: false,
    create_mint_ata: true,
    open_seed_optimize: false,
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
3. **RPC Compatibility**: Ensure your RPC provider supports lookup tables
4. **Network Specific**: Lookup tables are network-specific (mainnet/devnet/testnet)
5. **Testing**: Always test on devnet before using on mainnet

## 🔗 Related Documentation

- [Trading Parameters Reference](TRADING_PARAMETERS.md)
- [Example: Address Lookup Table](../examples/address_lookup/)

## 📚 External Resources

- [Solana Address Lookup Tables Documentation](https://docs.solana.com/developing/lookup-tables)