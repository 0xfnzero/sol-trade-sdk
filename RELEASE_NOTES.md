# sol-trade-sdk v8.0.0

Shared stock catalog, cached quote/routing improvements, and native-language examples.

## Changes

- Share the parser SDK's screened stock catalog and fee preference; retain StonkFun naming and protocol-specific route legs.
- Improve cached conversion/routing helpers and quote validation. Native examples and regression tests cover stock conversion and signed transaction replays.
- Organize examples and documentation; tests use the SDK's own language. Offline captured transactions do not represent fresh mainnet execution or production latency measurements.

## Migration

Review new quote/routing configuration fields when constructing public structs directly. Prefer constructors/defaults and named limits. Rust parser-adapter uses parser 0.8.0; Rust examples use streamer 4.0.0.

No RPC is performed by the parsing hot path. Explicit analysis/discovery tools can fetch RPC data outside that path. Wrapper reconstruction remains opt-in and is not proof of future third-party program behavior.
