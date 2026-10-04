# PublicNode gRPC cache and simulation examples

These examples use `https://solana-yellowstone-grpc.publicnode.com:443` as their
default gRPC endpoint. Set `GRPC_TOKEN` (or `GRPC_AUTH_TOKEN`) to the provider's free-service token
(`x-token`); it is not a wallet key. Tokens are not embedded in the new examples.
`GRPC_URL` (or `GRPC_ENDPOINT`) can select another Yellowstone endpoint.
`GRPC_URL` and `GRPC_TOKEN` take precedence over their legacy aliases. No private key is needed:
all transactions use an ephemeral wallet and are **simulated, never submitted**.
If `GRPC_URL` and `GRPC_TOKEN` are already exported in your shell, run the commands
below directly and omit the placeholder `export GRPC_TOKEN` lines.

## Raw parser subscription

From `sol-parser-sdk`:

```sh
export GRPC_TOKEN='<provider token>'
ACCOUNTS=HJPjoWUrhoZzkNfRpHuieeFk9WcZWjwy6PBjZ81ngndJ \
DURATION_SECONDS=30 cargo run --example grpc_raw_cache -- snapshots.jsonl
```

The example explicitly opts into `AccountRawSnapshot` and `BlockMeta`, subscribes
to the requested addresses plus Clock, writes raw account events as JSON lines,
writes block metadata to `snapshots.jsonl.blocks.jsonl`, and prints blockhashes. Raw events preserve account bytes, owner, lamports,
slot/write_version and closure evidence. Clock bytes contain the chain epoch and
Unix time. The stream is incremental: unchanged configs/mints/ALTs need startup
snapshots, and an empty stream fails rather than reporting success.

## Cache-backed StonkFun simulation

From `sol-trade-sdk`:

```sh
export GRPC_TOKEN='<provider token>'
# Only the cold bootstrap and simulator use this RPC endpoint.
export RPC_URL=https://api.mainnet-beta.solana.com

# Independent graduated buys with three payment assets.
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --asset SOL
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --asset WSOL
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --asset USDC

# Independent graduated sells with three receipt assets.
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --sell --asset SOL --amount 1000000
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --sell --asset WSOL --amount 1000000
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --sell --asset USDC --amount 1000000

# Direct USDC/stock CLMM pool: two business swaps, no WSOL intermediate.
# This selects a separate LaunchLab inner-pool fixture automatically.
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --direct-usdc --asset USDC --amount 50000
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --direct-usdc --asset USDC --sell --amount 100000000

# LaunchLab inner buy; direct stock quote sell has no conversion leg.
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --curve --asset SOL
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --sell --asset quote --amount 1000000
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --asset quote

# Conversion-only venue checks, separate from any meme buy/sell.
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --venue whirlpool --asset SOL
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --venue clmm --asset WSOL
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --venue dlmm --asset SOL
cargo run --example stonkfun_grpc_simulate --features parser-adapter -- --venue whirlpool --asset USDC
```

The examples use bounded real mainnet fixtures. A pool can migrate, lose
liquidity, or disable trading; such failures are reported, not silently skipped.
`--amount` is in input mint base units. `--timeout` bounds subscription warmup.
`SNAPSHOT_FILE=/tmp/cache.json` exports the snapshot used by a successful run.
Direct stock buys (`--asset quote`) use an unrelated conversion pool quote vault
for virtual funding. Set `SIM_TOKEN_SOURCE=<account>` to override fixture funding
or provide a source for custom mints; owner/mint/balance are validated and quoted dependency accounts
are rejected as sources. The default KNOTS source is an unrelated holder from a
checked-in real transaction, and can also become unavailable.

`--direct-usdc` uses CLMM `49iMatQtoyabsYAQc8GafVq6aeBFVDxSRH44oiatyyw6`
(USDC/stock mint `Xsc9qvGR1efVDFGLrVsmkzv3qi45LTBjeUKSPmx9qEh`) and LaunchLab
pool `BmQj9pBopxouHecN5rYvVLqN7a48CVndfTkhESEZzWgN`
(meme `DQsYFPcRKaKZWTjY4TiqvmxjN87vjumWMECJ7s4U1HbN`). It requires
`--asset USDC` and cannot combine with `--venue`. The buy conversion is one
`CachedRouteStep` from USDC to stock; the independent sell prepares one fresh
step from stock to USDC. Ordinary `--asset USDC` retains the existing
USDC/WSOL/stock route. To use other pools, provide their parser-derived identities
and dependency snapshots to `prepare_stonkfun_trade`; this fixture is not a pool finder.

### Data flow and RPC boundary

1. Discover fixture dependency identities in the **cold** path. In a real copy
   trader, pool/mint identities instead come from successful parser route legs.
2. Start gRPC subscriptions, wait for an actual BlockMeta event proving the stream
   connected, and bootstrap a complete **confirmed** account batch through RPC.
   Buffer later updates meanwhile. Ignore subscription writes at/before the
   bootstrap slot, whose write_version is unknown, to avoid reverting bytes.
3. Feed subsequent `RawAccountSnapshotEvent`s to `update_from_parser_snapshot`.
   Wait for live Clock and a blockhash watermark covering the applied writes.
   Conversion-only checks additionally require a real pool update. Inactive
   StonkFun fixtures can use unchanged bootstrap pools with live Clock/blockhash;
   the example reports `pool_updates=0` explicitly in those cases.
4. Quote and prepare `SwapParams` synchronously from the cache. Construct business
   instructions with `rpc=None`, `simulate=false`, `wait_tx_confirmed=false`.
   The blockhash and Clock come from gRPC. In V0 mode, ALTs are decoded locally through
   `decode_address_lookup_table_account`, with owner/layout validation.
5. Outside that boundary, add **simulation-only** artificial funding, compile the
   explicitly selected message version, enforce its wire-size limit and call `simulateTransaction` with
   `sigVerify=false` and `replaceRecentBlockhash=false`. Verify output credit
   against the local protected minimum. For native SOL output, verify WSOL was
   closed and recover wallet credit after accounting for retained ATA rent.

The simulated funding instructions lack real source signatures and must never
be sent. WSOL tests virtually prepare a WSOL balance before business instructions;
the business builder spends existing WSOL and does not fund its principal from
native SOL. Buys and sells run separately; this is not an arbitrage roundtrip.

The default is **V1**: no ALT discovery, subscription or lookup, up to 4,096 bytes,
64 addresses and 64 instructions, with inline compute configuration. Select
`--version v0` for the 1,232-byte V0 path. V0 permits optional ALTs; small V0
transactions do not require one. V1 rejects `ALTS` explicitly.

Only V0 USDC fixtures discover additional ALT identities from recent Whirlpool
transactions during cold startup, then include those accounts in the subscription
and bootstrap. Set `ALTS=<comma-separated keys>` to use an existing trusted list
instead of that discovery. Future pool/tick/bin changes can require additional
subscribed accounts; bounded fixture arrays cause a clear missing-state error,
not an RPC fetch in the trading path. Production should maintain dependency
subscriptions as pool state changes, refresh after reconnect/gaps, and handle
forks/commitment before publishing consistent cache snapshots. A BlockMeta
watermark alone does not provide a general atomic multi-account snapshot guarantee.

V1 defaults to a 900,000 CU inline limit and 64 MiB loaded-account data limit;
`--compute-unit-limit <units>` overrides that inline value. V0 defaults to Solana's
per-instruction budget; its explicit override adds a program key/instruction, so
large virtual-funded routes may then need more ALT coverage. Production should
choose compute/tip policy and current ALTs for its own wallet/request.

## Problems found by actual runs

- The direct USDC/stock fixture exposed overly strict Token-2022 extension
  rejection. Its stock mint defaults to initialized accounts, is not paused,
  and has a disabled transfer hook. The cached adapter now validates those states
  explicitly and permits ordinary public transfers on confidential-capable mints.
  Frozen default accounts, paused mints and active hooks remain rejected; this
  does not add confidential transfers or active-hook extra-account resolution.
- PublicNode's HTTP RPC returned **403** in this environment. gRPC remains
  PublicNode; the cold bootstrap/simulator defaults to the official Solana RPC.
- A simulator RPC can lag behind gRPC, causing `Minimum context slot has not
  been reached` or a future blockhash error. The simulator retries these errors
  at most 12 times with a 750 ms delay. It preserves the minimum slot and gRPC
  blockhash, and does not swallow actual swap errors.
- Free RPC `getTokenLargestAccounts` returned **429**. USDC and KNOTS fixtures now
  use validated unrelated funding accounts; custom cases accept `SIM_TOKEN_SOURCE`.
- Inactive stock pools emitted no pool update during warmup. They retain explicit
  bootstrap provenance while consuming live Clock/blockhash; the example does
  not claim those pool bytes came from a new gRPC update.
- The **V0** USDC three-swap route plus simulator funding was **1,434/1,424 bytes** with
  only the stock ALT. Additional subscribed ALTs and omitting an unnecessary
  simulator compute-budget override brought verified runs to **1,217/1,207 bytes**.
  The default V1 path has a 4,096-byte limit and needs neither workaround nor ALT.

Actual validation on 2026-10-02 used PublicNode gRPC and official Solana RPC.
The raw parser example captured 44 updates in 12 seconds. Graduated SOL/WSOL/USDC
and direct stock quote buys, graduated SOL/WSOL/USDC/stock quote sells, inner SOL buy, and concentrated
venue conversions executed successful mainnet simulations. Every successful run
checked output credit against the protected minimum; no transaction was submitted.
Pool-update counters and bootstrap provenance are recorded in console output.
See [route documentation](STONKFUN_ROUTES.md) for SDK coverage and cache requirements.

Regression: trade library 365 tests passed; both SDK example checks and whitespace
checks passed. Tests that submit real devnet transactions remain intentionally ignored.

V1 was actually simulated on mainnet after switching the example default:
USDC→stock quote→meme used 1,597 bytes with zero ALTs and returned 8,873,489
units (protected minimum 8,607,284); the independent reverse sell used 1,586 bytes
with zero ALTs and returned 4,522 USDC units (minimum 4,386). No transaction was
submitted. V1 wire serialization uses wincode rather than serde/bincode.

All nine V1 mainnet fixture simulations passed, with **zero ALTs**:

The `GRPC_URL`/`GRPC_TOKEN` configuration was also verified against the configured
provider: the parser captured 42 account and 42 BlockMeta events in 12 seconds.
V1 USDC buy passed at slot 452442502 (1,597 bytes, 150,511 CU, output 8,977,397
against minimum 8,708,075); independent SOL sell passed at slot 452442651
(1,233 bytes, 116,305 CU, output 39,768 lamports against minimum 38,574).
Both used zero ALTs and `RPC=None` for quote/build. These inactive meme pools
retained bootstrap account state; live Clock and blockhash came from gRPC.
Another WSOL buy verification passed at simulation slot 452455101 (1,114 bytes,
103,594 CU, output 1,081,659 against minimum 1,049,209), using the configured
provider, zero ALTs and the same RPC-free quote/build path.

Direct USDC/stock/meme V1 simulations also passed with the configured gRPC
provider and zero ALTs. These are separate transactions; simulator funding does
not modify either quoted pool's reserves:

| Direct route | Simulation slot | Wire bytes | CU | Net output | Protected minimum |
| --- | ---: | ---: | ---: | ---: | ---: |
| USDC → stock → meme, 50,000 USDC base units | 452466746 | 1459 | 164272 | 14381694009 | 13950243189 |
| meme → stock → USDC, 100,000,000 meme base units | 452467775 | 1448 | 169418 | 308 | 298 |

The buy used unchanged bootstrap pools plus live Clock/blockhash; the sell
observed one live pool update. Both quoted and built locally with `RPC=None`.
The stock-mint regression covers enabled public transfers and rejection of
frozen/uninitialized default accounts, paused transfers and active hooks.

| Case | Simulation slot | Wire bytes | CU | Net output | Protected minimum |
| --- | ---: | ---: | ---: | ---: | ---: |
| 外盘 SOL 买入 | 452427466 | 1103 | 99544 | 1082427 | 1049954 |
| 外盘 SOL 卖出 | 452427476 | 1233 | 105808 | 39639 | 38449 |
| 外盘 WSOL 买入 | 452427689 | 1114 | 111385 | 1082560 | 1050083 |
| 外盘 WSOL 卖出 | 452427697 | 1225 | 116192 | 39635 | 38445 |
| 外盘 USDC 买入 | 452427448 | 1597 | 151995 | 8873489 | 8607284 |
| 外盘 USDC 卖出 | 452427454 | 1586 | 154846 | 4522 | 4386 |
| 直接股票 quote 买入 | 452427707 | 960 | 65160 | 2375 | 2303 |
| 直接股票 quote 卖出 | 452427713 | 949 | 67401 | 19307644 | 18728414 |
| 内盘 SOL 买入 | 452427483 | 1276 | 126577 | 1348276320 | 1307828031 |

The final parser capture saved 40 account updates and 40 block-metadata records
as valid JSON lines. Quote/build remained RPC-free; unchanged fixture pools
retained the explicitly logged confirmed-bootstrap provenance.

Latest revalidation on 2026-10-02 (current example code): independent V1 USDC
buy passed at slot 452432831, 1,597 wire bytes, 154,996 CU, net output 8,879,220
(minimum 8,612,843). Independent V1 SOL sell passed at slot 452432838, 1,233
wire bytes, 108,811 CU, native credit 39,600 lamports (minimum 38,412). Both had
zero ALTs and no submission. Both examples now support `--help` without opening
a network connection; the raw subscriber validates and deduplicates account
addresses before creating capture files.
