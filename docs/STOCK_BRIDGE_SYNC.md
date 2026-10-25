# Shared stock bridge catalog (2026-10-22, unreleased)

All eleven SDKs align with the screened sol-parser-sdk / sol-trade-sdk catalog:
33 official stock/ETF mints and 58 stock pools, comprising 43 USDC pairs and
15 WSOL pairs. These are historical identities and reserve/fee observations,
not live quotes. Admission used at least $500 of liquid-side reserve and usable
bidirectional probes. Retain all admitted alternatives; the historical preferred
pool does not force the live route.

Trade registries include the 58 stock pools plus WSOL/USDC (59 total). Custom
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
registering 59 pools does not insert 59 swaps or create all their ATAs. Reusable
intermediate ATAs use idempotent setup. Build fresh snapshots from background
account subscriptions and invalidate them on continuity loss. Configuration,
quote/search and preparation perform no RPC; no cache miss triggers a fallback.

Parser SDKs own parsing, stock monitoring and historical identities. Streamer and
Shred transport signals; incomplete Shred observations never become confirmed
prices or executable reserves. Trade SDKs own quoting and transaction preparation.
The router reuses parser 0.10.0 and trade 10.0.0; its re-exported PumpSwapRouting
builds trade-sdk direct instructions, not automatic router-program CPI dispatch.
Single-market conversion cannot resolve PumpSwapAutoRoute without a routing cache.

## Pool removals on 2026-10-22

The reserve refresh at 04:25 UTC+8 valued WSOL at 107.991809 USDC using the
Orca SOL/USDC reference pool. Five specific pools were removed from defaults:

- ABNB.US / USDC (Raydium CLMM): `52PqYh77gmA6AbM5Hs6gHe9mhGBey5CwmD7sDybaYj27` — a successful removal transaction reduced both pool vaults by more than 50%.
- CHWY.US / WSOL (Meteora DLMM): `6E12HKuZkyQoMwhTtN4WFs95uMEoW2zXJMFSE4jNcjHE` — the same withdrawal criterion, and only $290.53 of liquid-side reserve at the refresh.
- ABNB.US / WSOL (Meteora DLMM): `Bu2A9LUZaMRnf7ZyC1nzREn6dVGVVuPwxz5aFBFkJLTa` — $443.49 of liquid-side reserve.
- BABA.US / USDC (Orca): `8YU7gLMh4T8RuGepxAuigTqZeGfBn3TDS6tnuj951XbW` — $122.19 of liquid-side reserve.
- COPX.US / USDC (Raydium CLMM): `5vJcDWLBQSh767DzHACeEmfMh7nGCWZc8eb76v6cCFVr` — $488.46 of liquid-side reserve.

COPX.US has no retained pool, so it also leaves the default mint list. Other
qualifying pools for ABNB.US, CHWY.US and BABA.US remain. Raw DEX parsing is
unchanged; the bundled historical ABNB price replay now emits no default stock
price for its removed pool. Explicit custom catalogs remain available.

The current regeneration input is `stock-catalog-20261022.json` in
sol-parser-sdk's `docs/research/`, with exact balances, slots, rejection reasons
and withdrawal signatures. Use this reviewed input instead of the older capture.
Fee and bidirectional quote evidence remains historical; this refresh is not a
new executable-depth or LP-lock certification. Defaults are static and do not
automatically enforce a live $500 floor. No RPC or historical scan was added to
SDK hot paths.

## Keeping catalogs aligned

From the eleven-repository workspace root:

```sh
cargo run --manifest-path sol-parser-sdk/Cargo.toml --example stock_catalog_sync -- \
  --report sol-parser-sdk/docs/research/stock-catalog-20261022.json \
  --fee-audit sol-trade-sdk/docs/research/bridge-defaults-20261020-expanded-scan.json \
  --parser-root sol-parser-sdk --trade-root sol-trade-sdk --workspace-root . --check
```

Omit `--check` to regenerate the nine packaged JSON copies. Streamer and router
reuse the parser catalog through dependencies instead of storing duplicate data.
The check validates saved screening evidence and byte equality without RPC.
