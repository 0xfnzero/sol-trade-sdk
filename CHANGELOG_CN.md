# 版本记录

## v9.0.0 — 2026-10-22

同步 SDK、examples、版本导航及 AI 发布规则。请阅读 [发布与迁移说明](RELEASE_NOTES.md)，尤其注意事件精简、V3 默认行为和 Go /v9 模块迁移。

以下保留旧版 README 的发布与集成记录，其中版本、依赖和验证结果对应当时版本。当前用法以 [README](README_CN.md) 和专题文档为准。

## v8.0.0

共享股票目录、缓存报价与路由改进，详见 [发布与迁移说明](RELEASE_NOTES.md)。

## v7.0.0 — 全局桥接池配置与无 RPC 自动路由

初始化时通过 `TradeConfig::builder(...).pumpswap_routing(...)` 注册候选桥接池，买卖时传入 `DexParamEnum::PumpSwapAutoRoute(pool)` 即可自动选路。调用方在后台发布一致的账户／Clock 快照；选路、报价和构建只读缓存，缺失、过期或无可用路径直接返回错误，不回退 RPC。该模式拒绝同步模拟和 RPC 轮询确认；交易发送仍使用已配置的发送通道。

**一次多跳买入或一次多跳卖出，均在一笔交易内原子完成。** 例如 `SOL → USDC → TTWO → 目标币` 是一笔交易中的三个兑换步骤；任一步失败，兑换及建户状态回滚，交易手续费仍会收取。桥梁 ATA 使用幂等创建，已有正确账户安全复用。`Auto` 账户策略在选路后应用，保证首次买入也能准备 WSOL 和中间账户。

迁移：`TradeConfig`、`TradingClient` 新增 `pumpswap_routing` 字段；使用结构体字面量时补充 `pumpswap_routing: None`，或改用现有构造器／builder。`DexParamEnum` 新增 `PumpSwapAutoRoute(Pubkey)`，穷尽匹配需要处理新分支，因此本次升级主版本。普通 builder 用法默认不启用自动路由。依赖使用已发布的 parser `0.7.18` 与匹配的示例 streamer `3.0.16`，不再依赖相邻 parser 仓库路径。

验证：440 项库测试及 1 项官方报价对照通过，workspace 全目标编译通过。2026-10-19 的真实主网只读模拟覆盖 SOL 连续买入、WSOL 连续买入和一次买入后卖回 SOL，均 `err=null`；使用虚拟注资，未广播交易。每次完整买卖都是三跳；将两次买入和一次卖出合并为九跳的压力场景超出 CPI trace 限制，不代表单次多跳买卖需要拆成多笔交易。

示例：[离线全局配置](examples/pumpswap_global_routing.rs)、[真实主网模拟](examples/pumpswap_global_routing_simulate.rs)。完整配置、约束与运行命令见 [路由说明](docs/PUMPSWAP_QUOTE_ROUTING.md)。

## v6.0.2 — PumpSwap 多报价池与 creator 账户修复

支持按实际 quote mint/token program 构建 PumpSwap 买卖，补齐 creator vault 校验，修复显式手续费收款方与 mayhem 模式行为，并移除不必要的全局成交量写锁。新增缓存报价、转换路线选择和完整离线签名准备，包含 TTWO 池及 SOL→USDC→TTWO→目标币的买卖回归。

四语言共享真实 3007 失败样本，并逐字段核对业务指令。原生本地 bank 回归使用历史公共程序／账户与合成钱包状态。说明见 [PumpSwap 修复与验证](docs/PUMPSWAP_REPORTED_FAILURE.md)。

## v6.0.1 — Pump depth-2 and SpeedLanding regional updates

- 修正 Temporal 默认最低小费为 0.001 SOL，并补充 Temporal/LunarLander 阈值边界回归测试。

Adds Pump depth-2 create_v2 account chains, including curve and migrated-pool combinations. Preserves stored-depth chains when an ancestor is later whitelisted. Synchronizes SpeedLanding regional endpoints, including London and the New York domain, and compatible client routing. Account construction adds no RPC calls.

Validation includes official Pump SDK 4.1.0 instruction comparisons, signed local LiteSVM simulations and SDK regressions. Local simulation state/config is synthetic; no funded transactions were broadcast and production latency was not measured.

## v6.0.0 — Signed transaction and hot-path hardening

Adds optional decimal SOL minimum tips to SWQoS provider configuration. Providers below the configured threshold are skipped before instruction construction, signing and submission; equality remains eligible and omitted thresholds preserve existing behavior.

Breaking migration: provider tuple constructors now take a final `Option<f64>` argument, for example `SwqosConfig::Jito(api_key, region, endpoint, Some(0.0001))`. Pass `None` to retain previous behavior. `Default(String)` is unchanged.

Hardens cached preparation, durable nonce lifecycle, transaction pools and integer transfer-fee boundaries. Adds signed transaction, ALT and offline bank regression coverage.

Validation includes local CPU benchmarks and offline signed-bank scenarios. Measured hot paths use cached inputs without RPC. Benchmarks do not establish production network or transaction-landing latency. No funded mainnet transactions were broadcast.

## 早期协议集成记录

### 现有协议集成说明

v5.0.7 适配 CPMM creator-fee 协议分成升级：两个领取指令追加必需的 share PDA/config 账户，AmmConfig 解码暴露分成比例。新增 gRPC 缓存准备、领取/交换/LP 模拟示例及离线主网样本，依赖 parser 0.7.8 和 streamer 3.0.7。参见 [CPMM 迁移说明](docs/cpmm-creator-fee-share.md)。

v5.0.7 新增基于缓存的 StonkFun 路由，支持独立的 SOL/WSOL/USDC/股票买卖及直接 USDC/股票兑换。可选 `parser-adapter` 将 parser 路由线索、gRPC 快照连接到本地报价和构建，热路径无 RPC。实际验证的 V1 示例及缓存边界见 [gRPC 模拟文档](docs/STONKFUN_GRPC_EXAMPLES.md)。

本版本新增共享程序的一等交易入口：`DexType::LaunchLab`、`DexParamEnum::LaunchLab` 与 `LaunchLabParams`，并通过 `DexType::StonkFun`、`DexParamEnum::StonkFun` 与 `StonkFunParams` 提供平台专用命名。同一个 `DexType::StonkFun` 搭配 `DexParamEnum::StonkFunSwap` / `StonkFunSwapParams` 时会路由毕业后的外盘：从主网状态解析任意交易对、SPL Token/Token-2022 混合 token program、当前 AmmConfig、creator fee、transfer fee、vault 余额与两个 swap 方向。曲线买入在毕业边界还会按官方 LaunchLab SDK 反算并缩小实际输入。对只持有 SOL、未提前准备股票 quote 的钱包，可用 `DexParamEnum::StonkFunViaSol` / `StonkFunViaSolParams` 在同一笔交易内完成 `SOL ↔ quote ↔ meme` 两跳，内盘曲线与毕业外盘均支持；SOL↔quote 跳目前支持 Raydium CPMM 与 AMM v4。旧 Bonk 与 `RaydiumCpmm` 名称继续兼容，可用于直接访问底层协议。

新增 `StonkFunSolHop::Route` 显式报价路径，支持 CLMM、Whirlpool、DLMM、中间币和分流合流；买卖路径分别准备。池发现由调用方负责；订阅缓存现支持本地报价，启用 `parser-adapter` 后可通过 `prepare_stonkfun_trade` 完成独立买入或卖出的无 RPC 参数准备，示例见 `examples/stonkfun_cached_prepare.rs`，详见 [StonkFun 路由说明](docs/STONKFUN_ROUTES.md)。

PublicNode 免费 gRPC 缓存接入与主网模拟示例：[使用说明](docs/STONKFUN_GRPC_EXAMPLES.md)。

`SimpleBuyParams::stonkfun_with_token` / `SimpleSellParams::stonkfun_to_token` 支持 SOL、USDC、WSOL，以及池子自身的股票 quote。SOL 买入会包装，WSOL 买入只花已有余额；默认 Auto 策略下，卖出收 SOL 会解包，收 WSOL／USDC 则保留 token 账户。股票 quote 使用 `StonkFunViaQuoteParams::curve_direct` / `graduated_direct` 直连一跳，不需兑换路径。买入和卖出各是一笔独立交易，直接组合 DEX 指令，不依赖 router 合约。

以下真实主网回归测试分别使用当前 StonkFun reward 内盘和毕业后的 KNOTS/STONK CPMM 池，仅模拟不上链：

```bash
# 全部主网 simulate 套件（ViaSol / 直买直卖 / Raydium CPMM 等）
# 建议使用私有 RPC；公共节点在本套件下容易限流。
SOLANA_RPC_URL=... RUN_MAINNET_TESTS=1 cargo test --lib mainnet -- --nocapture --test-threads=1

# 可选：指定当前仍在内盘的 PumpFun mint
PUMPFUN_MINT=<mint> RUN_MAINNET_TESTS=1 cargo test --lib pumpfun_mainnet -- --nocapture
```

测试会自动 `Keypair::new()` 创建临时钱包，并在 `simulateTransaction(sigVerify=false)` 内从主网大户账户虚拟转入 SOL，**无需 PRIVATE_KEY，也不会上链**。共享工具在 `src/common/mainnet_sim.rs`（含 RPC 重试；瞬时限流会 soft-skip）。

覆盖：

| 套件 | 路径 |
|---|---|
| `stonkfun_via_sol_mainnet` | 外盘买、内盘买、买卖往返、HotPathMinimal、更大滑点、卖出关 WSOL |
| `stonkfun_mainnet` | 直连曲线、毕业 StonkFunSwap、往返、Bonk/LaunchLab 别名 |
| `raydium_cpmm_mainnet` | WSOL↔STONK、WSOL↔CARDS、毕业 KNOTS/STONK、exact-out（STONK+CARDS） |
| `raydium_amm_v4_mainnet` | WSOL↔USDT 买卖/往返、WSOL→USDC buy/exact-out、seed-optimize |
| `raydium_clmm_mainnet` | SOL↔USDC buy/往返、hop 后反向、SOL↔USDT、大额宽滑点 |
| `whirlpool_mainnet` | SOL→USDC buy/往返、hop 后 USDC→SOL、SOL→USDT、USDT 反向 |
| `pumpswap_mainnet` | from_mint、主池/seed 买+卖、close-WSOL、USDC 合笔、classic/tiny/seed-optimize |
| `meteora_damm_v2_mainnet` | USDC meme hop、exact-out、sell build、SOL/USDC 直买+双向 build |
| `meteora_dlmm_mainnet` | SOL→USDC buy/往返、hop 后反向、高 TVL 备用池双向 |
| `cross_dex_mainnet` | CLMM↔Whirlpool 组合、AMM→DAMM/DLMM/CLMM、三角 AMM→Whirlpool、CLMM→PumpSwap |
| `pumpfun_mainnet` | 可选 `PUMPFUN_MINT=...` 内盘买、买卖往返 |

示例：

```bash
RPC_URL=... cargo run -p stonkfun_via_sol_simulate
RPC_URL=... cargo run -p stonkfun_via_sol_simulate -- --curve
```
