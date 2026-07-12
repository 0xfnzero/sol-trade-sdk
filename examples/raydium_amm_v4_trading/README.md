# Raydium AMM V4 Trading

[中文](README_CN.md)

Consumes Raydium AMM V4 events from `solana-streamer-sdk`, builds AMM params, and executes one buy/sell flow.

> This is a live-transaction template. Set `PRIVATE_KEY`, RPC/stream configuration, target filters, token amounts, and fees before running.

```bash
cargo run --package raydium_amm_v4_trading
```

AMM address, market accounts, vaults, token programs, and reserve direction must match the event. For low latency, prewarm the client and blockhash cache and refresh delayed-sell state rather than reusing the triggering snapshot.
