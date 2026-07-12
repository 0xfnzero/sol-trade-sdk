# 指令中间件

[English](README.md)

展示如何实现 `InstructionMiddleware`、安装到 `MiddlewareManager`，并在提交前观察或修改指令。

> 示例从 `PRIVATE_KEY` 加载钱包并调用 `client.buy`；配置完成后可能提交真实 PumpSwap 主网交易。

```bash
cargo run --package middleware_system
```

中间件必须保持 signer、账户顺序、compute budget、tip、nonce 和交易体积语义。低延迟中间件应确定性执行，不能包含阻塞 I/O。
