# Raydium CPMM Trading

[中文](README_CN.md)

Consumes Raydium CPMM events from `solana-streamer-sdk`, maps event pool data into SDK params, then performs one buy and sell.

> This is a live-transaction template. Set `PRIVATE_KEY`, RPC/stream configuration, target filters, amounts, and fee settings before running.

```bash
cargo run --package raydium_cpmm_trading
```

Production bots must initialize the trading client and blockhash cache before subscription, filter and deduplicate events, reject stale events, and refresh state before delayed sells. The current code focuses on CPMM parameter mapping.
