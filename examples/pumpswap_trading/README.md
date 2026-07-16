# PumpSwap Low-Latency gRPC Example

[中文](README_CN.md)

This example consumes PumpSwap events through `solana-streamer-sdk` and builds one follow-up buy from the event's post-trade reserves and dynamic fee rates. The trading client and blockhash cache are initialized before subscription, so the event hot path does not synchronously fetch a blockhash.

## Run

```bash
cp .env.example .env
# Edit .env, then load it into the current shell:
set -a; source .env; set +a
export PRIVATE_KEY=your_base58_private_key
export RPC_URL=https://your-rpc.example
cargo run --release --package pumpswap_trading
```

`GRPC_ENDPOINT` and `GRPC_AUTH_TOKEN` are optional. `TARGET_MINT` or `TARGET_POOL` is required; when both are set, both must match. `MAX_EVENT_AGE_MS` defaults to 1000. The binary reads environment variables but does not load `.env` itself, so source the file first when using it.

## Trade semantics

- The buy uses `BuyAmount::WithMaxInput`, which applies slippage to maximum quote cost and is appropriate when fill priority matters.
- Buy parameters use post-trade reserves and LP/protocol/creator fee bps from the event.
- The event's raw and virtual quote reserves come from the same transaction snapshot. The hot path does not fetch the Pool account, avoiding both added latency and mixed-slot quotes.
- The first matching event asynchronously records the pre-buy balance; the next fresh event performs the trade without a balance RPC in the submission hot path. It sells only the confirmed balance increase and refreshes pool state and blockhash before selling.
- Use `BuyAmount::ExactInput` when the quote spend must be exact. That mode protects minimum output and can fail more often in an active pool.
- If baseline warmup fails, the example waits for another event. Once transaction execution starts, an error keeps the one-shot guard locked because submission or position state may be uncertain; inspect the signatures and account state before retrying.

Production bots should also add durable signature deduplication, a position state machine, SWQoS configuration, and bounded requoting. Do not solve slippage errors by setting `min_out` to zero.
