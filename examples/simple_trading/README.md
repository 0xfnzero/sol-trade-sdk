# Simple Trading API

[中文](README_CN.md)

This is the recommended starting point for `SimpleBuyParams`, `SimpleSellParams`, `BuyAmount`, `SellAmount`, account policies, recent blockhashes, and durable nonces.

The example constructs parameters but does not submit the commented buy/sell calls. Set `PRIVATE_KEY` and replace protocol-state placeholders before adapting it to a live integration.

```bash
export RPC_URL=https://your-rpc.example
cargo run --package simple_trading
```

`WithMaxInput` protects maximum quote cost and is generally appropriate when fill priority matters. `ExactInput` protects minimum output and may fail more often in active markets. `HotPathMinimal` requires all needed token accounts to exist; normal integrations should use `AccountPolicy::Auto`.

See [Trading Parameters](../../docs/TRADING_PARAMETERS.md) and [Low-Latency Bots](../../docs/LOW_LATENCY_BOTS.md).
