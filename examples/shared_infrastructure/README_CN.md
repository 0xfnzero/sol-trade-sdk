# 多钱包共享基础设施

[English](README.md)

展示多个钱包专属 `TradingClient` 共享一份 `TradingInfrastructure`，降低多钱包服务的连接数、内存和初始化开销。

运行前替换 `src/main.rs` 中的 RPC、SWQoS 凭证和全部钱包占位值。示例只初始化客户端并打印共享信息，不提交交易。

```bash
cargo run --package shared_infrastructure
```

只有钱包使用相同 RPC、commitment 和提交通道时才共享基础设施；签名权限和持仓状态仍必须按钱包隔离。
