# Meteora DAMM V2 Direct Trading

[中文](README_CN.md)

Demonstrates a direct Meteora DAMM V2 buy/sell flow using pool data supplied by `solana-streamer-sdk` types and SDK trading params.

> This is a live mainnet template. Set `PRIVATE_KEY`, `RPC_URL`, and real raw-unit
> `MIN_BUY_OUTPUT_AMOUNT` / `MIN_SELL_OUTPUT_AMOUNT` quotes before running.

```bash
cargo run --package meteora_damm_v2_direct_trading
```

Token program selection, pool orientation, dynamic fee state, and remaining accounts must be current. Refresh pool state and blockhash before a delayed sell. Use Token-2022-aware balance/account handling where required.

`MIN_*_OUTPUT_AMOUNT` is the on-chain minimum output, not a token UI amount. Derive it from a current quote and your slippage tolerance. The example intentionally refuses to use `1`, because that provides almost no price protection in partial-fill mode.
