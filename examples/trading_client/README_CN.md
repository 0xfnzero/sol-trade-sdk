# TradingClient 创建方式

[English](README.md)

展示两种客户端创建模式：单钱包使用 `TradingClient::new`，多钱包通过 `TradingClient::from_infrastructure` 共享 RPC 和 SWQoS 客户端。

这是配置模板。运行前设置 `PRIVATE_KEY`、RPC 和所有已启用服务商凭证；程序会初始化网络客户端，但不会提交交易。

```bash
cargo run --package trading_client
```

客户端应在事件订阅前完成初始化，不要在交易回调中创建。
