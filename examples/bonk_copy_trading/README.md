# Bonk Copy Trading

[中文](README_CN.md)

Consumes Bonk events through `solana-streamer-sdk`, maps event state to SDK protocol params, then performs one follow-up buy/sell flow.

> This is a live-transaction template. Set `PRIVATE_KEY`, RPC/stream configuration, target policy, amounts, slippage, and fee limits before running.

```bash
cargo run --package bonk_copy_trading
```

Add signature/instruction deduplication, event-age checks, wallet and mint allowlists, position isolation, and maximum input/loss limits. Delayed sells require current state and a fresh blockhash; do not reuse the trigger snapshot.
