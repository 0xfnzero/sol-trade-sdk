# sol-trade-sdk v9.0.1

Released 2026-10-22. Rust, Node.js, Python and Go trade packages share family version 9.0.1; parser packages share 0.9.0.

## Default stock pools / 默认股票代币池子

Remove five pools from the default catalog: two have captured single-transaction withdrawals reducing both vault balances by at least 50%; four have less than $500 in USDC/WSOL-side reserves at the 2026-10-22 04:25 UTC+8 snapshot (one meets both criteria). Defaults now contain 58 stock pools across 33 mints; trading adds the SOL/USDC conversion pool for 59 pools total. COPX.US is no longer a default mint. See [the synchronized catalog notes](docs/STOCK_BRIDGE_SYNC.md) for addresses and evidence.

This is a static catalog refresh, not continuous liquidity monitoring, certified executable depth, or an LP-lock guarantee. Explicit custom catalogs remain supported. No RPC or history scanning was added to parsing or trading hot paths.

中文：默认目录剔除 5 个池子，涵盖整池双边储备单笔下降至少 50% 的历史记录，以及快照中 USDC/WSOL 一侧储备不足 500 美元的池子。保留 58 个股票池、33 个 mint；交易侧加 SOL/USDC 共 59 个池子。该目录为静态快照，不保证实时深度或流动性锁定。

## Changes / 更新

- Default client Pump.fun trades to V3 with global configuration and explicit per-request V1/V2/V3 overrides. Keep the chosen version through cached preparation, factories and multi-hop bridge legs.
- Support stock-quote Pump.fun routes and instance-owned PumpSwap routing configuration. Validate settlement endpoints, coherent snapshots, fees and minimum output for each hop.
- Reject unsupported V1 direct token settlement rather than silently changing the requested currency. Unsupported V3 states fail explicitly.
- Synchronize native examples, public exports, documentation and shared stock catalog integration. Quotes/builders using prepared inputs perform no RPC.

## Migration / 升级

The default Pump.fun instruction version changes to V3. Configure V1/Legacy or V2 explicitly when required. This is separate from the Solana transaction-message version. Low-level legacy builders retain their documented behavior; shared family versions do not imply identical language APIs or identical defaults on every low-level function.

Update public struct/configuration literals for new fields and prefer constructors/defaults. V3 token quotes require explicit minimum output and supported fee/state checks. Historical catalog data is not current liquidity or a guaranteed executable route.

中文：客户端默认使用 V3，单笔显式版本优先；缓存与多跳准备保留版本选择。V1 不再把不支持的代币结算端点静默改为 SOL。升级时检查配置和公开结构体字段，示例回放不代表实时成交或生产延迟。

Rust parser-adapter pins sol-parser-sdk =0.9.1; examples pin solana-streamer-sdk =5.0.0. See docs/PUMPFUN_STOCK_ROUTES.md and examples/SIMULATION.md.
