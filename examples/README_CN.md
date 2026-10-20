# 交易示例

[English](README.md) · [文档索引](../docs/README.md)

在仓库根目录运行。根目录 `examples/*.rs` 使用
`cargo run --example 名称`；带独立 Cargo.toml 的示例目录使用
`cargo run --package 名称`，不要混用。

## 离线回放与构建

- `stock_conversion_replay`：回放 6 笔 Stonk/Pump 真实交易，逐跳提取已核验兑换池。
- `pumpswap_streaming_quotes`：默认使用保存的账户快照验证报价，不连接网络。
- `pumpswap_cached_prepare`、`stonkfun_cached_prepare`：基于缓存构建交易参数。
- `pumpswap_global_routing`：基于缓存选择和构建多跳路径。
- `pool_quote_tools`、`bridge_pool_depth`：对输入快照做报价、深度检查。
- `pumpswap_hotpath_benchmark`：离线报价路径计时。

```sh
cargo run --features parser-adapter --example stock_conversion_replay
cargo run --features parser-adapter --example pumpswap_streaming_quotes
```

以上两个入口支持 `-- --help`。股票回放支持 `--fixture FILE`，
可读取单笔 RPC 保存结果或 cases 集合；历史数据不会变成实时状态。

## 在线订阅与模拟

报价与深度工具共用 `support/account_snapshot.rs`，要求 base64 账户数据，
并在创建缓存前校验 Clock 的 owner 与快照 slot。公共输入处理放在 `support/`，
策略与协议行为保留在具名入口中；保留 `StonkFun` 与官方 IDL 命名。

- `pumpswap_streaming_quotes --source 来源 --live`：采集订阅信号，不签名、不广播。
  实时报价需要接入数据源的完整快照、追平与完整批次确认；默认离线回放可直接验证报价。
  来源可选 `parser-grpc`、`parser-shred`、`streamer-grpc`、`streamer-shred`，
  默认 `parser-grpc`。[账户状态与环境配置](../docs/STREAMING_QUOTES.md)。
- `pumpswap_global_routing_simulate`、`stonkfun_grpc_simulate`、
  `cpmm_*_simulate`，以及 `stonkfun_via_sol_simulate` package：
  需要在线环境的模拟，具体条件见 [模拟指南](SIMULATION.md)。
- `simulation_replay`：把保存的 wire 提交到当前银行模拟，明确需要 RPC；
  不会自动刷新历史报价，与离线 fixture 回放不同。
- `cpmm_creator_fee_stream`：CPMM 手续费订阅示例。

```sh
cargo run --features parser-adapter --example pumpswap_streaming_quotes -- --source parser-grpc --live
```

不要把所有示例直接批量运行当测试：在线模式需要账户配置，交易 package
配置后可能发送真实交易。

## 交易 package

运行前阅读各目录自己的 README，再配置密钥与账户。

- 入门：`simple_trading`、`trading_client`、`shared_infrastructure`。
- 事件驱动：`pumpswap_trading`、`pumpfun_copy_trading`、`pumpfun_sniper_trading`、
  `bonk_copy_trading`、`bonk_sniper_trading`。
- 协议：`pumpswap_direct_trading`、`raydium_amm_v4_trading`、
  `raydium_cpmm_trading`、`meteora_damm_v2_direct_trading`。
- 基础设施：`address_lookup`、`nonce_cache`、`middleware_system`、`seed_trading`、
  `gas_fee_strategy`、`wsol_wrapper`、`cli_trading`。

生产用法参考 [低延迟指南](../docs/LOW_LATENCY_BOTS_CN.md)。在回调里查询 RPC
的旧模板只用于展示协议参数映射；热路径应使用后台刷新状态和自己的下单金额。

## 命名与迁移

Rust 测试统一使用 Cargo；启用 `parser-adapter` 才能覆盖可选解析集成：

```sh
cargo test --lib --tests --features parser-adapter
cargo check --examples --features parser-adapter
cargo test --example pumpswap_streaming_quotes --features parser-adapter
```

新增回归测试写在 Rust 的 `tests/` 或 `#[cfg(test)]` 中。研究目录中的 Python
采集或费率分析脚本不属于 Rust 测试依赖，历史 fixture 的生成语言也不影响回放。

统一使用 `协议或领域_对象_动作` 的 snake_case 命名，保留官方协议和 IDL
方法名。不同协议的验证用例继续保留；参数和 fixture 加载放在 `support/` 复用。

- `cross_dex_conversion_candidates` → `stock_conversion_replay`。
- `pumpswap_parser_grpc` / `pumpswap_parser_shred` →
  `pumpswap_streaming_quotes --source parser-grpc|parser-shred`。
- `pumpswap_streamer_grpc` / `pumpswap_streamer_shred` →
  `pumpswap_streaming_quotes --source streamer-grpc|streamer-shred`。

四种来源都保留，只合并重复入口。专题文档的命令和 Cargo feature 配置已同步。
