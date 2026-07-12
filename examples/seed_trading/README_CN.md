# Seed 优化交易

[English](README.md)

展示开启 seed 优化后的 PumpSwap 买卖账户派生，以及卖出前使用相同规则查询 token 账户余额。

> 这是会发送真实主网交易的模板。运行前设置 `PRIVATE_KEY` 和 `RPC_URL`，替换 mint 和 pool，并检查金额、滑点、费用及账户关闭 flag。

```bash
cargo run --package seed_trading
```

派生账户和构建交易必须使用一致的 `use_seed_optimize` 策略。示例在等待后复用 blockhash；生产代码应在卖出前刷新。
