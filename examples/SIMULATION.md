# Rust SDK 模拟交易示例

所有命令均在本仓库运行，使用 Rust 示例，不需要工作区外的 Python 测试脚本。离线测试与示例索引见 [README](README_CN.md)。以下银行模拟需要网络，不广播交易。

## gRPC → 本地报价 → 模拟

配置 `GRPC_URL`、`GRPC_TOKEN` 和可选的 `RPC_URL`。RPC 仅用于冷启动、模拟资金准备和显式模拟；报价与指令构建热路径使用订阅缓存。

```sh
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --help

# SOL → USDC；venue 也可选择 whirlpool 或 dlmm。
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- \
  --venue clmm --asset SOL --amount 100000 --timeout 15 \
  --require-pool-updates --simulation-out /tmp/clmm-simulation.json

# USDC → 股票 → meme。
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- \
  --direct-usdc --asset USDC --amount 1000 --timeout 15 \
  --simulation-out /tmp/stonk-buy.json

# meme → 股票 → USDC → SOL；换成 WSOL 可保留包装资产。
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- \
  --direct-usdc --sell --asset SOL --amount 1000000000 --timeout 15 \
  --simulation-out /tmp/stonk-sell.json
```

`--asset quote` 直接支付或接收股票，省去兑换腿；`--curve` 选择 LaunchLab 内盘。金额均为输入 mint 最小单位。默认 V1、无 ALT；V0 必须显式选择，按模板提供 ALT。只使用模板里的池，不保证模板在当前银行仍可成交。

`--require-pool-updates` 要求每个选定池收到实时更新；不传时允许冷启动状态，但仍检查陈旧预算。没有更新不能算实时订阅验收通过。`SIM_TOKEN_SOURCE` 可指定真实资金来源账户，仍检查 mint、owner、余额及依赖排除。未知、重复和无效参数在联网前报错。

模拟使用 `sigVerify=false` 和仅供模拟的资金转移前缀，不验证资金来源签名授权，不能广播。成功后检查输出账户及净到账保护，SOL 输出还检查 WSOL 关闭和租金留存。`--simulation-out` 保存原始 wire 和完整银行响应，包括执行失败；RPC 或预热失败不算模拟成功。

## 在当前银行重放原始交易

```sh
# OUTPUT.json 必须尚不存在，不覆盖历史证据。
cargo run --example simulation_replay -- INPUT.json OUTPUT.json

# 只有明确匹配银行错误才算负例通过；意外成功也报错。
cargo run --example simulation_replay -- INPUT.json OUTPUT.json \
  --expect-error ComputationalBudgetExceeded
```

输入为包含 `wire` 的模拟记录。重放使用 `replaceRecentBlockhash=true`，不刷新报价、不验证净到账、不发送交易。银行结果在判断退出状态前保存；缺失显式执行状态时拒绝判为成功。单次模拟最多等待 30 秒。

生成计算预算负例可在 `stonkfun_grpc_simulate` 命令中加入 `--compute-unit-limit 1`，预期非零退出，再用上述精确错误重放验证。传输失败不能代替银行错误。该负例可能在资金准备阶段失败，不证明业务指令已执行。

## CPMM 手续费领取前后对照

```sh
CPMM_REPORT_PATH=/tmp/cpmm-swaps.json \
  cargo run --example cpmm_swap_after_collection_simulate
CPMM_REPORT_PATH=/tmp/cpmm-lp.json \
  cargo run --example cpmm_lp_after_collection_simulate

# 选择新的目录保存每次模拟的原始 wire 和响应。
CPMM_SIMULATION_DIR=/tmp/cpmm-run-new/responses \
CPMM_REPORT_PATH=/tmp/cpmm-run-new/report.json \
  cargo run --example cpmm_swap_after_collection_simulate
```

Swap 比较 exact-in/exact-out 与两种领取权限，检查实际余额、Token-2022 费用及 Rust parser 成交日志。LP 比较存入/取出与两种领取权限，检查 token 和 LP 实际余额变化；LP 操作不是 swap，不要求产生交换事件。`cpmm_creator_fee_simulate` 也支持保存原始响应。

`CPMM_POOL`、`CPMM_PAYER`、`RPC_URL` 可覆盖模板。模拟检查显式 `value.err`、slot 和最低快照 slot。只有节点尚未到达最低 slot 时才有限重试，其它 RPC 错误和银行执行失败不重试。完整流程受超时约束，旧报告不能替代当前运行结果。

## 测试与研究工具的边界

Rust 单元测试、回归测试和模拟入口均使用 Cargo。`docs/research/` 中的 Python 费率研究脚本属于可选离线分析，包含动态费率采样，不是 Rust 测试依赖，也不能用基础费率报告替代其动态费率含义。其它语言 SDK 的测试在各自仓库运行。
