# Mainnet bridge defaults

`PumpSwapRoutingConfig::default()` provides **64 offline pool identities**:
63 stock/ETF conversion pools covering **34 verified mints**, plus WSOL/USDC.
The 2026-10-20 expanded catalog contains:

- Backpack: ABNB, AMD, BABA, CHWY, COPX, DJT, HIMS, HOOD, INTC, MU, NFLX, NKE,
  RACE, RDDT, SPCX, SPHR, TSM, TTWO, UVXY.
- xStocks: AAPLx, AVGOx, GLDx, GMEx, GOOGLx, HOODx, INTCx, METAx, MSFTx, MSTRx, NVDAx,
  QQQx, SPCXx, SPYx, TSLAx.

Stocks and ETFs are both included; UVXY, GLDx, QQQx and SPYx are fund products.
The same underlying issued by different providers remains a distinct mint.
For each official mint, mark a historical preference by lowest snapshot-start fee among
candidates that pass depth admission; break fee ties by deeper liquid-side reserve.
Dynamic fees are evaluated at the captured Clock and active tick/bin, not treated
as a fixed fee or a whole-order average. TTWO's previous USDC DLMM pool is now
its lowest-fee qualifying pool; other qualifying pools remain available.
This is a bounded sample, not an exhaustive market registry or popularity ranking.
CARDS is excluded because it is not a verified stock/ETF token.

The defaults now read the same packaged `src/stock_token_catalog.json` as
sol-parser-sdk: 34 mints and 63 qualified pools, including historical fee/tax
snapshots. Both SDKs include all qualified stock pools; trading additionally
includes the WSOL/USDC bridge. Historical preference combines
known snapshot-start swap and nominal transfer fees; unknown fees or active
unpriced hooks sort last. Transfer caps and order size can alter actual cost.
The offline Rust `sol-parser-sdk` example `stock_catalog_sync` updates both
copies and supports `--check` for parity. Neither SDK requires the other checkout
at runtime. Live cached quoting and order-size checks remain authoritative.

The registry configures PumpSwap automatic routing; it does not enable automatic
routing for StonkFun trade entry points. Missing default-market caches do not
cause network discovery or RPC fallback.

## Cross-DEX conversion candidates

With `parser-adapter`, `PoolTradeHint::from_transaction_route(&route, &approved_pools)`
extracts pool-specific identity clues across all supported DEXes. It rejects
failed transactions and conflicting mint pairs, deduplicates repeated pools,
and ignores unapproved pools. Pass verified conversion pool addresses, not every
pool in the transaction: target pools and conversion pools have distinct roles.
Approval does not replace cached owner/mint validation or fresh quoting.

```sh
cargo run --features parser-adapter --example stock_conversion_replay
cargo test --features parser-adapter --test cross_dex_conversion_candidates
```

The example replays six signed mainnet transactions, selects conversion pools
from the shared catalog and merges hints using `extend_pools`. `--fixture FILE`
accepts another saved RPC transaction or a `cases` capture. It performs no RPC,
signing or sending and never copies historical trade amounts into a new order.
Pool eligibility is independent of the target's launch platform. Stonk explicit
quote routes already support multiple conversion DEXes; this does not introduce
automatic Stonk route search. Unknown Shred CPI routes remain unresolved.

```rust,no_run
use std::sync::Arc;
use sol_trade_sdk::{PumpSwapRouting, PumpSwapRoutingConfig};
# fn main() -> anyhow::Result<()> {
let mut markets = PumpSwapRoutingConfig::default();
// Replace with your supported custom pool identities.
let custom_pools = markets.conversion_pools.clone();
markets.extend_pools(custom_pools)?;
let routing = Arc::new(PumpSwapRouting::new(markets)?);
// TradeConfig::builder(...).pumpswap_routing(routing.clone()).build()
// Use routing.conversion_pools() to set up background subscriptions.
// Publish coherent cache + Clock/slot snapshots before preparing trades.
# Ok(())
# }
```

`extend_pools` deduplicates by pool address, preserves the first occurrence, and
normalizes SOL to WSOL. Reversed pairs are equivalent. Distinct pool addresses
for one mint pair remain alternatives. Conflicting identities or more than
128 unique pools fail atomically. The 64 defaults leave room for 64 additional
unique custom pools. `PumpSwapRouting::new` also validates direct vector edits;
`PumpSwapRoutingConfig::new(pools)` remains the custom-only option.

## Official identities and transaction coverage

Mint addresses are matched exactly against independently fetched official sources:

- [Backpack assets API](https://api.backpack.exchange/api/v1/assets): 1,171 nonempty
  Solana stock/ETF contract addresses in the captured response.
- [xStocks product catalog](https://xstocks.com/products): 1,353 Solana addresses
  in the captured product data.

Names, symbols, prefixes and on-chain metadata alone are not admission evidence.
[Preserved issuer records and decoded pool evidence](research/bridge-defaults-20261020-expanded.json)
contain exact contract addresses, official source records, account bytes, capture
slots and depth results. This confirms official mint identity, not an independent
audit of the issuer's backing or legal rights.

The expanded scan sampled **1,000 recent signatures from each of four sources**:
two Stonk platform accounts, the Pump program and PumpSwap. It also sampled
40 official mint addresses (26 Backpack targets, up to 25 signatures each;
14 xStocks targets, up to 10 each), and reused the prior smaller sample.
Stonk platform accounts are not program IDs. Failed transactions are excluded.

Across these samples we captured **2,218 successful transactions**, matched
**55 verified stock/ETF mints**, and decoded **69 supported USDC/WSOL bridge
candidates**. Twelve successful signatures from the broad sample were unavailable
after RPC retries. Targeted mint samples also contain gaps; captured/requested
counts and time windows are preserved in the
[scan evidence](research/bridge-defaults-20261020-expanded-scan.json).
Mint references and transaction counts are not necessarily buy/sell counts.

## Depth admission: $500

Per the configured selection policy, a default candidate requires:

- At least **$500 on its liquid USDC/WSOL side**.
- Successful cached quotes in both directions at approximately **$10**.
- Runtime order-size impact is checked separately against the user-configured limit.
  The default is 100 bps per conversion hop; execution tolerance also defaults to 100 bps.
- An SDK-supported venue, valid pool/mint identities and usable mint extensions.

This uses liquid-side reserve, not total TVL. USDC is assumed $1 and WSOL is valued
through the WSOL/USDC small quote. The sell probe uses 10 times the stock output
of the $1 buy quote. Price impact is relative to the small quote, not absolute
fees or deviation from an external reference price. Small negative readings can
result from integer rounding and fee behavior. Concentrated-liquidity vault
balances alone do not establish executable depth, so bidirectional quotes are
required as well.

**55 of 69 candidates passed** the $500 reserve and small-order usability checks.
The initial fee-first selection yielded 30 stock/ETF pools for 30 mints, plus WSOL/USDC.
All candidates were reconsidered and accounts refreshed. Lowering the previous
$5k floor adds COPX.US, GMEx, INTC.US and SPCX.US. The initially sampled RACE.US
pool had sufficient vault reserve but no usable cached quote and remains excluded.
Small pools are catalog candidates, not promises that large orders will pass.

A [subsequent seven-source scan](research/stock-discovery-20261020.json) added
Raydium AMM v4/CPMM/CLMM to the Stonk/Pump sampling. It fetched all 2,080 successful
deduplicated transactions from 1,000 signatures per source, covering source windows
within 2026-10-20 05:34:02–05:37:19 UTC+8. Eight new pools qualified (seven CLMM,
one Orca), adding HIMS.US, AMD.US, RACE.US and RDDT.US plus existing-mint alternatives.
The new RACE.US pool has usable quotes; it is distinct from the earlier rejected
pool. Current totals are 34 mints / 63 qualified pools, with 64 trading defaults
including WSOL/USDC. These short windows do not establish exhaustive coverage.
User-configured per-order protection determines whether a route is executable.

## Transfer tax and pool trading fees

All 55 observed official mint accounts were separately checked at the epoch in
`research/bridge-mint-fees-20261020.json`: their current SPL transfer tax is **0 bps**.
This is not a claim of zero DEX fees or a guarantee against future configuration
changes. The expanded catalog now has 34 mints with pools passing admission checks.

Selected pools' base and snapshot-start fee rates, dynamic-fee flags and slots
are stored in the catalog's `fees` records and the
[per-pool fee CSV](research/bridge-default-fees-20261020.csv). The [offline fee auditor](research/audit_bridge_fees.py) replays these fields
from the retained snapshots without RPC (Python `solders` required). This is an
optional historical research tool, not part of the Rust test suite. For native
Rust catalog validation, use the parser SDK's `stock_pool_fees` and
`stock_catalog_screen` examples; they intentionally report unknown dynamic rates
instead of claiming to reproduce this tool's initial tick/bin rate. Rust trading
quotes use the supported venues' amount-specific fee calculations.
Protocol shares are allocations of
the trading fee, not an additional charge. These rates cover one bridge pool;
a SOL→USDC hop and the final PumpSwap trade add their own fees. Network fees,
priority fees, ATA rent and price impact are separate.

Runtime routing still compares protected net output using fresh cached accounts,
including the supported venue's dynamic fees. Catalog fee preference does not
replace order-size-specific quoting and introduces no RPC in the trading path.

Admission is a point-in-time check. Refresh depth outside the trading path and
replace the configured routing instance when pools no longer satisfy your
policy. User extensions are explicit choices and do not automatically inherit
this catalog's identity/depth certification. Larger orders require size-specific
quotes. Pool membership never substitutes for fresh account state.

## Cache behavior and validation

Creation, merging and trade preparation perform no RPC. Subscribe to the pool,
mints, vaults, configs, tick/bin arrays, bitmap extensions and Clock needed by
selected markets; publish coherent snapshots and invalidate on disconnect.
Missing or unusable paths are skipped. No complete fresh route means an error.
Expanding the catalog does not fetch or silently fabricate missing state.

Routing tests check official issuer records against decoded pool mint pairs,
recorded depth admission, deduplication/limits, atomic conflict handling, partially
populated default caches, repeatable idempotent ATAs and absence of hot-path RPC.
The depth auditor supports AMM v4, CLMM, DLMM, CPMM and Whirlpool snapshots.

```sh
cargo test --lib client::pumpswap_routing --features parser-adapter
cargo run --example pumpswap_global_routing
# Minimal historical depth fixture (three markets):
cargo run --example bridge_pool_depth -- tests/fixtures/bridge_defaults_20261020.json
# Other captured coherent snapshots use the same format:
cargo run --example bridge_pool_depth -- /path/to/pool-snapshot.json
```

The expanded audit's per-pool snapshots are retained in the task workspace under
`artifacts/bridge-expanded-20261020/*-snapshot.json`; selected snapshot SHA256s
are included in the catalog evidence. Each snapshot has its own coherent slot;
they must not be combined into a fictitious single-slot trading cache.

Earlier read-only mainnet simulations of the TTWO route passed: two full buys at
slot 458967103 (402730 CU, eight idempotent ATA setups), and a buy/sell roundtrip
at slot 458967287 (376308 CU, seven setups). These are TTWO transaction simulations;
the newly added markets were validated with official identities, live account
snapshots and bidirectional cached quotes, not complete PumpSwap target trades.
Each multi-hop buy or sell remains one atomic transaction.

See [streaming and quote examples](STREAMING_QUOTES.md) for parser/streamer
gRPC and shred integrations, per-hop protection and background cache lifecycle.
