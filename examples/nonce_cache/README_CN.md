# Durable Nonce

[English](README.md)

展示如何获取 durable nonce，并附加到由 `sol-parser-sdk` gRPC 事件触发的 PumpFun 交易。

> 这是会发送真实交易的模板。运行前设置 `PRIVATE_KEY`，替换 `use_your_nonce_account_here`，并配置 RPC 和 gRPC；nonce authority 必须与签名者匹配。

```bash
export GRPC_ENDPOINT=https://your-yellowstone.example
export GRPC_AUTH_TOKEN=optional-token
cargo run --package nonce_cache
```

新代码不要同时设置 recent blockhash 和 durable nonce，应使用 `SimpleBuyParams::with_durable_nonce`。构建前刷新 nonce，已消费状态不可复用。Nonce 只延长交易有效期，不会保持报价新鲜。

参见[Nonce Cache 说明](../../docs/NONCE_CACHE_CN.md)。
