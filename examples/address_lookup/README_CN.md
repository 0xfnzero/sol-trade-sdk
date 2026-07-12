# 地址查找表（ALT）

[English](README.md)

展示如何获取 Address Lookup Table、附加到 PumpFun 交易，并在收到 `sol-parser-sdk` gRPC 事件后提交。

> 这是会发送真实交易的模板。运行前设置 `PRIVATE_KEY`，替换 `use_your_lookup_table_key_here`，并配置 RPC 和 gRPC。

```bash
export GRPC_ENDPOINT=https://your-yellowstone.example
export GRPC_AUTH_TOKEN=optional-token
cargo run --package address_lookup
```

ALT 只能缩小消息体积，不能让过期池状态重新有效。已知 ALT 应在事件热路径前获取并缓存。当前示例在事件后初始化交易客户端，因此只用于展示 API，不代表推荐低延迟结构。

参见[地址查找表说明](../../docs/ADDRESS_LOOKUP_TABLE_CN.md)。
