# Pump.fun 内盘股票报价币路由 / Stock quote routes

示例：[pumpfun_stock_route.rs](../examples/pumpfun_stock_route.rs)。

需要实际 RPC 模拟时使用 [pumpfun_simulate.rs](../examples/pumpfun_simulate.rs)，
支持 V1/V2/V3 直接买卖和 V2/V3 股票多跳。[模拟命令与钱包要求](../examples/SIMULATION.md)。

买入路径为 `SOL/WSOL/USDC → 股票报价币 → Pump.fun 内盘目标币`；卖出路径为
`目标币 → 股票报价币 → SOL/WSOL/USDC`。内盘调用官方
`buy_exact_quote_in_v3` / `sell_v3`（默认），或通过 `--version v2` 选择 V2。
目标内盘必须实际使用该股票币作为
`quote_mint`，不能把普通 SOL 内盘随意改成股票币报价。

## Official or sequential execution

`PumpRouteExecution` is independent of `PumpFunVersion`. On an explicit
`PumpFunQuoteRoute`, `PumpSwapQuoteRoute` or `StonkFunQuoteRoute`, set separate
`with_buy_execution(PumpRouteOptions { .. })` and `with_sell_execution(...)`
options; buy and sell limits use different output units.

- `Auto` (default): prefer official `multi_hop_swap` for a supported chain of
  2–3 Pump.fun curves/canonical migrated PumpSwap pools when both a fresh
  `minimum_output_amount` and current PumpSwap `GlobalConfig` recipient wallet
  (`buyback_recipient`) are supplied. Otherwise keep sequential execution.
- `Official`: require those inputs and an eligible path; return an error otherwise.
- `Sequential`: retain the existing independent swaps in one atomic transaction,
  with their per-hop minima. Raydium/Orca/Meteora conversions and splits stay here.

```rust
let route = route.with_buy_execution(PumpRouteOptions {
    execution: PumpRouteExecution::Auto,
    minimum_output_amount: Some(fresh_final_minimum),
    buyback_recipient: Some(current_buyback_recipient),
});
```

The selector reads supplied state only; it never queries RPC or retries a failed
simulation/submission with a different route. The official fee placement differs
from separate swaps: do not reuse sequential hop quotes as an official quote.
An Auto fallback fails if its protected final output is below the supplied
minimum. `preview()` describes only the sequential hop plan, not official fees
or expected fills. Existing prepared-cache/global-routing entry points retain
sequential execution unless their explicit quote route is configured; selecting
`PumpFunVersion::V3` alone does not enable official multi-hop.

Native SOL at a currency **curve** is paid/received through the wallet; a currency
**pool** uses WSOL. The selector preserves this distinction and rejects using a
WSOL-only balance as wallet SOL. It creates endpoint/buyback ATAs, without user
intermediate ATAs. Low-level `derive_pump_multi_hop_accounts` still supports
4+ hops with v0 + caller-provided ALTs; the selector is limited to 2–3.
The separate `sol-trade-router-sdk` CPI router remains an explicit alternative.

中文：默认 Auto 仅在路径可用、提供官方最终到账下限和当前回购收款人时选择官方多跳；
否则保留逐跳原子交易。强制 Official 不满足条件即报错，Sequential 固定使用旧路径。
买卖分别配置下限，不能复用逐跳报价推断官方报价。全程不在热路径查 RPC，也不会在模拟或发送失败后换路重试。

## 直接运行

```sh
cargo run --example pumpfun_stock_route
cargo run --example pumpfun_stock_route -- --asset WSOL
cargo run --example pumpfun_stock_route -- --sell --asset USDC
cargo run --example pumpfun_stock_route -- --version v2
cargo run --example pumpfun_stock_route -- --help
```

默认使用合成账户，验证构建过程，不代表链上有对应池子，也不代表交易模拟或成交成功。
不需要 RPC、私钥，不签名或广播。

## 使用当前账户

```sh
export SOLANA_RPC_URL='YOUR_RPC_URL'
cargo run --example pumpfun_stock_route -- --live --asset USDC \
  --mint TARGET_MINT --pool CPMM_POOL --amount RAW_INPUT --curve-min-out RAW_CURVE_MIN
```

`--sell` 切换卖出。`--amount` 为输入币最小单位；买入时 `--curve-min-out`
为目标币最小单位，卖出时为股票报价币最小单位。这个值必须由调用者根据当前
内盘状态与容忍滑点给出，不能直接复用历史成交金额。兑换跳使用当前 CPMM 快照报价，
`--slippage-bps` 默认 100。CPMM 必须直接连接资金币与内盘的真实报价币。

RPC 仅用于构建前加载账户、Token Program 和转账费快照。随后构建时
`SwapParams.rpc = None`；此模式仍然只构建，不模拟、不发送交易。

## SDK 接入与限制

- 使用 `PumpFunQuoteRoute`、`PumpFunQuoteHop`、`PumpFunQuoteVenue::PumpFunV3`
  和 `DexParamEnum::PumpFunQuoteRoute`，交易协议选择 `DexType::PumpFun`。
  V2 则使用 `PumpFunQuoteVenue::PumpFun`。复用共享路由引擎，不需要第三方 router 合约。
- V3 使用 17 个固定账户，买入显式编码 `partial_fill = false`。
  V3 路由的最低输出以 hop.minimum_amount_out 为准，覆盖内层参数的最低输出。
  不支持 cashback 内盘，遇到时明确报错，不自动切换版本。
- 每跳指定独立 `minimum_amount_out`。后续跳只花前一跳保证的最低到账金额，
  多收到的中间币留在钱包；任一链上最低输出条件未满足，整个交易回滚。
- 买卖路径独立准备。报价、tick/bin arrays、池子与内盘状态必须保持新鲜；
  此示例不提供自动发现池子或自动计算股票内盘报价。
- 内盘必须未毕业，PDA、目标 mint 和 quote mint 必须匹配。
  支持 SPL Token / Token-2022 的真实账户程序；目前内盘两侧均要求零转账税。
  实时快照加载会拒绝需额外处理的活动 TransferHook。
- 使用标准 ATA，保留中间币账户；仅按调用者设置解包最终 SOL。
- 示例的实时加载入口使用 CPMM。共享路由也可传入预先准备的 Raydium
  CLMM/AMM v4、Orca、DLMM 或 PumpSwap 兑换跳；这些是兑换场所，最终目标仍是
  Pump.fun 内盘。多跳过多时仍需检查交易大小、账户数与计算预算。

## V3 直接买卖

`TradeConfig` 默认选择 V3，可按客户端统一配置，互不影响：

```rust,ignore
use sol_trade_sdk::{common::TradeConfig, PumpFunVersion};

let config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
    .pumpfun_version(PumpFunVersion::V3) // 默认；也可选择 V2 或 Legacy
    .build();
```

配置作用于客户端的 `buy/sell`、`buy_simple/sell_simple` 及其 prepare 入口。
普通 `DexParamEnum::PumpFun` 请求和未指定版本的内盘路由跳继承配置。
使用共享基础设施创建的客户端也默认 V3，可调用 `.with_pumpfun_version(...)`。
`transaction_version` 是 Solana 交易消息格式，与这里的 Pump.fun 指令版本无关。

单笔显式 `PumpFunVersioned { params, version }`、`PumpFunV3(...)` 或
`PumpFunQuoteVenue::PumpFunV3` 优先于客户端默认值，不会被静默降级。
Legacy 不支持股票报价币多跳。默认 V3 不支持 cashback 内盘时会报错，
需要调用者明确选择适用的旧版本。

普通原生 SOL 请求保留已有基于内盘快照的报价流程；触及毕业边界的自动报价会被拒绝，
需提供显式 V3 最低输出。非原生报价币需通过下面的 V3 参数或多跳参数提供真实
Token Program、费用快照及输出保护，热路径不会为了补齐信息访问 RPC。

统一交易入口使用 `DexType::PumpFun` 和 `DexParamEnum::PumpFunV3`：

```rust,ignore
let v3 = PumpFunV3Params {
    curve, // 当前 PumpFunParams，含完整内盘状态
    quote_token_program, // 报价币真实 owner
    minimum_output_amount: minimum_out, // 调用者新鲜报价，已包含滑点保护
    base_transfer_fee,
    quote_transfer_fee,
};
let protocol_params = DexParamEnum::PumpFunV3(v3);
```

精确输入买入调用 `buy_exact_quote_in_v3`；在 `SwapParams` 设置
`fixed_output_amount` 时调用 `buy_v3`，此时 `input_amount` 是最大支付预算。
卖出调用 `sell_v3`，最低输出优先使用 `fixed_output_amount`，否则使用
`minimum_output_amount`。这些限制均以对应币的最小单位计，不自动套用 SOL 报价公式。

原生 SOL 内盘直接买卖的资金端使用 SOL sentinel，实际支付/收到 lamports；
不能传 WSOL ATA 余额冒充 SOL。股票报价币多跳中的 SOL/WSOL/USDC 则由兑换跳处理。
V3 多跳当前要求内盘使用非原生报价币。标准 ATA 派生不受 seed 优化开关影响。
直接使用低层 `SwapParams` / 指令构造器会绕过 `TradeConfig`：应显式传入
`PumpFunVersioned` 或 `PumpFunV3`；低层旧 `PumpFun` 参数保留兼容布局。

## English

Use `pumpfun_simulate -- --help` for actual RPC simulation with current accounts:
`--version v1|v2|v3`, `--sell` for an independent sell, and `--pool CPMM_POOL`
for a stock conversion route (V2/V3 only). Supply `SOLANA_RPC_URL`, `--keypair`,
`--mint`, `--amount` and a fresh `--curve-min-out` for V3 or token-quote routes.
Native V1/V2 direct trades instead use `--slippage-bps`. `--report NEW_PATH`
saves the unsigned wire, logs, CU and result, including execution failures.
The wallet needs real input balances; simulations do not persist state. The
keypair is not used to sign; simulation uses sigVerify=false and never broadcasts.

This example builds funding asset → stock quote → **Pump.fun bonding curve** buys
and independently prepared reverse sells. The curve must actually use that stock
mint as its quote. It defaults to official `buy_exact_quote_in_v3` / `sell_v3`
instructions; `--version v2` selects the original V2 route. Partial fills are
disabled and cashback curves are rejected by the V3 integration.

For direct V3 trades, use `DexParamEnum::PumpFunV3(PumpFunV3Params { ... })`
with a fresh explicit output minimum. Exact-output buys use `fixed_output_amount`
and treat `input_amount` as the maximum quote budget. Native SOL curves require
SOL endpoints; the stock multi-hop route requires a non-native curve quote.

`TradeConfig::builder(...).pumpfun_version(PumpFunVersion::V3)` is the default for
unversioned requests through TradingClient, including curve route legs. V2 and
Legacy are selectable; explicit per-request versions take precedence. Shared
infrastructure clients use `.with_pumpfun_version(...)`. This setting is unrelated
to the Solana transaction message version. Low-level instruction builders bypass
TradeConfig and require an explicit version to opt into V3. Native automatic quotes
that reach graduation are rejected; supply an explicit current limit instead.

Default runs use synthetic offline accounts. `--live` reads current curve, CPMM
and mint accounts using `SOLANA_RPC_URL`, then builds with no RPC access. Neither
mode signs, simulates or broadcasts. The live convenience loader requires a CPMM
pair between the funding asset and the actual curve quote; other prepared
conversion venues can be supplied through the shared route API.

Supply `--curve-min-out` from a fresh caller quote, in target-token raw units for
buys or stock-quote raw units for sells. Each later hop spends only guaranteed
minimum credit; surplus remains in the wallet. Both curve mints currently require
verified zero transfer fees, and the actual SPL Token/Token-2022 program must be
provided. Historical fills are not current quotes or execution guarantees.
