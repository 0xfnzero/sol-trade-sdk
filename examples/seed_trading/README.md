# Seed-Optimized Trading

[中文](README_CN.md)

Demonstrates PumpSwap buy/sell account derivation with seed optimization enabled, including the matching token-account lookup used before selling.

> This is a live mainnet transaction template. Set `PRIVATE_KEY` and `RPC_URL`, replace mint and pool values, and review amounts, slippage, fees, and account-close flags before running.

```bash
cargo run --package seed_trading
```

The same `use_seed_optimize` policy must be used when deriving accounts and building transactions. The example reuses a blockhash across a delay; production code should refresh it before the sell.
