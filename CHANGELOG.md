# Changelog

## v9.0.0 — 2026-10-22

Synchronize SDK code, examples, version navigation and AI release rules. Read the [release and migration notes](RELEASE_NOTES.md) for callback removals, V3 defaults and the Go /v9 migration.

Historical release and integration notes preserved from the README. Versions, dependencies and validation results describe those releases. See the [README](README.md) and topic guides for current usage.

## v8.0.0

Shared stock catalog, cached quote and routing improvements. See [release and migration notes](RELEASE_NOTES.md).

## v7.0.0 — Configured bridge pools and RPC-free automatic routing

Register candidate bridge pools once with `TradeConfig::builder(...).pumpswap_routing(...)`, then pass `DexParamEnum::PumpSwapAutoRoute(pool)` to buy or sell. Applications publish coherent account/Clock snapshots in the background. Selection, quoting and construction read only the cache; missing, expired or unavailable routes fail without RPC fallback. This mode rejects synchronous simulation and RPC confirmation polling; submission still uses the configured senders.

**One multi-hop buy or sell executes atomically in one transaction.** For example, `SOL → USDC → TTWO → target` contains three swaps in a single transaction. If a step fails, swaps and account creation roll back; transaction fees still apply. Intermediate ATAs use idempotent creation and can be reused. `Auto` account policy is applied after routing so first-time buys include WSOL and bridge accounts.

Migration: `TradeConfig` and `TradingClient` gain a `pumpswap_routing` field. Struct literals must add `pumpswap_routing: None`, or use existing constructors/builders. `DexParamEnum` gains `PumpSwapAutoRoute(Pubkey)`; exhaustive matches must handle it. These source changes require a major version. Existing builder calls leave automatic routing disabled. Dependencies use published parser `0.7.18` and matching example streamer `3.0.16`, without a sibling parser path dependency.

Validation: 440 library tests and one official quote comparison passed; all workspace targets compiled. Live read-only mainnet simulations on 2026-10-19 passed for repeated SOL buys, repeated WSOL buys and one buy/sell roundtrip (`err=null`). They used virtual funding and never broadcast transactions. Each complete buy/sell has three hops. Combining two buys and one sell into nine swaps exceeded the CPI trace limit; a normal multi-hop trade is not split into multiple transactions.

Examples: [offline global configuration](examples/pumpswap_global_routing.rs) and [live mainnet simulation](examples/pumpswap_global_routing_simulate.rs). See the [routing guide](docs/PUMPSWAP_QUOTE_ROUTING.md) for configuration, limits and commands.

## v6.0.2 — PumpSwap custom quotes and creator account fixes

Builds PumpSwap buys and sells with the actual quote mint/token program, validates creator vault identities, honors explicit fee recipients in mayhem mode, and removes an unnecessary global volume write lock. Adds cached quotes, conversion-route selection and complete offline signing preparation, with TTWO pool and SOL→USDC→TTWO→target buy/sell regressions.

Four language implementations share a real 3007 failure fixture and compare instruction data and account privileges. Native local bank regressions use captured public programs/accounts and synthetic wallets. See [PumpSwap fixes and validation](docs/PUMPSWAP_REPORTED_FAILURE.md).

## v6.0.1 — Pump depth-2 and SpeedLanding regional updates

- Corrects the Temporal default minimum tip to 0.001 SOL, with Temporal/LunarLander boundary regressions.

Adds Pump depth-2 create_v2 account chains, including curve and migrated-pool combinations. Preserves stored-depth chains when an ancestor is later whitelisted. Synchronizes SpeedLanding regional endpoints, including London and the New York domain, and compatible client routing. Account construction adds no RPC calls.

Validation includes official Pump SDK 4.1.0 instruction comparisons, signed local LiteSVM simulations and SDK regressions. Local simulation state/config is synthetic; no funded transactions were broadcast and production latency was not measured.

## v6.0.0 — Signed transaction and hot-path hardening

Adds optional decimal SOL minimum tips to SWQoS provider configuration. Providers below the configured threshold are skipped before instruction construction, signing and submission; equality remains eligible and omitted thresholds preserve existing behavior.

Breaking migration: provider tuple constructors now take a final `Option<f64>` argument, for example `SwqosConfig::Jito(api_key, region, endpoint, Some(0.0001))`. Pass `None` to retain previous behavior. `Default(String)` is unchanged.

Hardens cached preparation, durable nonce lifecycle, transaction pools and integer transfer-fee boundaries. Adds signed transaction, ALT and offline bank regression coverage.

Validation includes local CPU benchmarks and offline signed-bank scenarios. Measured hot paths use cached inputs without RPC. Benchmarks do not establish production network or transaction-landing latency. No funded mainnet transactions were broadcast.

## Earlier protocol integration notes

### Existing protocol integration notes

Version 5.0.7 updates CPMM creator-fee collection to the protocol-share upgrade: both collection builders append the required share PDA/config accounts, and AmmConfig decoding exposes the share rate. Adds cached gRPC preparation and collection/swap/LP simulation examples with offline mainnet fixtures. Uses parser 0.7.8 and streamer 3.0.7. See [CPMM migration](docs/cpmm-creator-fee-share.md).

Version 5.0.7 adds cache-backed StonkFun quote routes for independent SOL/WSOL/USDC/stock buys and sells, including direct USDC/stock conversion. The optional `parser-adapter` feature connects parser route clues and gRPC snapshots to local quote/build without hot-path RPC. See [gRPC simulations](docs/STONKFUN_GRPC_EXAMPLES.md) for verified V1 examples and cache boundaries.

This release adds first-class shared-program trading through `DexType::LaunchLab`, `DexParamEnum::LaunchLab`, and `LaunchLabParams`, plus platform-specific StonkFun names through `DexType::StonkFun`, `DexParamEnum::StonkFun`, and `StonkFunParams`. It supports dynamic quote mints and token programs, reads current LaunchLab pool and fee configuration by RPC, and builds the current 18-account buy/sell instruction layout. The same `DexType::StonkFun` routes graduated pools when paired with `DexParamEnum::StonkFunSwap` / `StonkFunSwapParams`: arbitrary token pairs, mixed SPL Token/Token-2022 programs, current AmmConfig and creator fees, transfer fees, vault balances, and both swap directions are resolved from mainnet state. Buy quotes also reduce the submitted input at the curve graduation boundary, matching the official LaunchLab SDK. For wallets that only hold SOL, `DexParamEnum::StonkFunViaSol` / `StonkFunViaSolParams` builds an atomic `SOL ↔ quote ↔ meme` two-hop for both the LaunchLab curve and graduated CPMM legs; the SOL↔quote hop currently supports Raydium CPMM and AMM v4. Existing Bonk and `RaydiumCpmm` names remain available for compatibility and direct underlying-protocol access.

Explicit `StonkFunSolHop::Route` funding paths additionally support CLMM, Whirlpool, DLMM, intermediate mints and split/merge execution, with independently quoted buy/sell paths. See [StonkFun quote routes](docs/STONKFUN_ROUTES.md) for the API, fee handling and subscription-fed local quotes and remaining discovery requirements. With `parser-adapter`, `SubscriptionAccountCache::prepare_stonkfun_trade` prepares a complete independent buy or sell without hot-path RPC; see `examples/stonkfun_cached_prepare.rs`.

PublicNode gRPC cache + mainnet simulation examples: [guide](docs/STONKFUN_GRPC_EXAMPLES.md).

`SimpleBuyParams::stonkfun_with_token` / `SimpleSellParams::stonkfun_to_token` accept SOL, USDC, WSOL or the pool's stock quote. SOL funds wrapping; WSOL spends existing tokens. Under Auto, SOL receipt unwraps while WSOL/USDC accounts stay open. `StonkFunViaQuoteParams::curve_direct` / `graduated_direct` skip conversion for already-held quotes. Each buy/sell is a separate transaction composed directly from DEX instructions, requiring no router contract.

The gated mainnet regressions use a current StonkFun reward pool and the graduated KNOTS/STONK CPMM pool. Run them without submitting a transaction:

```bash
# All mainnet simulate suites (ViaSol / direct StonkFun / Raydium CPMM)
# Prefer a private RPC — public mainnet endpoints rate-limit under this suite.
SOLANA_RPC_URL=... RUN_MAINNET_TESTS=1 cargo test --lib mainnet -- --nocapture --test-threads=1

# Optional live PumpFun bonding-curve buy:
PUMPFUN_MINT=<mint> RUN_MAINNET_TESTS=1 cargo test --lib pumpfun_mainnet -- --nocapture
```

Tests create an ephemeral `Keypair::new()` wallet and virtually fund it inside `simulateTransaction(sigVerify=false)` from a high-balance mainnet account. No `PRIVATE_KEY` and no on-chain submit. Shared helpers live in `src/common/mainnet_sim.rs` (retry + soft-skip on transient RPC errors).

Coverage:

| Suite | Paths |
|---|---|
| `stonkfun_via_sol_mainnet` | graduated buy, curve buy, buy+sell, HotPathMinimal, wider slip, close-WSOL sell |
| `stonkfun_mainnet` | direct curve quote↔meme, graduated StonkFunSwap, roundtrips, Bonk/LaunchLab aliases |
| `raydium_cpmm_mainnet` | WSOL↔STONK, WSOL↔CARDS, graduated KNOTS/STONK, exact-out (STONK+CARDS) |
| `raydium_amm_v4_mainnet` | WSOL↔USDT buy/sell/roundtrip, WSOL→USDC buy/exact-out, seed-optimize |
| `raydium_clmm_mainnet` | SOL↔USDC buy/roundtrip, reverse after hop, SOL↔USDT, larger+wide-slip |
| `whirlpool_mainnet` | SOL→USDC buy/roundtrip, USDC→SOL after hop, SOL→USDT, USDT reverse |
| `pumpswap_mainnet` | from_mint, canonical/seed buy+sell, close-WSOL, USDC combined, classic/tiny/seed-optimize |
| `meteora_damm_v2_mainnet` | USDC meme hop, exact-out, sell build, SOL/USDC direct buy + reverse build |
| `meteora_dlmm_mainnet` | SOL→USDC buy/roundtrip, reverse after hop, high-TVL alt both dirs |
| `cross_dex_mainnet` | CLMM↔Whirlpool compose, AMM→DAMM/DLMM/CLMM, triangle AMM→Whirlpool, CLMM→PumpSwap |
| `pumpfun_mainnet` | optional `PUMPFUN_MINT=...` bonding-curve buy + buy/sell |

Example:

```bash
RPC_URL=... cargo run -p stonkfun_via_sol_simulate
RPC_URL=... cargo run -p stonkfun_via_sol_simulate -- --curve
```
