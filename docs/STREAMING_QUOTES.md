# Cached quotes with parser or streamer signals

The SDK separates pool identities, account state, quotes and transaction signals.
`PumpSwapRoutingConfig::default()` supplies screened bridge identities, not live
prices. `PumpSwapRoutingFeed` ingests background account updates and publishes
immutable caches. `PumpSwapRouting::prepare` searches and prepares one atomic
multi-hop buy or sell without RPC. The example strategy only prints quotes; it
has no signer or broadcast path. In an application, use the returned
`PreparedPumpSwapTrade::into_buy_params` / `into_sell_params` with a fresh
blockhash and gas policy, then the existing `TradingClient` preparation/build
APIs. The prepared route preserves amounts and min-out protections; do not
reconstruct them from event amounts. See `pumpswap_global_routing` and
`pumpswap_global_routing_simulate` for instruction building and read-only
simulation respectively.

## One entry point, four transaction sources

All sources default to the same captured mainnet-account replay. This verifies the
common quote path, not connectivity or live behavior of a transport.

```sh
cargo run --features parser-adapter --example pumpswap_streaming_quotes -- --source parser-grpc
cargo run --features parser-adapter --example pumpswap_streaming_quotes -- --source parser-shred
cargo run --features parser-adapter --example pumpswap_streaming_quotes -- --source streamer-grpc
cargo run --features parser-adapter --example pumpswap_streaming_quotes -- --source streamer-shred
cargo run --example pool_quote_tools
```

Add `--live` after the source argument to subscribe. `--source` selects the transaction source:
parser Yellowstone, parser ShredStream, streamer Yellowstone, or streamer
ShredStream. All use a **separate parser raw-account gRPC connection** for state;
streamer itself builds on parser, and decoded trade events are not account state.
Shred modes therefore require both endpoints. Streamer is only a dev dependency,
not a new mandatory SDK runtime dependency.

```sh
export GRPC_ENDPOINT=https://YOUR_YELLOWSTONE_ENDPOINT
export GRPC_AUTH_TOKEN=YOUR_TOKEN
export SHRED_ENDPOINT=http://YOUR_SHRED_ENDPOINT:10800  # shred examples only
export TARGET_POOL=YOUR_PUMPSWAP_POOL
export TARGET_MINT=YOUR_TARGET_MINT
export BUY_AMOUNT_RAW=1000000  # raw payment units (WSOL by default)
export SELL_AMOUNT_RAW=100000 # raw TARGET_MINT units; independent sell size
export ACCOUNT_KEYS=POOL,MINTS,VAULTS,CONFIGS,TICK_OR_BIN_ARRAYS,BITMAPS,ORACLES
export BRIDGE_SLIPPAGE_BPS=100
export BRIDGE_IMPACT_BPS=100
export TARGET_SLIPPAGE_BPS=100
cargo run --features parser-adapter --example pumpswap_streaming_quotes -- --source parser-grpc --live
```

Replace all placeholders. `ACCOUNT_KEYS` is a comma-separated list of actual
addresses, covering the target PumpSwap market and the conversion paths you want
available. Include each protocol's configuration/fee accounts and mint accounts;
Clock is added automatically. `ASSET_MINT` can override WSOL (including native
SOL). Accounts/arrays that become relevant after price movement must be added by
your background discovery/subscription service. Filters are explicit, so an
unknown or missing account never causes RPC in a quote.

`GRPC_URL` / `GRPC_TOKEN` are accepted aliases, matching the parser examples;
`GRPC_ENDPOINT` / `GRPC_AUTH_TOKEN` take precedence. Blank endpoints are rejected
and blank authentication tokens are omitted. Signal deduplication uses the raw
signature without Base58 allocation. Queue discard and processing loops are
bounded so continuous traffic cannot indefinitely postpone cancellation checks.
Closing the stop channel also exits the worker loops and stops their subscriptions.

This example does **not** bootstrap accounts: unchanged accounts may never appear
on an incremental stream. For production, obtain a validated complete startup
snapshot in a cold/background path, catch up updates from the same provider/fork,
and call `feed.confirm_bootstrap(slot)?` at the coherent snapshot slot before
publishing. Continue publishing only at verified complete batch boundaries.
An empty consumer queue or a recent Clock alone does not prove that all writes
in a transaction have arrived. The generic live example therefore collects
updates without publishing executable snapshots; connect your provider's
bootstrap and batch-completion contract to enable live quoting.
Keep this lifecycle outside transaction callbacks. Without
sufficient fresh state the example logs `quote skipped`, rather than constructing
an unsafe order. This is a reference integration, not a turnkey copy-trading bot.

## SDK configuration and protection

```rust,no_run
use sol_trade_sdk::{PumpSwapRouting, PumpSwapRoutingConfig, PumpSwapRoutingFeed};
# fn main() -> anyhow::Result<()> {
let mut config = PumpSwapRoutingConfig::default();
config.bridge_slippage_basis_points = 100; // 1% execution tolerance per bridge hop
config.maximum_bridge_price_impact_basis_points = 100; // 1% size impact per hop
let routing = PumpSwapRouting::new(config)?;
let mut feed = PumpSwapRoutingFeed::default();
// Background: feed.update(pubkey, cached_account)?;
// Or parser-adapter: feed.update_from_parser_snapshot(raw_event)?;
// After a complete startup snapshot and ordered catch-up on the same fork:
// feed.confirm_bootstrap(coherent_slot)?;
// After validating a complete account batch and transport continuity:
let latest_slot = 123; // Replace with a recent trusted block/slot watermark.
feed.publish_if_advanced(&routing, latest_slot, 32);
// On reconnect, loss, filter replacement or fork rollback:
feed.reset(&routing);
# Ok(())
# }
```

Bootstrap confirmation lets unchanged mint/config accounts remain valid as the
continuous subscription advances; their actual slot/write_version are preserved.
Without confirmation, conservative per-account last-write age checks remain in
effect. New same-slot write versions can publish a new snapshot; identical or
stale replay updates cannot refresh its wall-clock timeout. Reconfirm bootstrap
after reconnect, event loss, filter replacement or fork rollback.

Both bridge limits default to 100 bps and accept 0..9999. They apply to every
conversion leg, even when a conversion venue is itself PumpSwap. The selected
final/initial PumpSwap market uses the trade request's separate slippage setting.
Each hop carries an on-chain min-out; the next spends only the guaranteed output.
A failed hop rolls back the whole transaction. Favorable intermediate leftovers
may remain in user ATAs. Per-hop tolerances compound across multiple hops: two
1% tolerances are not a total-route 1% guarantee.

Slippage tolerance limits movement after quoting. Price impact limits the current
order's execution-price deterioration versus a 1/100-size cached probe in the same
direction. Fees are included in both probes. This reference is not an external
oracle or NAV; low-decimal rounding and dynamic fee changes can affect it. If a
small probe cannot be quoted, admission fails closed. Price-impact comparisons use
integer arithmetic; displayed bps are informational floats. The actual order size,
not a fixed $1,000 order, is checked on every preparation.

The feed publishes new account revisions covered by Clock, including same-slot
writes. It preserves actual write slots and never renews freshness from repeated
identical updates. It cannot establish fork identity or detect transport loss by itself.
The live examples check the account transport's continuity counters, discard
queued/cache data on gaps, require recent block progress, and expire snapshots.
Always preserve these checks when adapting the example. Signal queues are bounded;
old signals are skipped, and the demo quotes at most once per recent transaction,
pool and direction. This is conservative signal suppression, not a universal
transaction deduplication API.

## Quote tools

`SubscriptionAccountCache::quote_pool_exact_in(pool, input_mint, output_mint,
request, context)` automatically selects the cached engine for PumpSwap,
Raydium AMM v4, CPMM, CLMM, Orca Whirlpool, or Meteora DLMM. It returns estimated
net output, protected output, slot/epoch and executable venue parameters.
`quote_exact_in(hint, ...)` is the equivalent typed-hint API;
`hint.reversed()` prepares an independent opposite-direction quote.
`quote_exact_in_with_impact(..., max_bps)` adds order-size protection.
`CachedQuoteRequest::new(raw_amount, chain_timestamp)` defaults to 1% tolerance
and six arrays; `.with_slippage_basis_points(bps)` changes tolerance.

```sh
cargo run --example pool_quote_tools -- /path/to/pool-snapshot.json 10000000
cargo run --example pool_quote_tools -- /path/to/pool-snapshot.json 100000 reverse
```

Snapshot format is the `bridge_pool_depth` format: one coherent `slot`, pool
`hints` and RPC-format `accounts`, including Clock. The example prints both quote
and impact result. Accounts must use base64 encoding; Clock's owner and slot must
match the coherent snapshot. A mismatched snapshot is rejected before quoting.
Its opposite-direction quote uses the same snapshot; it is
not a sequential buy/sell simulation that mutates reserves. An unsupported venue
returns an explicit error; this API does not imply cache quoting for every other
DEX supported by the SDK's instruction builders. Existing protocol-specific
helpers remain under `utils::calc`; use their typed inputs when needed.

## Stock monitoring and catalog sharing

Both SDKs use the same catalog of 34 verified stock/ETF mints and all 63
qualified pools (46 USDC pairs and 17 WSOL pairs). Trade defaults additionally
include the WSOL/USDC bridge, for 64 pools total. Historical fee preferences are
shared metadata; actual route selection uses fresh cached protected output.

Search starts from the requested payment asset. USDC input can take
`USDC -> stock -> target` directly; it may select `USDC -> WSOL -> stock -> target`
if that path produces higher protected output within the configured limits.
Equal protected output prefers fewer hops. Paths never revisit a mint, and USDC
input is never implicitly treated as WSOL. Default search budget is 512 nodes;
exhaustion fails explicitly rather than returning an incompletely searched route.

Pass `STOCK_CATALOG_JSON=/path/to/stock_token_catalog.json` to this example to merge
all parser-catalog pool identities. The example accepts the bundled catalog's
base58-string JSON format. Duplicate addresses and reversed pairs collapse;
conflicts fail atomically. The 128-pool limit leaves 64 slots beyond the defaults.
A typed application can map `StockPool { pool, mint, quote_mint, .. }` directly to
`PoolTradeHint { pool, input_mint: quote_mint, output_mint: mint }` and call
`extend_pools`; no parser-specific coupling is needed in the core registry.

Stock `ReportedSwapAmounts` prices can trigger strategy decisions or dashboards;
they are never substituted for tick/bin state, fee configuration or executable
quotes. Stock-price subscriptions are opt-in and need a dedicated parser client.
The stock module is in the local parser 0.7.19 work; these transport examples
compile against the currently pinned parser 0.7.18 / streamer 3.0.16 without
requiring that unpublished module.
