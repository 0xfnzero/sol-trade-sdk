# PumpSwap 直接交易

[English](README.md)

不依赖事件流的 RPC 驱动 PumpSwap 买入/等待/卖出示例：通过 RPC 读取池状态，买入后查询钱包 token 余额，刷新池状态再卖出。

> 模板会提交真实主网交易。运行前设置 `PRIVATE_KEY` 和 `RPC_URL`，替换 mint 和 pool，并检查金额、滑点、费用策略、ATA flag 及是否关闭 WSOL 账户。

```bash
cargo run --package pumpswap_direct_trading
```

新代码优先使用 `SimpleBuyParams`，并明确选择 `BuyAmount::WithMaxInput` 或其他交易意图。延迟卖出前必须获取新 blockhash，不能复用买入前储备。

事件驱动版本见 [`pumpswap_trading`](../pumpswap_trading/README_CN.md)。
