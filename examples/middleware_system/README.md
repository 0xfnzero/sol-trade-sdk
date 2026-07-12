# Instruction Middleware

[中文](README_CN.md)

Demonstrates implementing `InstructionMiddleware`, installing it in a `MiddlewareManager`, and observing or modifying instructions before submission.

> The example loads `PRIVATE_KEY` and calls `client.buy`. After configuration it can submit a real PumpSwap mainnet transaction.

```bash
cargo run --package middleware_system
```

Middleware must preserve signer, account-order, compute-budget, tip, nonce, and transaction-size semantics. Keep latency-sensitive middleware deterministic and free of blocking I/O.
