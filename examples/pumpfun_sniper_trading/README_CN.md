# PumpFun 狙击交易

[English](README.md)

使用 `sol-parser-sdk` Yellowstone gRPC，只选择创建者首次买入事件 `is_created_buy`。示例将 quote 储备、token program、cashback 和 mayhem 等事件字段映射到 PumpFun 参数，再执行一次买卖流程。

> 这是会发送真实交易的模板。运行前设置 `PRIVATE_KEY`、`RPC_URL`、`GRPC_ENDPOINT`、可选 `GRPC_AUTH_TOKEN`、目标策略、金额和费用。

```bash
export RPC_URL=https://your-rpc.example
export GRPC_ENDPOINT=https://your-yellowstone.example
cargo run --package pumpfun_sniper_trading
```

当前源码在事件处理中创建 `SolanaTrade` 并查询 blockhash。生产低延迟实现应按[低延迟指南](../../docs/LOW_LATENCY_BOTS_CN.md)在订阅前完成预热，同时进行签名去重并刷新延迟卖出状态。
