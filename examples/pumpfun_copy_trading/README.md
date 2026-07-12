# PumpFun Copy Trading

[中文](README_CN.md)

Consumes PumpFun buy/sell events through `sol-parser-sdk` Yellowstone gRPC and maps the event's post-trade protocol fields into one follow-up buy/sell flow.

> This is a live-transaction template. Set `PRIVATE_KEY` and configure RPC, gRPC, target filtering, amounts, slippage, and fee limits before running.

```bash
export RPC_URL=https://your-rpc.example
export GRPC_ENDPOINT=https://your-yellowstone.example
cargo run --package pumpfun_copy_trading
```

Copying every event is unsafe. Add wallet/mint allowlists, signature plus instruction-index deduplication, event-age limits, position isolation, and maximum input/loss controls. Prewarm the client and blockhash cache before subscription for low latency.
