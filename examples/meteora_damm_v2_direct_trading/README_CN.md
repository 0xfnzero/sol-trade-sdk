# Meteora DAMM V2 直接交易

[English](README.md)

展示使用 `solana-streamer-sdk` 类型提供的池数据和 SDK 参数执行 Meteora DAMM V2 直接买卖流程。

> 这是会发送真实主网交易的模板。运行前必须设置 `PRIVATE_KEY`、`RPC_URL`，以及以原始单位表示的真实报价 `MIN_BUY_OUTPUT_AMOUNT` / `MIN_SELL_OUTPUT_AMOUNT`。

```bash
cargo run --package meteora_damm_v2_direct_trading
```

Token program、池方向、动态费率状态和 remaining accounts 必须保持最新。延迟卖出前刷新池状态和 blockhash；需要时使用兼容 Token-2022 的余额与账户处理。

`MIN_*_OUTPUT_AMOUNT` 是链上的最低输出原始数量，不是 UI 数量。应根据当前报价和滑点容忍度计算。示例会拒绝使用 `1`，因为在 partial-fill 模式下它几乎不提供价格保护。
