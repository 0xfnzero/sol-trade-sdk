# 简化交易 API

[English](README.md)

这是学习 `SimpleBuyParams`、`SimpleSellParams`、`BuyAmount`、`SellAmount`、账户策略、recent blockhash 和 durable nonce 的推荐入口。

示例只构造参数，买卖调用保持注释状态。用于真实集成前必须设置 `PRIVATE_KEY` 并替换协议状态占位值。

```bash
export RPC_URL=https://your-rpc.example
cargo run --package simple_trading
```

`WithMaxInput` 保护最大 quote 成本，适合优先成交；`ExactInput` 保护最小输出，在活跃市场更容易失败。`HotPathMinimal` 要求所需 token 账户均已存在，普通接入应使用 `AccountPolicy::Auto`。

参见[交易参数说明](../../docs/TRADING_PARAMETERS_CN.md)和[低延迟 Bot 指南](../../docs/LOW_LATENCY_BOTS_CN.md)。
