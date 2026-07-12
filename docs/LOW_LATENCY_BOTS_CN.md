# 低延迟 Bot 集成清单

## 启动阶段

- 创建并预热 `SolanaTrade`、RPC 和全部 SWQoS 客户端。
- 启动后台 blockhash cache，或准备并持续刷新 durable nonce pool。
- 对已知 mint 准备 ATA、WSOL ATA 和 ALT。
- 建立签名 + 指令索引去重，并恢复持仓状态。

## 事件热路径

```text
过滤 -> 去重 -> 事件年龄检查 -> 映射成交后状态 -> Simple*Params -> 签名 -> 提交
```

事件处理期间不要初始化客户端、同步查询 blockhash、查余额或搜索池。Shred 缺少必需参数时可以 RPC 回退，但要明确该路径不再是纯低延迟路径。

## 交易意图

| 目标 | 参数 |
|---|---|
| 固定花费，保护最小输出 | `BuyAmount::ExactInput` |
| 狙击/套利优先成交，保护最大成本 | `BuyAmount::WithMaxInput` |
| 固定买到数量，限制最大输入 | `BuyAmount::ExactOutput` |
| 卖出固定 token 数量 | `SellAmount::ExactInput` |

`WithMaxInput` 仍有滑点保护。不要通过设置 `min_out = 0` 处理滑点错误。

## 状态和费率

- 使用解析事件中的成交后 reserves。
- PumpFun 保留 quote mint、creator、creator vault、token program、cashback 和 mayhem 字段。
- PumpSwap 使用 `from_trade_with_fee_basis_points` 传入最新储备和动态费率。
- 自己的买入会改变池状态；延迟卖出前必须使用更新后的流缓存或 RPC 快照。
- durable nonce 只延长交易有效期，不会延长报价有效期。

## 6040 重报价

发生 `BuySlippageBelowMinBaseAmountOut` 时，丢弃旧交易，取得更新的储备和费率，检查报价年龄，然后在业务配置的有限次数内重建。没有新状态时，不要通过不断放大滑点重发同一报价。

参考示例：

- `fnzero-examples/pumpfun_grpc_sniper`
- `fnzero-examples/pumpfun_shredstream_sniper`
- `examples/pumpswap_trading`
