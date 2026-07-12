# Bonk 跟单交易

[English](README.md)

通过 `solana-streamer-sdk` 消费 Bonk 事件，把事件状态映射为 SDK 协议参数，再执行一次跟随买卖流程。

> 这是会发送真实交易的模板。运行前设置 `PRIVATE_KEY`、RPC/stream 配置、目标策略、金额、滑点和费用上限。

```bash
cargo run --package bonk_copy_trading
```

必须增加签名/指令去重、事件年龄检查、钱包和 mint allowlist、持仓隔离及最大输入/亏损限制。延迟卖出需要最新状态和新 blockhash，不能复用触发快照。
