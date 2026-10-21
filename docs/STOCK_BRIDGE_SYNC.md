# Shared stock bridge catalog (2026-10-21, unreleased)

All eleven SDKs align with the screened sol-parser-sdk / sol-trade-sdk catalog:
34 official stock/ETF mints and 63 stock pools, comprising 46 USDC pairs and
17 WSOL pairs. These are historical identities and reserve/fee observations,
not live quotes. Admission used at least $500 of liquid-side reserve and usable
bidirectional probes. Retain all admitted alternatives; the historical preferred
pool does not force the live route.

Trade registries include the 63 stock pools plus WSOL/USDC (64 total). Custom
pools merge by address; reverse pairs and SOL/WSOL aliases deduplicate. Conflicting
identities reject the entire merge. The bound is 128 unique registered pools,
with a default search budget of 512 nodes and two conversion hops (excluding
the target PumpSwap hop). Custom-only configurations remain supported.

Each conversion hop defaults to 100 bps execution slippage and 100 bps maximum
price impact against a 1/100-size cached probe. These are independent user settings;
the target trade has its own slippage. Integer arithmetic accounts for net output;
unusable probes fail closed. No guarantee of a 1% total-route loss is implied.
A chosen independent buy or sell is built as one transaction, subject to packet,
account, compute and CPI limits; it is never silently split.

Search starts at the requested payment/receipt asset. USDC input considers direct
USDC/stock pools without first wrapping SOL. All valid paths rank by protected net
output, then fewer hops and deterministic pool order. Paths never revisit a mint.
Only the selected route contributes swap/ATA instructions to the transaction;
registering 64 pools does not insert 64 swaps or create all their ATAs. Reusable
intermediate ATAs use idempotent setup. Build fresh snapshots from background
account subscriptions and invalidate them on continuity loss. Configuration,
quote/search and preparation perform no RPC; no cache miss triggers a fallback.

Parser SDKs own parsing, stock monitoring and historical identities. Streamer and
Shred transport signals; incomplete Shred observations never become confirmed
prices or executable reserves. Trade SDKs own quoting and transaction preparation.
The router reuses parser 0.8.0 and trade 8.0.0; its re-exported PumpSwapRouting
builds trade-sdk direct instructions, not automatic router-program CPI dispatch.
Single-market conversion cannot resolve PumpSwapAutoRoute without a routing cache.

## Keeping catalogs aligned

From the eleven-repository workspace root:

```sh
cargo run --manifest-path sol-parser-sdk/Cargo.toml --example stock_catalog_sync -- \
  --report sol-parser-sdk/docs/research/stock-catalog-20261020.json \
  --fee-audit sol-trade-sdk/docs/research/bridge-defaults-20261020-expanded-scan.json \
  --parser-root sol-parser-sdk --trade-root sol-trade-sdk --workspace-root . --check
```

Omit `--check` to regenerate the nine packaged JSON copies. Streamer and router
reuse the parser catalog through dependencies instead of storing duplicate data.
The check validates saved screening evidence and byte equality without RPC.
