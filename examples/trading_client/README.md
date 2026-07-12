# Trading Client Construction

[中文](README_CN.md)

Shows two client-construction patterns: `TradingClient::new` for one wallet and `TradingClient::from_infrastructure` for wallets sharing RPC and SWQoS clients.

This is a configuration template. Set `PRIVATE_KEY`, RPC, and every enabled provider credential; it initializes network clients but does not submit a trade.

```bash
cargo run --package trading_client
```

Initialize the client before subscribing to events. Do not construct it in a trading callback.
