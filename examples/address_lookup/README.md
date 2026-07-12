# Address Lookup Table

[中文](README_CN.md)

Shows how to fetch an Address Lookup Table (ALT), attach it to a PumpFun transaction, and submit after receiving a `sol-parser-sdk` gRPC event.

> This is a live-transaction template. Set `PRIVATE_KEY`, replace `use_your_lookup_table_key_here`, and configure RPC and gRPC before running.

```bash
export GRPC_ENDPOINT=https://your-yellowstone.example
export GRPC_AUTH_TOKEN=optional-token
cargo run --package address_lookup
```

An ALT reduces message size but does not make stale pool state valid. Fetch and cache known ALTs before the event hot path. The current example initializes the trading client after an event and is therefore an API demonstration, not the recommended low-latency structure.

See [Address Lookup Tables](../../docs/ADDRESS_LOOKUP_TABLE.md).
