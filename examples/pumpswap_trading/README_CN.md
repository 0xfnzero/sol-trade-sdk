# PumpSwap 低延迟 gRPC 示例

[English](README.md)

该示例通过 `solana-streamer-sdk` 监听 PumpSwap 买卖事件，并用事件中的成交后储备和动态费率构建一次跟随买入。交易客户端和 blockhash cache 会在订阅前初始化，事件热路径不会同步查询 blockhash。

## 运行

```bash
cp .env.example .env
# 编辑 .env，然后载入当前 shell：
set -a; source .env; set +a
cargo run --release --package pumpswap_trading
```

也可以直接导出环境变量。`TARGET_MINT` 或 `TARGET_POOL` 至少设置一个；两者同时设置时必须都匹配。`MAX_EVENT_AGE_MS` 默认 1000。程序只读取环境变量，不会自行加载 `.env`。

## 交易语义

- 买入使用 `BuyAmount::WithMaxInput`，适合优先成交的跟单/狙击场景，滑点限制最大 quote 成本。
- 买入参数使用事件中的成交后储备和 LP/protocol/creator fee bps。
- `solana-streamer-sdk 0.5.0` 尚未暴露追加的 `virtual_quote_reserves` 事件字段，因此该兼容示例会在报价前读取一次 Pool 字段。解析器暴露该字段后，应直接传给 `PumpSwapParams::from_trade_with_fee_basis_points`，或维护 Pool 账户缓存，以移除热路径中的这次 RPC。
- 示例记录买前余额，只卖出确认后的余额增量；卖出前重新获取池状态和 blockhash。
- 若业务必须精确花费 quote，应改用 `BuyAmount::ExactInput`。这会启用最小输出保护，在活跃池中更容易因状态变化而失败。

生产机器人还应增加持久化签名去重、持仓状态机、SWQoS 配置和有限次数的重新报价。不要通过把 `min_out` 设为零来处理滑点错误。
