# Rust SDK 模拟交易示例

所有命令均在本仓库运行，使用 Rust 示例，不需要工作区外的 Python 测试脚本。离线测试与示例索引见 [README](README_CN.md)。以下银行模拟需要网络，不广播交易。

## Pump.fun V1 / V2 / V3 内盘模拟

统一入口为 `pumpfun_simulate`。默认 V3；`--version v1` 对应 Legacy。
读取当前 mint/内盘和费用账户后构建指令，再调用 `simulateTransaction`。
不传 `--pool` 为直接买卖，传 CPMM 池为股票报价币多跳。V1 不支持股票币多跳。

```sh
cargo run --example pumpfun_simulate -- --help
export SOLANA_RPC_URL='YOUR_RPC_URL'

# V1/V2 原生 SOL 内盘买入。替换 mint、金额和本地钱包路径；v1 可替换为 v2。
cargo run --example pumpfun_simulate -- --version v1 \
  --mint CURRENT_ACTIVE_MINT --amount RAW_SOL_INPUT --keypair /path/to/wallet.json \
  --slippage-bps 100 --report /tmp/pump-v1-buy.json

# V2 原生 SOL 内盘独立卖出。钱包必须真实持有输入币。
cargo run --example pumpfun_simulate -- --version v2 --sell \
  --mint CURRENT_ACTIVE_MINT --amount RAW_TOKEN_INPUT --keypair /path/to/wallet.json \
  --slippage-bps 100 --report /tmp/pump-v2-sell.json

# V3 直接买入；--sell 切换独立卖出。
cargo run --example pumpfun_simulate -- --version v3 \
  --mint CURRENT_ACTIVE_MINT --amount RAW_INPUT --curve-min-out RAW_MIN_OUTPUT \
  --keypair /path/to/wallet.json --report /tmp/pump-v3-buy.json

# V2/V3 股票币多跳；买入 USDC → 股票币 → 内盘币。
cargo run --example pumpfun_simulate -- --version v3 --asset USDC \
  --mint CURRENT_STOCK_QUOTED_MINT --pool CURRENT_CPMM_POOL \
  --amount RAW_USDC_INPUT --curve-min-out RAW_TARGET_MIN \
  --keypair /path/to/wallet.json --report /tmp/pump-stock-buy.json

# 卖出 内盘币 → 股票币 → SOL。CPMM 必须连接股票币与 WSOL。
cargo run --example pumpfun_simulate -- --version v2 --sell --asset SOL \
  --mint CURRENT_STOCK_QUOTED_MINT --pool CURRENT_CPMM_POOL \
  --amount RAW_TOKEN_INPUT --curve-min-out RAW_STOCK_MIN \
  --keypair /path/to/wallet.json --report /tmp/pump-stock-sell.json
```

以上大写参数是占位符，必须替换为当前有效账户与独立报价。不要用历史签名中的
金额冒充当前报价；已毕业的内盘会报错。各版本都支持独立买/卖：增加或移除
`--sell` 即可，金额和最低输出也必须换成相应币的单位。

V1/V2 原生 SOL 直接交易使用 SDK 快照报价和 `--slippage-bps`，打印实际编码的
输入/最低输出；该模式拒绝 `--curve-min-out`，防止参数被忽略。V2 非原生报价币
直接交易、V3 直接交易和所有股票多跳均要求显式 `--curve-min-out`。
买入时它是目标币最低数量，卖出时是内盘报价币最低数量。多跳中
`--slippage-bps` 只控制 CPMM 兑换跳；`--asset` 可选 SOL、WSOL、USDC。

钱包密钥文件仅供 SDK 获取正确钱包身份与派生 ATA，交易签名保持空值，
使用 `sigVerify=false`；不签名、不广播。钱包仍必须在当前链上有足够 SOL
支付模拟中的费用/租金，以及所需输入代币。模拟买入不会持久化余额，不能据此
直接进行下一次独立卖出模拟。V3 不接受 cashback 内盘，当前集成要求零转账税。

报告包括请求版本、方向、账户/指令数据、未签名 wire、模拟 slot、CU、日志和
`response.value.err`；银行执行失败也会保存报告并返回非零退出码。
预检查、构建或 RPC 传输失败不会生成成功报告。报告路径必须不存在。
当前入口使用 **Solana V1 交易消息**，与 Pump.fun 指令 V1/V2/V3 是不同概念；
RPC 必须支持该消息格式。该模拟不验证签名授权，也不等价于真实成交。

`pumpfun_stock_route` 仍然只构建，默认离线；`--live` 只加载账户，不会模拟。
两个入口共享 `support/pumpfun_example.rs`，避免两套路由组装逻辑。

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
