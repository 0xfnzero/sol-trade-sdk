# Raydium AMM V4 交易

[English](README.md)

通过 `solana-streamer-sdk` 消费 Raydium AMM V4 事件，构建 AMM 参数并执行一次买卖流程。

> 这是会发送真实交易的模板。运行前设置 `PRIVATE_KEY`、RPC/stream 配置、目标过滤、token 数量和费用。

```bash
cargo run --package raydium_amm_v4_trading
```

AMM 地址、market 账户、vault、token program 和储备方向必须与事件一致。低延迟场景应预热客户端和 blockhash cache，并刷新延迟卖出状态，不能复用触发快照。
