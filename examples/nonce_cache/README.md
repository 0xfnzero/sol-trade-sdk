# Durable Nonce

[中文](README_CN.md)

Shows how to fetch a durable nonce and attach it to a PumpFun transaction triggered by a `sol-parser-sdk` gRPC event.

> This is a live-transaction template. Set `PRIVATE_KEY`, replace `use_your_nonce_account_here`, and configure RPC and gRPC before running. The nonce authority must match the signer.

```bash
export GRPC_ENDPOINT=https://your-yellowstone.example
export GRPC_AUTH_TOKEN=optional-token
cargo run --package nonce_cache
```

Do not set both a recent blockhash and a durable nonce in new code; use `SimpleBuyParams::with_durable_nonce`. Refresh nonce state immediately before construction, and never reuse consumed nonce state. A nonce extends transaction validity but does not preserve quote freshness.

See [Nonce Cache](../../docs/NONCE_CACHE.md).
