# Trade examples

[中文](README_CN.md) · [Documentation](../docs/README.md)

Run from the repository root. A root `examples/*.rs` target uses
`cargo run --example NAME`; a directory with its own Cargo.toml uses
`cargo run --package NAME`. These are different kinds of Cargo targets.

## Offline replay and preparation

- `stock_conversion_replay`: extract approved conversion pools from six signed
  Stonk/Pump transactions; keeps each leg's DEX identity.
- `pumpswap_streaming_quotes`: captured-account quote replay by default.
- `pumpswap_cached_prepare`, `stonkfun_cached_prepare`: cached instruction preparation.
- `pumpswap_global_routing`: cached multi-hop route selection and construction.
- `pool_quote_tools`, `bridge_pool_depth`: caller-supplied snapshot quotes/depth.
- `pumpswap_hotpath_benchmark`: offline quote-path timing.

```sh
cargo run --features parser-adapter --example stock_conversion_replay
cargo run --features parser-adapter --example pumpswap_streaming_quotes
```

Use `-- --help` for either entry point. The stock replay accepts `--fixture FILE`
with a raw saved RPC response or a cases capture. Historical data is not live state.

## Online quotes and simulations

Quote/depth tools share `support/account_snapshot.rs`. It requires base64 account
data and validates the Clock owner and snapshot slot before creating a cache.
Common input handling belongs under `support/`; strategy and protocol behavior
stay in the named entry points. Keep `StonkFun` and official IDL names intact.

- `pumpswap_streaming_quotes --source SOURCE --live`: collect subscription signals
  without signing or broadcasting. Live quotes require provider-verified bootstrap
  and complete batch publication; offline replay works out of the box.
  Sources: `parser-grpc`, `parser-shred`,
  `streamer-grpc`, `streamer-shred`. Default source: `parser-grpc`.
  See [required state and environment](../docs/STREAMING_QUOTES.md).
- `pumpswap_global_routing_simulate`, `stonkfun_grpc_simulate`,
  `cpmm_*_simulate` and the `stonkfun_via_sol_simulate` package:
  online simulation; see [simulation guide](SIMULATION.md).
- `simulation_replay`: simulate saved wire against the current bank. Unlike
  fixture replay, this explicitly needs RPC; it does not refresh old quotes.
- `cpmm_creator_fee_stream`: fee-specific subscription demonstration.

```sh
cargo run --features parser-adapter --example pumpswap_streaming_quotes -- --source parser-grpc --live
```

Do not run every example as a smoke test: online modes need configured state,
and trading packages may submit transactions.

## Trading packages

Read each directory's README before configuring keys and mainnet accounts.

- Basics: `simple_trading`, `trading_client`, `shared_infrastructure`.
- Event-driven: `pumpswap_trading`, `pumpfun_copy_trading`,
  `pumpfun_sniper_trading`, `bonk_copy_trading`, `bonk_sniper_trading`.
- Protocol-specific: `pumpswap_direct_trading`, `raydium_amm_v4_trading`,
  `raydium_cpmm_trading`, `meteora_damm_v2_direct_trading`.
- Infrastructure: `address_lookup`, `nonce_cache`, `middleware_system`,
  `seed_trading`, `gas_fee_strategy`, `wsol_wrapper`, `cli_trading`.

For production patterns, use [low-latency guidance](../docs/LOW_LATENCY_BOTS.md).
An event callback that fetches RPC state is a protocol demonstration, not the
recommended hot path. Use fresh background account state and independent order sizes.

## Naming and migration

Rust regression tests run with Cargo. Include `parser-adapter` to cover the
optional parser integration and its examples:

```sh
cargo test --lib --tests --features parser-adapter
cargo check --examples --features parser-adapter
cargo test --example pumpswap_streaming_quotes --features parser-adapter
```

Write new regression tests in Rust under `tests/` or `#[cfg(test)]`.
The shared catalog is checked by
the parser SDK's Rust `stock_catalog_sync` example, as documented in
[bridge defaults](../docs/PUMPSWAP_BRIDGE_DEFAULTS.md).
The historical Python fee-audit collector under `docs/research/` is supporting
evidence, not a Rust SDK test dependency.

Names use `protocol_or_domain_subject_action` in snake_case; official protocol
and IDL names stay unchanged. Keep separate protocol validation cases, but share
fixture parsing and transport configuration under `support/`.

- `cross_dex_conversion_candidates` → `stock_conversion_replay`.
- `pumpswap_parser_grpc` / `pumpswap_parser_shred` →
  `pumpswap_streaming_quotes --source parser-grpc|parser-shred`.
- `pumpswap_streamer_grpc` / `pumpswap_streamer_shred` →
  `pumpswap_streaming_quotes --source streamer-grpc|streamer-shred`.

These replacements remove duplicate entry wrappers; all four transports remain.
Cargo features and commands in topic guides use the new targets.
