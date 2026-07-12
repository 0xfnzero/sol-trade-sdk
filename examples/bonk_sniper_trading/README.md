# Bonk Sniper Trading

[中文](README_CN.md)

Uses `solana-streamer-sdk` ShredStream events to detect Bonk launch activity and execute one buy/sell flow.

> This is a live-transaction template. Replace `use_your_shred_stream_url_here`, set `PRIVATE_KEY`, RPC, target filters, amounts, slippage, and fees before running.

```bash
cargo run --package bonk_sniper_trading
```

Shreds may not contain every log-derived field. Only trade when every required account, reserve, token-program, and fee field is present and current; otherwise use a correctness-first RPC fallback. Prewarm the trading client and blockhash cache before subscription in a production bot.
