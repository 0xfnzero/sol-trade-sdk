# 买入前风险门（Pre-Buy Risk Gate）

[English](PRE_BUY_RISK_GATE.md) | 中文

买入前风险门允许策略在 SDK 构建或提交交易之前拒绝买入。它适合执行 mint authority、freeze authority、持仓集中度、内幕地址聚类、白名单和黑名单等本地缓存风险判断，同时避免在交易热路径中增加网络请求。

## 它的用途

`TradingClient::buy` 按以下顺序处理买入：

```text
校验请求 -> 执行 TradeRiskGate -> 构建指令 -> 签名 -> 提交
```

当 `TradeRiskGate::check_buy` 返回错误时，`buy` 会立即返回该错误。SDK 不会继续构建或提交交易，也不会产生交易签名。

风险门具有以下行为：

- 默认关闭，不配置时不改变现有交易行为；
- 同时覆盖 `buy` 和 `buy_simple`；
- 模拟买入和真实买入都会执行；
- 不作用于卖出；
- SDK 不负责获取风险数据，也不替用户维护风险缓存。

应用负责选择数据源、刷新周期、过期规则和风险判定逻辑。SDK 只提供买入提交前最后一个同步决策点。

## 推荐架构

在后台任务中查询 RPC 或第三方审计服务，生成不可变风险快照，再原子发布给风险门。交易线程只查询本地快照。

```text
RPC / 审计 API -> 后台刷新任务 -> 不可变本地风险快照
                                       |
市场事件 -> 构造买入参数 -> 风险门查询 -> 构建/签名/提交
```

不要在 `check_buy` 中直接请求 RPC 或远程审计 API。该方法是同步接口，因为它直接运行在延迟敏感的买入路径中。

## 快速接入

在应用中添加 SDK 和 `arc-swap`。`arc-swap` 允许交易线程无锁读取当前不可变快照，同时允许后台任务原子替换快照。

```toml
[dependencies]
anyhow = "1"
arc-swap = "1.7"
sol-trade-sdk = "5"
solana-sdk = "3"
```

下面的代码是示意片段，需要组合到应用的初始化函数和后台刷新函数中。首先定义缓存结果和风险门：

```rust
use arc_swap::ArcSwap;
use sol_trade_sdk::{TradeBuyParams, TradeRiskGate, TradingClient};
use solana_sdk::pubkey::Pubkey;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy)]
enum RiskVerdict {
    Allow,
    Deny,
}

type RiskSnapshot = HashMap<Pubkey, RiskVerdict>;

struct CachedRiskGate {
    snapshot: Arc<ArcSwap<RiskSnapshot>>,
    fail_closed_on_miss: bool,
}

impl TradeRiskGate for CachedRiskGate {
    fn check_buy(&self, params: &TradeBuyParams) -> anyhow::Result<()> {
        match self.snapshot.load().get(&params.mint) {
            Some(RiskVerdict::Allow) => Ok(()),
            Some(RiskVerdict::Deny) => {
                anyhow::bail!("risk gate rejected mint {}", params.mint)
            }
            None if self.fail_closed_on_miss => {
                anyhow::bail!("risk verdict missing for mint {}", params.mint)
            }
            None => Ok(()),
        }
    }
}

let risk_snapshot = Arc::new(ArcSwap::from_pointee(RiskSnapshot::new()));
let risk_gate = Arc::new(CachedRiskGate {
    snapshot: risk_snapshot.clone(),
    fail_closed_on_miss: true,
});

let client: TradingClient = client.with_risk_gate(risk_gate);
```

后台任务或事件消费者取得新数据后，整体发布新的快照。共享同一个风险门的所有客户端会自动读取新快照，不需要重新创建客户端。

```rust
let mut next_snapshot = RiskSnapshot::new();
next_snapshot.insert(safe_mint, RiskVerdict::Allow);
next_snapshot.insert(risky_mint, RiskVerdict::Deny);
risk_snapshot.store(Arc::new(next_snapshot));
```

之后按正常方式提交买入：

```rust
match client.buy_simple(buy_params).await {
    Ok(result) => {
        // 请求已通过风险门，并进入正常交易执行流程。
        handle_trade_result(result);
    }
    Err(error) => {
        // 包含风险拒绝，以及正常的参数校验/构建错误。
        handle_rejected_buy(error);
    }
}
```

示例中的 `client`、`safe_mint`、`risky_mint`、`buy_params` 和结果处理函数是应用占位符，需要替换为交易程序中的实际对象。

[`dexra_risk_gate`](../examples/dexra_risk_gate/README_CN.md) 示例是这种结构的完整闸门，其快照由托管审计 API 刷新。

## 缓存策略

风险快照应携带足够的元数据，供应用判断它是否仍然可信，例如数据源 slot、抓取时间、过期时间、服务版本和已完成的检查项目。

必须显式选择缓存缺失或过期时的行为：

| 策略 | 行为 | 适用场景 |
|---|---|---|
| Fail closed | 没有新鲜结果时拒绝买入 | 安全优先、新币或未知 mint |
| Fail open | 没有结果时继续买入 | 可用性优先，且已有其他风险控制 |
| 白名单 | 仅显式允许的 mint 可买入 | 受控策略和固定交易标的集合 |
| 黑名单 | 仅显式拒绝的 mint 被拦截 | 覆盖面较广且有独立监控的策略 |

不要把过期的 `Allow` 结果静默视为有效。后台刷新任务可以把过期项改为拒绝、删除该项以触发缺失策略，或者将整个风险门切换为数据不可用状态。

## 热路径要求

风险门关闭时，SDK 只增加一次 `Option` 判断。安装风险门后，SDK 借用该对象并调用 `check_buy`，不会克隆 gate、分配 future、加锁或 `await`。

最终延迟由风险门自身的缓存实现决定。低延迟场景应遵守：

- `check_buy` 只读取本地内存；
- 通过整体替换不可变快照更新数据，不要逐项修改共享 map；
- 日志、序列化、RPC、HTTP 和数据库操作全部放到后台刷新任务；
- 放行路径避免内存分配；
- 详细的拒绝诊断尽可能移出关键路径。

拒绝时构造 `anyhow::Error` 可能发生分配，但此时交易已被终止。高频放行路径应直接返回无分配的 `Ok(())`。

## API 说明

### `TradeRiskGate`

```rust
pub trait TradeRiskGate: Send + Sync {
    fn check_buy(&self, params: &TradeBuyParams) -> Result<(), anyhow::Error>;
}
```

`Send + Sync` 允许多个并发交易客户端共享同一个风险门。`params` 包含 mint、DEX、金额、quote token 类型、滑点、协议参数和其他买入设置，可用于策略自定义规则。

### `TradingClient::with_risk_gate`

```rust
pub fn with_risk_gate(self, risk_gate: Arc<dyn TradeRiskGate>) -> TradingClient;
```

该 builder 方法返回安装好风险门的客户端。克隆客户端时也会克隆风险门的 `Arc`，因此所有客户端副本读取同一个底层缓存。

## 它不能替代什么

风险门只是一个决策点，不是完整的交易安全系统。它不能替代：

- 滑点和 exact-input/exact-output 限制；
- 新鲜的池储备与费率数据；
- 策略需要的交易模拟；
- MEV 提交保护和优先费配置；
- 提交后的签名与持仓核对；
- 已放行 token 后续状态变化的持续监控。

完整的事件处理、状态新鲜度、blockhash、账户和提交规范见[低延迟 Bot 集成清单](LOW_LATENCY_BOTS_CN.md)。
