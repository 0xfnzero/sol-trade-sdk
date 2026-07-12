# PumpFun Sniper Trading

[中文](README_CN.md)

Uses `sol-parser-sdk` Yellowstone gRPC and selects creator-first-buy events (`is_created_buy`). Event fields are mapped into PumpFun params, including quote reserves, token program, cashback, and mayhem state, before one buy/sell flow.

> This is a live-transaction template. Set `PRIVATE_KEY`, `RPC_URL`, `GRPC_ENDPOINT`, optional `GRPC_AUTH_TOKEN`, target policy, amounts, and fees.

```bash
export RPC_URL=https://your-rpc.example
export GRPC_ENDPOINT=https://your-yellowstone.example
cargo run --package pumpfun_sniper_trading
```

The current source creates `SolanaTrade` and fetches blockhash inside event handling. For production latency, initialize both before subscription as shown in the [low-latency guide](../../docs/LOW_LATENCY_BOTS.md). Deduplicate signatures and refresh state before delayed sells.
