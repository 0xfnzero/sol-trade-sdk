# Bonk 狙击交易

[English](README.md)

使用 `solana-streamer-sdk` ShredStream 事件识别 Bonk launch 活动，并执行一次买卖流程。

> 这是会发送真实交易的模板。运行前替换 `use_your_shred_stream_url_here`，设置 `PRIVATE_KEY`、RPC、目标过滤、金额、滑点和费用。

```bash
cargo run --package bonk_sniper_trading
```

Shred 不一定包含所有日志字段。只有必需账户、储备、token program 和费率字段完整且新鲜时才能交易，否则应使用正确性优先的 RPC 回退。生产 Bot 应在订阅前预热交易客户端和 blockhash cache。
