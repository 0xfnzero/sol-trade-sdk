# Shared Infrastructure

[中文](README_CN.md)

Demonstrates sharing one `TradingInfrastructure` across multiple wallet-specific `TradingClient` values. This reduces connection, memory, and initialization overhead in multi-wallet services.

Replace the RPC, SWQoS credentials, and all wallet placeholders in `src/main.rs`. The example initializes clients and prints sharing information; it does not submit a trade.

```bash
cargo run --package shared_infrastructure
```

Only share infrastructure when wallets use the same RPC, commitment, and submission routes. Keep wallet signing and position state isolated.
