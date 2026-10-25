# sol-trade-sdk v10.0.0

Released 2026-10-26.

Add explicit Auto / Official / Sequential Pump route execution. Auto prefers eligible 2–3 hop official routes only with an independently quoted final minimum and current buyback recipient; cross-DEX paths retain sequential execution. Keep SOL curve wallet settlement distinct from pool WSOL settlement, preserve final limits on fallback, and never query RPC or retry a submitted route on the hot path. Synchronize official curve/pool buy/sell examples and current pool decoding across languages.

新增官方优先、强制官方和逐跳执行选择；官方路由需独立最终到账下限和当前回购收款人。保留跨 DEX 路径与 SOL/WSOL 结算语义，不在热路径查询 RPC 或发送失败后换路重试；同步内外盘买卖示例。

## Migration / 迁移

Rust callers constructing `StonkFunQuoteRoute` (also aliased as `PumpFunQuoteRoute` / `PumpSwapQuoteRoute`) with struct literals must initialize `buy_execution` and `sell_execution`, or use the existing constructors / `..Default::default()`. The new fields make this a major family release. Instruction-version V3 remains independent of route execution.

Go users must change imports from `github.com/0xfnzero/sol-trade-sdk-golang/v9/...` to `github.com/0xfnzero/sol-trade-sdk-golang/v10/...`, then run `go get github.com/0xfnzero/sol-trade-sdk-golang/v10@v10.0.0` and `go mod tidy`. Existing v9 tags remain available. Node.js and Python retain their package/import names.

中文：Rust 路由字面量需补齐两个执行配置字段，或改用构造器/default；Go 模块及示例统一改为 `/v10`，旧 v9 版本继续可用。

## Validation / 验证

Native-language regression suites and replay/build checks passed for the synchronized changes. Four trade implementations completed curve buy/sell and pool buy/sell unsigned mainnet simulations (16 successful simulations); these small-amount interface checks do not replace fresh production quotes. No transaction was broadcast.
