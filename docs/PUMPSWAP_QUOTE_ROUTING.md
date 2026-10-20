# PumpSwap quote tokens and explicit conversion routes

PumpSwap buy/sell builders match the requested input and output against the pool's
base and quote mints. A quote-to-base swap uses `buy_exact_quote_in` (or `buy` for
exact output); base-to-quote uses `sell`. This works for custom quote mints such
as TTWO, including Token-2022, and for pools whose base is WSOL or USDC.
Amounts are raw units of the corresponding mint, not necessarily lamports.

Each complete multi-hop buy or sell is assembled into **one atomic transaction**.
For example, SOL → USDC → TTWO → target is three swaps in one transaction.
Any failing step rolls back swaps and account creation; transaction fees still
apply. The live examples test repeated buys and buy/sell roundtrips separately
because combining two full buys plus one full sell (nine swaps) exceeded the
runtime CPI trace limit. They do not split an individual route across transactions.

## Configure bridge markets once per client

`PumpSwapRouting` stores candidate conversion markets and an atomically replaceable
subscription snapshot. Register it during initialization:

```rust,ignore
let routing = Arc::new(PumpSwapRouting::new(
    PumpSwapRoutingConfig::new(vec![sol_usdc_pool, usdc_ttwo_pool]),
)?);
let config = TradeConfig::builder(rpc_url, senders, commitment)
    .pumpswap_routing(routing.clone())
    .create_wsol_ata_on_startup(false)
    .build();
let client = TradingClient::new(wallet, config).await;

// Background subscription consumer: publish a coherent cache + current Clock.
routing.publish(Arc::new(cache), context, chain_unix_timestamp);

// Trade hot path: candidate pools are already configured.
let request = SimpleBuyParams::new(
    DexType::PumpSwap, TradeTokenType::SOL, target_mint,
    BuyAmount::ExactInput(lamports),
    DexParamEnum::PumpSwapAutoRoute(target_pool),
    cached_blockhash, gas,
);
client.buy_simple(request).await?;
```

For shared infrastructure, attach the same handle using
`TradingClient::from_infrastructure(...).with_pumpswap_routing(routing.clone())`.
Cloned clients share the handle; unrelated instances have independent registries.
Add any pool supported by the existing cached quote engine to `conversion_pools`.
Adding a new DEX protocol still requires a quote/builder implementation.

`PumpSwapAutoRoute` accepts only a market identity. Configured clients also
rebuild ordinary `DexParamEnum::PumpSwap` requests from that pool's cached state.
`PumpSwapQuoteRoute` remains an explicit per-trade override; the caller owns its
quote freshness. Both buy and sell support exact input. Sell paths are searched
and quoted independently. Direct trades remain possible when the funding token
already belongs to the selected pool.

Publishing does **not** subscribe automatically: the application updates accounts,
Clock, slot and epoch in the background and publishes one coherent snapshot.
The reader retains one immutable snapshot throughout a search. Preserve each
account's real update slot and fork; never label old bytes as freshly fetched.
The configured slot age and context's slot age are both enforced. A local
snapshot timeout (default two seconds) also rejects stopped subscriptions.
Call `clear()` on disconnect/reorg. Refresh bin/tick arrays as active liquidity
moves. Blockhash/nonce and real ALTs must likewise be supplied from background
state. This API does not discover markets or migrate seeded token balances.

Configured PumpSwap requests have `rpc=None` and reject `simulate=true` or
`wait_tx_confirmed=true`. Missing, stale, expired, unsupported or disconnected
routes return errors without network fallback. `prepare_simple_buy/sell` expose
the same local preparation used by `buy_simple/sell_simple`, without submitting;
`prepare_buy/sell` expose the low-level equivalent and preserve manual ATA flags.
`Auto` account flags are resolved after routing; `AssumePrepared` and
`HotPathMinimal` require existing standard ATAs, including the bridges.

The SDK's selection/quoting/build path makes no RPC reads. Custom risk gates and
middleware must also use local data. Submission still uses the chosen senders
(an RPC sender submits over RPC); confirmations belong in background subscriptions.
Initialization and explicit external simulations may perform network reads.

Runnable examples:

```bash
# Historical fixture; no network or broadcast.
cargo run -p sol-trade-sdk --example pumpswap_global_routing --locked
# Fresh mainnet pools, ephemeral wallet, virtual simulation-only funding.
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com \
  cargo run -p sol-trade-sdk --example pumpswap_global_routing_simulate --locked -- sol-repeat sol-repeat.json
# Other modes: wsol-repeat, roundtrip
```

The live example keeps both complete sets of ATA setup instructions. In
`wsol-repeat`, only the simulation harness prepares the initial WSOL balance;
the production buys spend WSOL without adding SOL transfers. `roundtrip` buys
once, then sells the guaranteed acquired amount back to native SOL. The scenarios
are separate because three complete three-hop routes can exceed the runtime CPI
trace limit even when the message fits the transaction size limit. Fresh pool
reads occur before local preparation; `simulateTransaction` occurs afterwards.
It uses V1, requires node support, disables signature verification only for the
virtual funder, and never calls a transaction submission endpoint. Reported
hot-path time is one diagnostic sample, not a latency benchmark.

## Directly spending the pool quote

Load a coherent pool snapshot with `PumpSwapParams::from_pool_address_by_rpc`,
or prepare `from_cached_pool_snapshot_strict` from verified subscription data.
Use the pool address when a mint has several pools. Supply current fee config,
mint programs, reserves, cashback flags and fee recipients.

```rust
use sol_trade_sdk::{BuyAmount, SimpleBuyParams, TradeTokenType};
use sol_trade_sdk::common::GasFeeStrategy;
use sol_trade_sdk::trading::{factory::DexType, core::params::{DexParamEnum, PumpSwapParams}};
use solana_sdk::hash::Hash;

fn direct_buy(pool: PumpSwapParams, amount_in: u64, blockhash: Hash) -> SimpleBuyParams {
    SimpleBuyParams::new(
        DexType::PumpSwap, TradeTokenType::Token(pool.quote_mint), pool.base_mint,
        BuyAmount::ExactInput(amount_in), DexParamEnum::PumpSwap(pool),
        blockhash, GasFeeStrategy::new(),
    )
}
```

The wallet must already hold the quote token. For example, Dacity/TTWO is pool
`G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b`, with TTWO quote mint
`TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo` (6 decimals, Token-2022).
Passing SOL as the input to this single pool is rejected: it needs a conversion.

## Prepare from a subscription cache

`SubscriptionAccountCache::prepare_pumpswap_trade` quotes the complete route,
including the final PumpSwap leg. Pass one `CachedRouteStep` for a direct trade,
or explicit conversion steps followed by PumpSwap. The caller chooses the pools;
this API does not discover markets. Use `None` for each step's `input_amount` to
spend only its available guaranteed credit.

```rust
use anyhow::Result;
use sol_trade_sdk::{SimpleBuyParams, common::GasFeeStrategy};
use sol_trade_sdk::trading::core::params::{
    CacheReadContext, CachedQuoteRequest, CachedRouteStep, SubscriptionAccountCache,
};
use solana_sdk::{hash::Hash, pubkey::Pubkey};

fn prepare_buy(
    cache: &SubscriptionAccountCache,
    steps: &[CachedRouteStep],
    input_mint: Pubkey, // SOL sentinel, WSOL, USDC, or another funding mint
    target_mint: Pubkey,
    quote: CachedQuoteRequest,
    context: CacheReadContext,
    blockhash: Hash,
) -> Result<SimpleBuyParams> {
    let prepared = cache.prepare_pumpswap_trade(
        steps, input_mint, target_mint, quote, context, true,
    )?;
    println!("Protected output: {}", prepared.preview().minimum_output_credit);
    prepared.into_buy_params(blockhash, GasFeeStrategy::new())
}
```

For sells, provide independently ordered/requoted steps with `is_buy = false`
and call `into_sell_params`. The prepared value keeps amount, direction, mint
selection and slippage together. `apply_to` also supports low-level `SwapParams`,
retaining the caller's wallet, account flags, ALTs and nonce/blockhash. It requires
simulation and confirmation flags to be off and removes the RPC client.
High-level constructors default to `AccountPolicy::Auto`.

Cache ingestion can use `update` with uncompressed account bytes, or
`update_from_owned_parser_snapshot` / `update_from_parser_snapshot` under the
`parser-adapter` feature. `PoolTradeHint::from_route_leg` accepts PumpSwap identity
clues; it never copies the observed wallet or amounts. A hint is not a fresh quote
or evidence that an observed trade filled.

`prepared.steps()` exposes the selected pools, mint directions and exact quoted
inputs for inspection; native SOL endpoints appear as WSOL in these pool steps.

Each PumpSwap cache read requires the pool, both mints, both token vaults,
`pumpswap::accounts::GLOBAL_ACCOUNT`, and `pumpswap::accounts::FEE_CONFIG`.
Conversion steps additionally need their own pools/configs/vaults and relevant
tick/bin arrays. Supply a consistent context from the chain Clock/slot stream.
Missing, stale, future, or wrongly owned accounts fail before instruction build.
Vault mint, authority and initialized state are checked; mint transfer fees are
evaluated at the supplied epoch. Refresh/reprepare after state or epoch changes.
The age check bounds staleness; it does not establish fork consistency for the
caller. Slippage applies **per hop**, not once across the entire route.

The cached PumpSwap path rejects active mint transfer fees, unsupported mint
extensions, and cashback pools whose cashback rate is absent from Pool state.
Cashback requires explicit verified fee parameters through the lower-level API.
These limits are separate from direct support for ordinary Token-2022 mints.

Run the offline example with the captured TTWO snapshot:

```bash
cargo run -p sol-trade-sdk --example pumpswap_cached_prepare --locked -- buy 10000
cargo run -p sol-trade-sdk --example pumpswap_cached_prepare --locked -- sell 1000000000
# Optional third argument: your exported snapshot JSON, with explicit buy_steps/sell_steps.
cargo run -p sol-trade-sdk --example pumpswap_cached_prepare --locked -- buy 10000 snapshot.json
```

By default the example constructs a high-level request without a signer, RPC or broadcast.
Its bundled historical snapshot and placeholder blockhash are for demonstration;
live execution requires current state and blockhash. The snapshot JSON contains
uncompressed `data_base64`; RPC `base64+zstd` must be decompressed first.

## Select a path from cached candidate pools

`prepare_best_pumpswap_trade` accepts an unordered allowlist of conversion pools.
The SDK finds mint-connected paths, determines each swap's direction, quotes the
entire path at your input amount, and chooses the highest protected final output.
This removes manual ordering of known markets. It does not scan the network for
pools or promise the best price across markets outside your allowlist.

```rust
use anyhow::Result;
use sol_trade_sdk::trading::core::params::{
    CacheReadContext, CachedQuoteRequest, PoolTradeHint, PumpSwapRouteSearch,
    PumpSwapRouteSelection, SubscriptionAccountCache,
};
use solana_sdk::pubkey::Pubkey;

fn choose_route(
    cache: &SubscriptionAccountCache,
    pool: Pubkey, token_mint: Pubkey, asset_mint: Pubkey,
    candidates: Vec<PoolTradeHint>,
    request: CachedQuoteRequest, context: CacheReadContext, is_buy: bool,
) -> Result<PumpSwapRouteSelection> {
    let search = PumpSwapRouteSearch::new(pool, token_mint, asset_mint, candidates);
    cache.prepare_best_pumpswap_trade(&search, request, context, is_buy)
}
```

`pool` fixes the PumpSwap market used to buy/sell `token_mint`; the token can be
either mint in that pool. `asset_mint` is the payment/receipt asset, including the
native SOL sentinel. Candidate hints contain a pool and its two mints in either
orientation; historical instruction amounts are not used. Buy and sell calls
search and quote independently. If the chosen asset is already the pool's other
mint, the result is a direct one-step trade.

The result contains `trade`, `routes_evaluated`, and `rejected_routes` (pool paths
and reasons). A stale, malformed, unsupported or illiquid candidate path is
excluded while other paths remain eligible. No valid path returns an error with
the first rejection cause. Conflicting hints for the same pool are rejected;
duplicate/reversed hints are deduplicated. Ties prefer fewer hops, then canonical
pool pubkey byte order, independent of input order.

Defaults allow two conversion hops plus the PumpSwap leg and 128 explored search
nodes. At most 32 candidate hints, three conversion hops and 4,096 search nodes
are accepted. `maximum_search_nodes` counts partial paths and dead ends as well
as complete paths; exhaustion returns an error rather than a partially searched
result. The search excludes cycles and reuse of the selected pool, and selects
one serial path. Order splitting remains available through explicit steps.

Ranking uses protected token output after pool/mint fees and per-hop slippage.
It does not subtract SOL gas/rent, inspect the wallet balance, estimate the
transaction packet size, or choose ALTs. Cache freshness and supported-mint
restrictions described above also apply to every searched path.

A captured full-route snapshot makes both directions runnable without RPC:

```bash
cargo run -p sol-trade-sdk --example pumpswap_cached_prepare --locked -- \
  buy 1000000 tests/fixtures/pumpswap_ttwo_route_accounts.json
cargo run -p sol-trade-sdk --example pumpswap_cached_prepare --locked -- \
  sell 1000000000 tests/fixtures/pumpswap_ttwo_route_accounts.json
```

This JSON uses `route_search` instead of ordered `buy_steps`/`sell_steps`. Its
candidate DLMM pool is intentionally listed before AMM v4 and with the opposite
mint orientation. The example prints the selected execution order and amounts.

## Build the complete transaction offline

`PreparedPumpSwapTrade::build_transaction` locally signs one transaction using
the production builder. Supply the caller's `SwapParams`, latest subscription
context, one chosen lane's `GasFeeStrategyValue`, and that lane's tip recipient.
The method overrides trade fields using the prepared quote, removes the RPC
client, and requires `simulate = false` and `wait_tx_confirmed = false`.
It does not submit, reserve a nonce, select a relay or check wallet balances.

```rust
use anyhow::Result;
use sol_trade_sdk::{common::GasFeeStrategyValue, trading::core::params::{
    CacheReadContext, PreparedPumpSwapTrade, SwapParams,
}};
use solana_sdk::{pubkey::Pubkey, transaction::VersionedTransaction};

async fn build(
    prepared: PreparedPumpSwapTrade, params: SwapParams,
    latest: CacheReadContext, lane_fee: GasFeeStrategyValue, tip_recipient: Pubkey,
) -> Result<VersionedTransaction> {
    prepared.build_transaction(params, latest, lane_fee, tip_recipient).await
}
```

`validate_freshness(latest)` is also available before converting to a high-level
request. It rejects epoch changes, future preparation slots, and quote ages
exceeding the smaller of the original/current `maximum_slot_age`. This check is
time since preparation; it does not re-read source accounts or extend their
freshness. Reprepare to validate current accounts, prices and fees. The caller
must supply a current context and maintain consistency of its subscription data.
`apply_to` and the high-level request constructors do not perform this later age
check themselves because they do not receive a current context.

Complete construction retains nonce/blockhash, account flags, version and ALTs.
It runs both protocol and full-transaction middleware, then checks signed wire
size and unsupported extra signers. Nonce advance, compute settings and relay tip
are included. Caller-provided middleware must remain offline for a fully offline
build. A nonce must have both account and hash; its authority/state and blockhash
validity remain caller responsibilities. Tip must be finite, nonnegative and
representable in lamports; a positive enabled tip requires a destination.

V0 without ALTs uses the legacy message format and the 1,232-byte limit. A V0
route may require actual on-chain ALTs. V1 does not accept ALTs. The builder never
switches versions or removes priority instructions to make a transaction fit.
The chosen validator and sending provider must support the configured version.

The example's optional fourth argument demonstrates complete construction with
an ephemeral wallet and synthetic blockhash; it verifies the local signature:

```bash
cargo run -p sol-trade-sdk --example pumpswap_cached_prepare --locked -- \
  buy 1000000 tests/fixtures/pumpswap_ttwo_route_accounts.json v1
cargo run -p sol-trade-sdk --example pumpswap_cached_prepare --locked -- \
  sell 1000000000 tests/fixtures/pumpswap_ttwo_route_accounts.json v1
```

These examples use historical state and never submit. Passing `v0` for this
three-hop fixture deliberately fails the wire-size check without ALTs. Synthetic
ALTs in unit tests demonstrate compilation only; they cannot be used on chain.

## SOL/USDC → quote → target in one transaction

`DexParamEnum::PumpSwapQuoteRoute` composes an explicit sequence of exact-input
swaps. `PumpSwapQuoteRoute`, `PumpSwapQuoteHop`, and `PumpSwapQuoteVenue` reuse the
existing quote-route planner, including Raydium CPMM/AMM v4/CLMM, Orca Whirlpool,
Meteora DLMM, and the new PumpSwap venue. Prepare current conversion pools,
direction-specific tick/bin arrays and minimum output quotes before building.
The builder does not discover pools or request quotes over RPC.

```rust
use sol_trade_sdk::{PumpSwapQuoteHop, PumpSwapQuoteRoute, PumpSwapQuoteVenue};
use sol_trade_sdk::trading::core::params::{PumpSwapParams, TokenTransferFee};

fn append_pool_buy(
    mut conversions: Vec<PumpSwapQuoteHop>,
    pool: PumpSwapParams,
    minimum_base_out: u64,
    base_fee: TokenTransferFee,
    quote_fee: TokenTransferFee,
) -> PumpSwapQuoteRoute {
    // Last conversion must output pool.quote_mint. Quote this final leg using
    // only that conversion's guaranteed minimum credit, not its estimated output.
    let input = pool.quote_mint;
    let output = pool.base_mint;
    conversions.push(PumpSwapQuoteHop::exact_in(
        PumpSwapQuoteVenue::PumpSwap {
            pool, base_transfer_fee: base_fee, quote_transfer_fee: quote_fee,
        },
        input, output, minimum_base_out,
    ));
    PumpSwapQuoteRoute::buy(conversions)
}
```

Pass the result as `DexParamEnum::PumpSwapQuoteRoute(route)` to
`SimpleBuyParams::new(DexType::PumpSwap, TradeTokenType::SOL /* or USDC/WSOL */,
target_mint, BuyAmount::ExactInput(amount), ..., blockhash, gas)`.
`route.preview(input_mint, output_mint, amount, slippage_bps, true)` validates
the amounts and mint continuity without RPC. `minimum_amount_out` must be nonzero.
The builder rejects a PumpSwap minimum above its current protected quote.

Each following leg spends only guaranteed credit. Better fills can leave quote
dust in the wallet. Intermediate token accounts stay open. `AccountPolicy::Auto`
creates required route accounts, wraps native SOL once, and unwraps SOL proceeds
on a sell; WSOL input spends existing WSOL. `AssumePrepared` requires existing
accounts. Supply ALTs or an appropriate transaction version for large routes.
All `PumpSwapQuoteRoute` user token accounts are standard ATAs, even when
`open_seed_optimize` is enabled. Setup uses idempotent ATA creation and every leg
uses the same ATA addresses, allowing repeated trades with existing intermediate
accounts. `AssumePrepared` also expects those ATAs. Existing seeded-account
balances are not automatically migrated. Direct pool and StonkFun seed behavior
is unchanged.
The three-leg mainnet simulation uses V1; set
`.transaction_version(TradeTransactionVersion::V1)` on the `TradeConfig` builder when
using V1, or prepare actual ALTs to fit V0 packet limits.

Prepare independent sell hops in execution order, using `with_sell_hops` or
`PumpSwapQuoteRoute::sell`, and submit through `SimpleSellParams` with
`DexType::PumpSwap`. The buy route is never automatically reversed. Route sizing
supports exact input only; exact-output and `WithMaxInput` requests are rejected.

PumpSwap route legs currently require **zero active transfer fees on both mints**.
Supply actual epoch-specific fee snapshots, not guessed zero values. External
Token-2022 output hops also require `with_output_transfer_fee` (or its RPC helper).
Mint extensions requiring additional transfer-hook accounts are outside this
builder's supported path; arbitrary Token-2022 extensions are not implied by
support for the Token-2022 program.

## Events, creator vaults and native Pump routes

`validate_accounts()` runs before a direct instruction is built. It rejects
missing execution accounts, unsupported token programs, a creator authority that
does not match `coin_creator`, or a creator ATA that does not match the actual
quote mint and quote token program. It does not check live account existence or
freshness, and performs no RPC reads.

Legacy trade instructions expose creator vault accounts at positions 17 and 18.
V2 and `multi_hop_swap` have different layouts. Log-only events can contain
default account fields; do not blindly pass them to `from_trade`. Enrich them
from a verified pool cache or load a fresh pool snapshot. An emitted route intent
describes requested execution and is not proof of a filled trade.

The reported `WSOL → hi → OK` samples use Pump's native `multi_hop_swap`, including
a PumpFun curve leg. Existing low-level `derive_pump_multi_hop_accounts` and
`build_pump_amm_multi_hop_swap_instruction` support that separate path. The
quote-conversion wrapper above does not automatically convert a native route
intent into a new copy trade or discover an external SOL/TTWO market.

### Confirmed reported 3007 failure

See [cross-language fixes and regression commands](PUMPSWAP_REPORTED_FAILURE.md)
for creator validation, explicit fee recipients and readonly volume accounts.

Transaction `34mNYS59JLuy6q55gS1PbYY2DN9v1mtkuCMhdoXKRaiB5WczkfweVAhuTC7dVyEpLzg94T4HvNd5Eu75Gbkaodbe`
failed at outer instruction index 5 (the sixth instruction), `PumpSwap Buy`,
with `coin_creator_vault_ata` / error 3007. This specific transaction trades
`2kuY5DHqRvYKfytfLfrey1oB1Z1J3voq343FWMp6pump` (current mint name `dev/null`)
against WSOL in pool `BwTKRFjssVL6c7Wb8MjvHMy9BET1Jb3xSKXcxz8t4RH`.

Its creator ATA and creator authority, instruction account positions 17 and 18,
are **both `11111111111111111111111111111111`**. That value is `Pubkey::default()`
and the executable System Program address, not the required creator token vault.
Thus this failure is caused by incomplete creator accounts; it does not establish
that a SOL-to-TTWO conversion was missing. The base mint uses Token-2022 and the
WSOL quote uses the classic Token program.

The current builder rejects those default fields before returning instructions.
Prepare from verified Pool/mint/vault/config state using the cache or
`PumpSwapParams::from_pool_address_by_rpc`. Do not fill missing accounts with zero
or derive a creator vault from an unverified default `coin_creator`. The historical
transaction alone cannot identify which caller/parser/SDK version lost the fields.

`pumpswap_reported_failure.json` preserves the failed instruction and a separately
captured later account snapshot. The offline regression rebuilds the historical
Buy payload with validated creator accounts and rejects the original defaults.
The gated `pumpswap_reported_3007_mainnet` test compares current-state Buy simulations
with correct accounts and with only positions 17/18 replaced by defaults. It uses
a small fresh quote, virtual funding and no broadcast; it does not replay the old
nonce or reproduce the historical price.

## Verification

```bash
cargo test -p sol-trade-sdk --lib pumpswap --locked
cargo test -p sol-trade-sdk --lib stonkfun_quote_route --locked
# Live, read-only simulation. Virtual SOL/TTWO funding, no broadcast or source key.
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com \
  cargo test -p sol-trade-sdk --lib pumpswap_ttwo_mainnet_simulates \
  --locked -- --ignored --nocapture --test-threads=1
```

`pumpswap_ttwo.json` preserves account identities from a real successful PumpSwap
leg (its enclosing Jupiter transaction failed later); that fixture's instruction
tests use synthetic pricing reserves. `pumpswap_ttwo_accounts.json` contains
decoded historical mainnet account bytes, capture references and explicit slots
for offline cache tests and the runnable example.

Live tests refresh current state and require the simulation funding accounts to
still hold enough tokens. The conversion tests build `SOL → USDC → TTWO → Dacity`
and independently `Dacity → TTWO → USDC → SOL`, using Raydium AMM v4, Meteora DLMM
and PumpSwap. All quote inputs for each test come from one RPC account batch.
The sell starts from virtually funded Dacity and checks WSOL closure. Both use
the new cache preparation API and simulate all three swaps atomically. Source
funding signatures are bypassed only in simulation; no transactions are sent.
The `pumpswap_ttwo_mainnet_simulates_searched_*` tests exercise the same routes
through automatic candidate selection using newly fetched account snapshots.
`pumpswap_ttwo_mainnet_simulates_repeated_seed_default_buy` executes two full
buys with seed optimization enabled and preserves all eight ATA setup instructions
without deduplication. This exercises creation against already initialized ATAs
within one simulation. It uses virtual funding and does not broadcast.

## Default bridge catalog

Use `PumpSwapRoutingConfig::default()` and `extend_pools(...)` for screened mainnet
markets plus deduplicated user additions. `new(...)` remains custom-only. See
[pool selection and depth evidence](PUMPSWAP_BRIDGE_DEFAULTS.md).
