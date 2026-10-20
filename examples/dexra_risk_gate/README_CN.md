# Dexra 风控闸门

[English](README.md)

基于托管审计 API [Dexra](https://dexra.dev) 的[买入前风控闸门](../../docs/PRE_BUY_RISK_GATE_CN.md)。按照指南推荐的架构实现：后台任务轮询 API 并发布不可变快照，`check_buy` 只读取本地内存。

Dexra 的新币流（`GET /new_launches`）返回 pump.fun、PumpSwap、LetsBonk 的新发射代币，每个都带有结论 `ok`、`caution`、`avoid` 或 `unknown` 及其原因：铸币/冻结权限、流动性及其是否锁定或被撤出、创建者以往发射中有多少已归零，以及首批买家是否在发射区块买入、是否由创建者或同一钱包出资、是否为已知狙击钱包。

只有在新鲜快照中被评为 `ok` 的 mint 才允许买入。`caution`、`avoid`、`unknown`、新币流中未出现的 mint，以及超过三个轮询周期的快照都会被拒绝。同一 mint 有多个池子时（pump.fun 曲线及其 PumpSwap 池），以最差结论为准。

```bash
export DEXRA_API_KEY=...        # 免费 key: https://dexra.dev/app/keys
cargo run --package dexra_risk_gate -- <MINT> [<MINT> ...]
```

示例打印每个新发射，并在每次轮询后输出命令行中 mint 的闸门决策，不签名也不发送交易。在机器人中把同一个闸门挂到客户端上：

```rust
let client = SolanaTrade::new(Arc::new(payer), trade_config).await.with_risk_gate(gate.clone());
```

环境变量：`DEXRA_API_KEY`（必填）、`DEXRA_POLL_SECS`（默认 60）、`DEXRA_PLATFORMS`（默认 `pump_fun,pump_swap,letsbonk`）、`DEXRA_LIMIT`（默认 20，超过 20 需要付费套餐）、`DEXRA_URL`（默认 `https://api.dexra.dev`）。

每次轮询计为一次付费调用。免费 key 每天约一百次，建议约 15 分钟轮询一次；每分钟轮询需要套餐（https://dexra.dev/pricing）。没有 key 时 API 返回 `402` 及 [x402](https://x402.org) USDC 支付挑战。

时机：创建者历史在发射第一秒即可得到，首批买家分析在发射后数秒到一分钟内完成，在此之前结论通常为 `unknown`。该闸门适合在首批成交之后买入的策略，而非在创建区块内狙击。已知的 mint 可用 `POST /new_launches/batch`（`{"mints": [...]}`）获得同样的检查。
