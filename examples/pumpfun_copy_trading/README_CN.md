# PumpFun 跟单交易

[English](README.md)

通过 `sol-parser-sdk` Yellowstone gRPC 消费 PumpFun 买卖事件，把事件中的成交后协议字段映射到一次跟随买卖流程。

> 这是会发送真实交易的模板。运行前设置 `PRIVATE_KEY`，并配置 RPC、gRPC、目标过滤、金额、滑点和费用上限。

```bash
export RPC_URL=https://your-rpc.example
export GRPC_ENDPOINT=https://your-yellowstone.example
cargo run --package pumpfun_copy_trading
```

无条件复制全部事件是不安全的。必须增加钱包/mint allowlist、签名加指令索引去重、事件年龄上限、持仓隔离和最大输入/亏损控制。低延迟场景应在订阅前预热客户端和 blockhash cache。
