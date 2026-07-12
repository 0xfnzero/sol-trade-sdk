# Raydium CPMM 交易

[English](README.md)

通过 `solana-streamer-sdk` 消费 Raydium CPMM 事件，将事件池数据映射为 SDK 参数，然后执行一次买入和卖出。

> 这是会发送真实交易的模板。运行前设置 `PRIVATE_KEY`、RPC/stream 配置、目标过滤、金额和费用设置。

```bash
cargo run --package raydium_cpmm_trading
```

生产 Bot 必须在订阅前初始化交易客户端和 blockhash cache，过滤并去重事件、拒绝过期事件，并在延迟卖出前刷新状态。当前代码重点展示 CPMM 参数映射。
