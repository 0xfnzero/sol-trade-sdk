# StonkFun quote routes

StonkFun's meme leg and its funding route are separate capabilities. Direct
quote↔meme trading uses `StonkFunParams` (LaunchLab curve) or
`StonkFunSwapParams` (verified graduated CPMM). A wallet with SOL additionally
needs a quote-conversion route. Pool discovery remains the application's job;
historical parser output identifies candidate pools, not fresh executable quotes.

## Explicit route API

Buy and sell are independent user transactions for sniping, copy trading and
ordinary trading. These APIs directly compose DEX instructions with the user's
accounts; they require no router contract. They do not provide arbitrage or a
combined buy+sell product flow.

### Payment and receipt assets

Use `SimpleBuyParams::stonkfun_with_token` / `SimpleSellParams::stonkfun_to_token`
with `TradeTokenType::SOL`, `USDC`, `WSOL`, or `Token(pool_quote_mint)`.
`StonkFunViaQuoteParams` is the neutral alias of the legacy `StonkFunViaSolParams`.
Legacy SOL helpers remain compatible.

| Selection | Behavior under Auto |
| --- | --- |
| Buy with SOL | Wrap the requested native SOL once; convert to quote when needed |
| Buy with WSOL | Spend existing WSOL; never charge native SOL for swap principal |
| Buy with USDC | Spend existing USDC; convert to quote when needed |
| Sell for SOL | Convert quote to WSOL and close WSOL at the end |
| Sell for WSOL | Convert quote to WSOL and keep the token account open |
| Sell for USDC | Convert quote to USDC and keep its token account open |
| Buy/sell with the pool quote | One meme swap; no conversion hop |

Transaction fees/rent still require native SOL. Existing WSOL/USDC/stock balances
must be in the account selected by the configured account strategy (`use_seed`
or standard ATA). Account creation does not move balances from other accounts.
SOL close-account unwrapping returns the entire WSOL account balance, including
any prior WSOL; use WSOL receipt if that account must remain open.
For prepared accounts, native SOL input still deposits and syncs the requested
amount even when ATA creation is disabled. WSOL input never makes that deposit.
If a custom policy disables output closure, it leaves WSOL wrapped; use Auto
for native SOL receipt, or explicitly set the WSOL close flag in the low-level API.

```rust,ignore
let via = StonkFunViaQuoteParams::curve(curve.clone(), usdc_quote_route);
let buy = SimpleBuyParams::stonkfun_with_token(
    meme, TradeTokenType::USDC, BuyAmount::ExactInput(usdc_units),
    via, blockhash, gas.clone(),
);
// A separate later sell; prepare a fresh quote→USDC path for the sell amount.
let sell = SimpleSellParams::stonkfun_to_token(
    meme, TradeTokenType::USDC, SellAmount::ExactInput(meme_units),
    fresh_sell_via, later_blockhash, later_gas,
);
// Already hold the pool's stock quote: no funding route is required.
let direct = StonkFunViaQuoteParams::curve_direct(curve);
let stock_buy = SimpleBuyParams::stonkfun_with_token(
    meme, TradeTokenType::Token(stock_quote), BuyAmount::ExactInput(stock_units),
    direct, blockhash, gas,
);
```

`graduated_direct` provides the same direct behavior for graduated CPMM pools.
SPL Token and Token-2022 quotes use their pool's token program and fee parameters.
Funding paths are validated against the chosen USDC/WSOL endpoint, not a
hardcoded WSOL pair. Buy/sell direction is explicit and never inferred from the
input mint being WSOL.

`StonkFunViaSolParams::curve(curve, route)` and
`StonkFunViaSolParams::graduated(cpmm, route)` accept a `StonkFunQuoteRoute`.
Its supported venues are CPMM, AMM v4, CLMM, Orca Whirlpool and Meteora DLMM.
The old `curve_with_cpmm` / `graduated_with_amm_v4` helpers remain available.

```rust,ignore
use sol_trade_sdk::{StonkFunQuoteHop, StonkFunQuoteRoute,
    StonkFunQuoteVenue, StonkFunViaSolParams};

// Load each pool's accounts for the actual direction and obtain fresh quotes.
// min_usdc and min_quote are caller-supplied conservative thresholds.
let route = StonkFunQuoteRoute::buy(vec![
    StonkFunQuoteHop::exact_in(
        StonkFunQuoteVenue::RaydiumClmm(sol_usdc_pool),
        wsol, usdc, min_usdc),
    StonkFunQuoteHop::exact_in(
        StonkFunQuoteVenue::MeteoraDlmm(usdc_quote_pool),
        usdc, quote, min_quote),
]);
let via = StonkFunViaSolParams::curve(curve_params, route);
// Pass via to SimpleBuyParams::stonkfun_with_sol as before.
```

`DexParamEnum::StonkFunQuoteRoute` also builds quote conversion independently.
Prepare `sell_hops` separately with `with_sell_hops`; sell quotes and tick/bin
arrays are direction-specific. The SDK does not reverse a buy path automatically.

## Amounts, fees and accounts

Hops execute in list order. `input_amount: None` spends the route's guaranteed
balance of that mint; `.with_input_amount(n)` allocates a fixed portion to a
branch. Branches may merge into the same quote. The ledger starts with only the
requested source amount, never an assumed pre-existing wallet balance. Pool
pairs, token programs, nonzero thresholds, available funds and final bounds are
validated. Cycles back to the source and spending the final output are rejected.
Intermediate-mint cycles and stranded guaranteed branch credit are also rejected.
This does not reject actual output above the threshold: that excess remains as
wallet dust after the next hop spends only the guaranteed credit.

Before building instructions, call
`route.preview(input_mint, output_mint, amount, slippage_bps, is_buy)?` to inspect
every normalized mint pair, token program, planned input amount and the final
`minimum_output_credit`. It performs the same validation as the builder without
RPC or signing; it does not refresh quotes or validate their market accuracy.

Concentrated-pool thresholds are explicit instruction min-outs. Token-2022
outputs require a current `TokenTransferFee` snapshot, including a zero-fee
snapshot; the subscription cache supplies this snapshot using the current epoch.
Instruction thresholds and route ledger credits are already recipient **net**
amounts: do not subtract the output transfer fee a second time. Refresh quotes and fee snapshots
across pool/configuration/epoch changes. CPMM/AMM v4 retain their existing SDK
quote calculations and reject caller minima above that protected quote.

The next hop spends protected credit, so excess intermediate output stays in
the wallet. ATA setup happens once per mint; bridge ATAs are kept. SOL is wrapped
once and WSOL closes only at the end if requested. Choose `TradeTransactionVersion::V1` for inline configuration and a 4,096-byte
transaction limit without ALTs. V0 has a 1,232-byte limit and permits optional
current address lookup tables; use sufficient compute budget for either version. ALT decoding now preserves
the all-zero System Program entry instead of shifting lookup indices.

## Validation and boundaries

Real mainnet simulations succeeded for Whirlpool quote-conversion directions,
SOL and pre-funded WSOL→Whirlpool→StonkFun inner buy, USDC→CLMM→StonkFun inner buy,
and SOL→DLMM→graduated CPMM buy. No transaction
was submitted. Conservative simulation thresholds verify account layouts and
execution, not quote accuracy. Offline tests cover CLMM+DLMM composition, split
and merge accounting, reverse sell, fee credit and rejected invalid paths.
The asset matrix also checks independent inner/graduated buys and sells for
SOL/USDC/WSOL, SOL-only wrapping/unwrapping, and one-swap stock quote trades under
both SPL Token and Token-2022. Live sell simulations for every asset/venue pair
are not claimed by that offline matrix.

On 2026-10-02, the USDC→CLMM→inner-buy simulation was re-run after removing
the fixture's one-array truncation. It executed successfully (`err=None`,
201,677 CU). Keep the direction-specific tick arrays loaded from current pool
state: even a small trade can cross an array boundary. This validates this
fixture's execution, not every CLMM pool or trade size.
The ViaSol simulation example and SOL-funded tests explicitly select native
SOL. The HotPathMinimal fixture prepares WSOL first, then spends that existing
balance; selecting WSOL alone never funds it from native SOL.
The full `instruction::stonkfun` regression run with `RUN_MAINNET_TESTS=1`
passed all 35 tests after these fixes, with no RPC soft-skips. The simulation
example also passed `cargo check -p stonkfun_via_sol_simulate`.

This API does not supply automatic pool discovery, best-route selection,
arbitrary private-program adapters, transfer-hook
account discovery, exact-output routing or full aggregator instruction replay.
The cache can price supported pools and select amount-appropriate arrays from
subscribed state. Low-level explicit-hop callers still supply fresh quotes. A
historical transaction can contain unrelated swaps; do not copy every observed
leg into a new trade without establishing its account and mint flow.

See the [capture audit](../../sol-parser-sdk/docs/STONKFUN_AUDIT.md) for actual
transaction signatures, sampling limits and parser capabilities.

Re-run the gated simulations without submitting transactions:

```sh
RUN_MAINNET_TESTS=1 cargo test --lib instruction::stonkfun_quote_route::tests:: -- --nocapture
```

## Parser → subscription cache → trade (no hot-path RPC)

Enable `parser-adapter` to use `PoolTradeHint::from_route_leg`. Select a successful
meme leg from `TransactionRoute`; this adapter copies only pool and mint identity.
It does not copy the source wallet, amounts, thresholds or historical reserves.

Feed Yellowstone account updates (pubkey, owner, bytes, slot, write_version) into
`SubscriptionAccountCache::update` in the background. Publish a stable snapshot
only after applying a complete batch; account freshness checks alone do not make
independent subscription updates atomic. The caller must handle forks/rollback,
account deletion and commitment by replacing the snapshot when necessary.
Supply slot and epoch from subscriptions using `CacheReadContext`; mint transfer
fees are selected from the cached schedule for that epoch, without network access.
The freshness bound applies to *every* dependency, including unchanged configs:
refresh snapshots in the background or choose an appropriate age policy. Never
relax it merely because a hot-path lookup failed.

```rust,ignore
let hint = PoolTradeHint::from_route_leg(meme_leg)?;
let context = CacheReadContext { slot, epoch, maximum_slot_age: 32 };
let request = CachedQuoteRequest {
    amount_in: my_amount, slippage_basis_points: 100,
    unix_timestamp: subscribed_clock_time, maximum_arrays: 6,
};
let prepared = cache.prepare_stonkfun_trade(
    hint, meme_mint, my_asset, &my_conversion_steps, request, context, is_buy,
)?;
let protected_net_output = prepared.minimum_amount_out;
let params = prepared.apply_to(my_wallet_params)?; // invokes without_rpc()
```

`prepare_stonkfun_trade` completes the current LaunchLab or graduated CPMM meme
leg and quotes the caller-selected conversion path. It supports SOL, WSOL, USDC
and direct stock quote endpoints for **one independent buy or one independent
sell**. Direct stock quote trades pass an empty conversion list. Its `apply_to`
keeps the caller's payer, blockhash/nonce, ALTs and account lifecycle policy;
exact-input mode stays exact-input and builders enforce the protected net minimum.

`quote_exact_in` provides integer local pricing and direction/amount-specific
execution accounts for CPMM, AMM v4, CLMM, Whirlpool and DLMM. `quote_route_exact_in`
quotes ordered serial or split/merge steps; `input_amount: None` spends that mint's
guaranteed credit. The automatic cache quoter rejects repeating a pool in the
same path because the second swap would need the post-first-swap state. Route
search and automatic topology reconstruction remain application responsibilities.

Required snapshots include pools, configs, mints and relevant vaults/arrays.
LaunchLab uses GlobalConfig/PlatformConfig fees and the actual quote mint; CPMM
subtracts protocol/fund/creator fees from reserves. AMM v4 v2 subtracts pending
PnL and reads actual swap fees. Concentrated pools select initialized arrays
using current bitmaps; missing bitmap-set arrays fail rather than being treated
as empty liquidity. CLMM supports fee-side modes, dynamic fees and boundary
orders. Whirlpool supports fixed/dynamic arrays and adaptive-fee Oracle state;
it emits supplemental arrays when more than three are needed (maximum six).
DLMM uses current dynamic fee/bin state and checks activation. All exact-input
quotes require full consumption; partial fills do not produce executable quotes.

Transfer fees come from mint schedules selected by the supplied epoch. Missing,
malformed, stale, disabled, unopened or mismatched state returns an error without
RPC fallback or default fee substitution. Unsupported Token-2022 extensions
fail explicitly. These are immutable prepared snapshots: **reprepare after any
dependent state or epoch update**, and use current Clock time. Freshness alone
does not establish consistency across independently received updates. Decode and
quote on background subscription updates when latency requires precomputation.
The legacy `from_trade` / `from_launchlab_trade` helpers remain partial APIs and
must not be used as complete adapters.

Enable the parser's explicit `EventTypeFilter::include_only` containing
`AccountRawSnapshot`. Feed `RawAccountSnapshotEvent` to
`update_from_parser_snapshot`; it preserves slot/write_version ordering and
zero-lamport closures. Include all required dependency addresses in subscription
filters. Raw snapshots are opt-in so default event consumers avoid byte copies.
See the compile-checked [offline integration example](../examples/stonkfun_cached_prepare.rs),
which reads snapshots and parsed routes from a local JSON plan:

```sh
cargo run --example stonkfun_cached_prepare --features parser-adapter -- plan.json
```

`SwapParams::without_rpc()` removes executor RPC access and rejects simulation
and synchronous confirmation. Supply blockhash/nonce and ALTs from background
state; confirm through transaction subscriptions. Configure non-RPC submit
providers separately: this method cannot change a sender's transport or custom
middleware behavior.

### Local quote verification

On 2026-10-02, six mainnet simulations verified CLMM, Whirlpool and DLMM in both
WSOL→USDC and USDC→WSOL directions. Local integer quote output exactly equaled
simulated recipient credit in all six cases. Snapshot loading and simulation
use RPC only in the test's cold verification stage; the quote/build path has no
RPC client. Reverse cases use test-only artificial funding with signature
verification disabled; no transactions were submitted. This is bounded fixture
coverage, not verification of every dynamic fee/order/Token-2022 combination.

```sh
RUN_MAINNET_TESTS=1 cargo test --lib cached_quote_mainnet -- --nocapture --test-threads=1
cargo test --lib --tests --features parser-adapter
cargo check --workspace --examples --features parser-adapter
```

Math dependencies are pinned to `orca_whirlpools_core` 2.1.1 (Orca license),
`solana-clmm-raydium` 0.3.0 (MIT/Apache-2.0), and `meteora-dlmm` 0.2.0 (MIT).
Local orchestration validates subscription layouts and bounded traversal around
these pure math dependencies. No router contract is involved.

Parser `native_token_actions` supplies system funding of identified WSOL
accounts, `SyncNative`, and close-account destination/authority in execution
order. These are instruction evidence, not new quotes or caller intent; check
`route.succeeded`. WSOL transfer alone does not imply SOL. The caller chooses
native SOL (wrap/unwrap policy) or existing WSOL (retain token balance).

Final regression on 2026-10-02: trade library 364 passed (2 ignored), devnet
submission tests intentionally ignored; parser default library/integration suite
325 passed, zero-copy library/StonkFun suite 299 passed. Both SDK example checks
and `git diff --check` passed. The six gated local-quote mainnet simulations were
actually executed again after state validation changes and all passed with exact
predicted/actual output equality. No live transaction was submitted.
