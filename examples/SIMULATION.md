# Rust SDK 模拟交易示例

这些入口只调用 simulateTransaction，不广播。报价与业务指令构建时 SwapParams.rpc=None，状态由冷启动及 Yellowstone gRPC 提供。模拟使用 sigVerify=false 和真实银行账户的模拟资金转移前缀；不验证资金来源签名授权。该前缀不能用于真实发送，不能将此验证视作生产钱包余额或签名验收。

## gRPC → 本地报价 → 模拟

```sh
# 在 sol-trade-sdk 仓库，预先配置 GRPC_URL / GRPC_TOKEN。
cargo build --example stonkfun_grpc_simulate --features parser-adapter

# SOL → USDC，仅验证兑换池。
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- \
  --venue clmm --asset SOL --amount 100000 --timeout 15 \
  --require-pool-updates --simulation-out /tmp/clmm-simulation.json

# USDC → 股票 → meme；买入与卖出单独构建。
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- \
  --direct-usdc --asset USDC --amount 1000 --timeout 15 \
  --simulation-out /tmp/stonk-buy.json
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- \
  --direct-usdc --sell --asset USDC --amount 1000000000 --timeout 15 \
  --simulation-out /tmp/stonk-sell.json
```

默认 V1，无 ALT；V0 需显式选择并提供覆盖所需账户的 ALT。`--asset` 还接受 SOL、WSOL、quote，是否连通取决于模板里的真实池。参数金额是输入 mint 最小单位；卖出很小的 meme 数量可能因整数舍入/费用产生零保护输出，此时明确拒绝，不能自动扩大用户传入金额。

`--require-pool-updates` 要求所有选定池都收到实时更新；不传此参数时，静止池可以保留冷加载状态，仍受陈旧预算校验。无更新不能宣称这些池已通过实时流验收。初始连接证明与账户预热分别受 `--timeout` 限制；每次模拟请求最多 30 秒。冷启动 RPC 与显式模拟是允许的准备/验证操作，不在报价与构建热路径中使用。

`--simulation-out` 保存 wincode 序列化的原始 V1 wire 与完整响应，包括执行失败；不经过 Legacy/V0 重编码。执行成功后验证返回账户数量，SPL 输出的 owner/mint/余额以及净到账保护；原生 SOL 输出同时检查 WSOL 关闭及租金留存。RPC/预热失败没有模拟证据，不能算执行成功。

保存的 `{wire,response}` 可传给各语言 parser 的 `examples/simulation_routes`，离线检查每个成交腿。Token-2022 日志没有证明扣费后的净到账时，parser 保留未知；银行返回输出账户余额与日志证据是不同验证依据。

## CPMM 实际费率与 exact-out

```sh
CPMM_REPORT_PATH=/tmp/cpmm-swaps.json \
  cargo run --example cpmm_swap_after_collection_simulate
```

该示例比较 exact-in/exact-out、creator/permissionless collection 四种组合，每种模拟有/无提取费用的独立交易，检查报价、输入/输出账户变化、Token-2022 转账费及 Rust parser 成交日志。`CPMM_POOL`、`CPMM_PAYER`、`RPC_URL` 可覆盖默认模板。它属于冷准备后本地构建与显式模拟的开发验证，不提供生产 gRPC 订阅器；账户状态变化时会重新准备。

## 工作区批量运行

```sh
# 从 Solana-SDK-Projects 根目录，先完成上面的 Rust build。
python tools/native-parity/simulate_rust_examples.py \
  --output-dir /tmp/rust-simulation --scenario clmm --scenario whirlpool --scenario dlmm

# 允许静止池保留冷加载，仍需 gRPC Clock/blockhash。
python tools/native-parity/simulate_rust_examples.py \
  --output-dir /tmp/rust-stonk --scenario usdc-buy --allow-cold-pools
```

批量脚本记录执行、银行错误、账户验证失败与未执行状态，保存日志并隐藏环境中的端点和凭据；重复运行前清除对应旧证据，避免误判。默认仅 CLMM/Whirlpool；可显式选择 DLMM、股票、SOL/WSOL 独立买卖。任何未通过场景使脚本非零退出。

2026-10-05 的真实银行执行记录在工作区 `tools/native-parity/rust_live_simulations_20261005`。模板池可能迁移、停用或无交易，公开资金账户余额也会变化；报告中未执行的项目不能算通过。

## WSOL、股票直接买卖与预算失败

```sh
# 已包装 WSOL 支付；独立卖出收 WSOL 选 --scenario wsol-sell。
python tools/native-parity/simulate_rust_examples.py \
  --output-dir /tmp/rust-wsol --scenario wsol-buy --allow-cold-pools

# 股票直接买入 meme：单个 LaunchLab 指令，不附加兑换跳。
# 设置 SIM_TOKEN_SOURCE 为该输入股票的真实有余额账户；不得是所报价池的依赖。
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- \
  --direct-usdc --asset quote --amount 1000 --simulation-out /tmp/stock-buy.json
# meme → 股票：单独卖出，股票是收款资产。
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- \
  --direct-usdc --sell --asset quote --amount 1000000000 \
  --simulation-out /tmp/stock-sell.json

# 真实银行应拒绝的负例：仅给 1 CU，不会实际成交。
python tools/native-parity/simulate_rust_examples.py \
  --output-dir /tmp/rust-negative --scenario clmm-low-cu --allow-cold-pools
```

`clmm-low-cu` 只有明确返回 ComputationalBudgetExceeded 且示例返回非零，才记为 passed_expected_execution_failure（失败处理验收通过）；传输错误、RPC 错误、其它银行错误或意外成功不能计为通过。失败 parser 实际成交字段必须为空。此负例在资金准备前缀就失败，不证明 CLMM 业务指令曾执行。

`--allow-cold-pools` 对转换池也生效：允许静止池保留当前冷加载状态，仍需 gRPC Clock/blockhash；显式要求 --require-pool-updates 时仍需每个池更新。转换池 `--venue` 只接受 SOL/WSOL/USDC，不接受会被忽略的 --sell、--curve 或 quote 资产。无效计算预算在联网前拒绝。

免费 RPC 可能对 getTokenLargestAccounts 限流或禁用；可在冷准备时指定 SIM_TOKEN_SOURCE 来避免该发现请求。示例仍验证真实来源账户的 mint、owner、余额及依赖排除，不覆盖银行余额。RPC_URL 可选择其它提供商，报价与构建不会因此加入 RPC。2026-10-06 本轮结果见工作区 tools/native-parity/rust_live_simulations_20261006/REVIEW.md。

## 在当前银行重放保存的原始交易

`simulation_replay.rs` 用于检查历史保护价、旧 tick/bin arrays 或指令是否仍可执行。只读取 `{wire,...}` 文件，保留原始 wire，以 `replaceRecentBlockhash=true` 显式模拟；不刷新报价、不确认净到账、不发送交易。输出必须是新文件，防止覆盖原证据，包括通过 symlink/hardlink 指向旧文件的路径。

```sh
# 从 Rust 仓库运行；OUTPUT.json 必须尚不存在。
cargo run --example simulation_replay -- INPUT.json OUTPUT.json

# 明确的失败验收，错误枚举名必须匹配，意外成功也返回失败。
cargo run --example simulation_replay -- INPUT.json OUTPUT.json \
  --expect-error ComputationalBudgetExceeded
```

所有有银行执行结果的响应（包括失败）在判断退出状态前保存，可交给 parser 的 simulation_routes。RPC/HTTP 失败没有银行结果，不算执行成功。一次模拟最多 30 秒。`execution_succeeded` 仅表示这笔历史指令在当前银行执行成功；即使如此，仍是 `quote_refreshed=false`、`net_credit_verified=false`，不能当成当前状态的重新报价和净到账证明。

StonkFun gRPC 示例现在也拒绝未知、重复选项及多余位置参数，例如 `--ammount` 或同时指定两个 `--asset`；这些输入在联网前报错，不能静默使用默认金额/资产。

## meme → 股票 → USDC → WSOL/SOL 独立卖出

Stock/USDC 模板也可以显式追加已有 WSOL/USDC CLMM 池，按卖出方向依次准备股票→USDC→WSOL，或按买入方向准备逆序路径。只使用模板中已知池，quote 直接交易仍为零兑换腿；不会访问聚合器或构建买卖闭环。

```sh
python tools/native-parity/simulate_rust_examples.py \
  --output-dir /tmp/nvda-receive-wsol --scenario nvda-wsol-sell --allow-cold-pools
python tools/native-parity/simulate_rust_examples.py \
  --output-dir /tmp/nvda-receive-sol --scenario nvda-sol-sell --allow-cold-pools
```

本轮这两个新模板均通过银行模拟，parser 各识别三个连续交易腿；SOL 收款额包含正确的 WSOL 关闭和租金留存校验。它们使用 NVDA 股票模板，不能代替旧 CARDS 模板的验收。保存证据和解析结果在工作区 `tools/native-parity/rust_replay_simulations_20261006`。

两个 Rust 示例现在先检查原始 RPC result 的显式 `value.err` 与 slot，再判断成功。Rust 可选字段直接反序列化会把缺失 err 和 err:null 都变成 None，不能据此报告执行成功。重放示例保留原始响应，并使用 create_new 输出文件，现有路径、symlink/hardlink 不会覆盖原证据。未知 simulation_only_funding 字段保留 null。

## CPMM 手续费领取前后的 swap／LP 对照

以下示例冷加载当前账户，在本地构造指令，再分别模拟基线交易和插入手续费领取的交易；比较实际 token 余额、费用计数和 parser 事件，不把报价当成成交结果。

```sh
# 在 Rust 仓库运行；RPC_URL 可选，默认 mainnet RPC。
CPMM_REPORT_PATH=/tmp/cpmm-swaps.json cargo run --example cpmm_swap_after_collection_simulate
CPMM_REPORT_PATH=/tmp/cpmm-lp.json cargo run --example cpmm_lp_after_collection_simulate
```

Swap 覆盖 exact-in／exact-out 与两种领取权限，共四组、八次银行模拟。LP 覆盖存入／取出与两种领取权限；每组还先模拟准备资金和 LP，取出组还增加一次 LP 准备，因此四组共十四次银行模拟。LP 指令按 IDL 构造，仅用于示例验证，并非 SDK 新增公开 LP API。

模拟使用真实账户余额及 `sigVerify=false` 资金前缀，仅验证执行逻辑，不验证资金账户签名授权，不能发送这些交易。账户稳定性比较包含 executable、rent_epoch；返回的账户必须完整可解码，显式 null 才视为不存在。余额从完整 SPL／Token-2022 状态读取，截断或未初始化账户会报错。

2026-10-06 本次修复及最终代码的银行模拟证据见工作区 `tools/native-parity/rust_cpmm_account_review_20261006/REVIEW.md`。公开节点出错或限流不算通过。

## 保留 CPMM 原始模拟响应并解析

CPMM 三个模拟示例现在统一先校验原始响应的显式 `value.err`、合法 slot 和最小 snapshot slot，模拟请求最多等待 30 秒。缺少执行状态会报错，不会视为成功。设置 `CPMM_SIMULATION_DIR` 可保存每次模拟的原始 wire／响应，包括预期失败；目录内生成 `simulation-000.json` 等文件，使用 create_new，已有证据不会被覆盖。

```sh
# 选择全新的证据目录；在 Rust 仓库运行。
CPMM_SIMULATION_DIR=/tmp/cpmm-run-new/responses \
CPMM_REPORT_PATH=/tmp/cpmm-run-new/report.json \
  cargo run --example cpmm_swap_after_collection_simulate

# 在 sol-parser-sdk-python 仓库运行；无需网络。
python examples/simulation_routes.py /tmp/cpmm-run-new/responses/simulation-000.json
```

也可对 `cpmm_creator_fee_simulate` 和 `cpmm_lp_after_collection_simulate` 使用相同变量。响应保留 innerInstructions，方便 parser 检查实际交换与失败意图；手续费领取／LP 操作本身不是 swap，不要求它们产生交换路线。保存结果仍为未验证签名的模拟交易，不能广播。2026-10-06 本轮证据见 `tools/native-parity/rust_cpmm_strict_simulations_20261006`。

节点返回 `-32016`（Minimum context slot has not been reached）时，CPMM 模拟最多尝试四次，间隔 500ms，整个请求流程仍受 30 秒上限约束。重试保留相同 wire/config/minContextSlot；其它 RPC 错误和实际银行执行失败不重试。

本轮最终实测 31 次银行模拟：24 次成功、7 次按精确 InstructionError 位置与 Custom 错误码验证的负例。Python simulation_routes 解析其中 21 份；另 10 份 LP 响应包含暂不支持的 parsed mintTo/burn CPI，会明确报错。LP 银行执行及 Rust 账户校验已通过，但不能据此宣称 Python LP 模拟路线适配已验收。


## LP 实际余额变化与三语言模拟解析

LP 示例现在额外输出 token0_delta、token1_delta、lp_delta，均为最小单位的有符号十进制字符串，从 prepared 与实际模拟账户字节计算，并与报告互相校验。存入通常 token delta 为负、LP delta 为正；取出方向相反。不能使用资金准备之前的空钱包作基准，也不能用报价替代余额变化。

```sh
# 在 Rust 仓库运行，使用全新目录保存每次模拟。
CPMM_SIMULATION_DIR=/tmp/cpmm-lp-new/responses \
CPMM_REPORT_PATH=/tmp/cpmm-lp-new/report.json \
  cargo run --example cpmm_lp_after_collection_simulate

# 在对应 parser 仓库运行，对同一份原始响应离线解析。
go run ./examples/simulation_routes /tmp/cpmm-lp-new/responses/simulation-001.json
npx tsx examples/simulation_routes.ts /tmp/cpmm-lp-new/responses/simulation-001.json
python examples/simulation_routes.py /tmp/cpmm-lp-new/responses/simulation-001.json
```

本轮已修复上述三语言 LP mintTo/burn 模拟 CPI 适配缺口，并覆盖 checked 变体。LP 铸造／销毁保留为指令，不被当作 swap 或转账；LP 操作仍可显示为未知顶层调用，不宣称新增 LP 事件 API。最新银行证据与三语言解析对照见 tools/native-parity/rust_lp_supply_review_20261006/REVIEW.md。
