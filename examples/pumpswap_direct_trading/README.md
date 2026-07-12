# PumpSwap Direct Trading

[中文](README_CN.md)

RPC-driven PumpSwap buy/wait/sell example without an event stream. It resolves pool state through RPC, buys, reads the wallet token balance, refreshes pool state, and sells.

> This template submits real mainnet transactions. Set `PRIVATE_KEY` and `RPC_URL`, replace mint and pool, then review amounts, slippage, fee strategy, ATA flags, and whether WSOL accounts should be closed.

```bash
cargo run --package pumpswap_direct_trading
```

For new code, prefer `SimpleBuyParams` with `BuyAmount::WithMaxInput` or an explicitly selected intent. Fetch a fresh blockhash before the delayed sell; do not reuse pre-buy reserves.

For an event-driven version, see [`pumpswap_trading`](../pumpswap_trading/README.md).
