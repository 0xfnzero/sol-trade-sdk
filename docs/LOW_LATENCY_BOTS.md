# Low-Latency Bot Integration Checklist

Before subscription, initialize and warm `SolanaTrade`, RPC and SWQoS clients, a background blockhash cache or durable nonce pool, known ATAs, and ALTs. Restore signature/instruction deduplication and position state before accepting events.

The event hot path should be limited to:

```text
filter -> deduplicate -> reject stale event -> map post-trade state -> Simple*Params -> sign -> submit
```

Do not initialize clients, synchronously fetch a blockhash, query balances, or search for pools in this path. An RPC fallback is valid for incomplete shred data but is no longer a purely low-latency path.

## Trade intent

| Goal | Parameter |
|---|---|
| Exact spend with minimum-output protection | `BuyAmount::ExactInput` |
| Fill-priority sniping/arbitrage with maximum-cost protection | `BuyAmount::WithMaxInput` |
| Exact token output with maximum-input protection | `BuyAmount::ExactOutput` |
| Sell an exact token amount | `SellAmount::ExactInput` |

`WithMaxInput` still enforces slippage. Never use `min_out = 0` as routine error handling.
Exact-output support is protocol- and pool-direction-specific. PumpSwap exposes exact output through its on-chain `buy` instruction, but its `sell` instruction accepts exact base input plus minimum quote output; the SDK rejects `SellAmount::ExactOutput` when that direction would require `sell`.

Use post-trade event reserves. Preserve PumpFun quote mint, creator/vault, token program, cashback, and mayhem fields. PumpSwap event integrations should use `from_trade_with_fee_basis_points`. Refresh delayed sells because the triggering trade and your own buy both change pool state. Durable nonce extends transaction validity, not quote validity.

For `BuySlippageBelowMinBaseAmountOut`, discard the old transaction, obtain newer reserves and fee rates, enforce a quote-age limit, and rebuild only within a bounded retry policy.
After a submit timeout or ambiguous relay error, reconcile the signature and position before retrying. A retry policy may rebuild quotes automatically only when the previous transaction is known not to have been submitted.

Reference examples:

- `fnzero-examples/pumpfun_grpc_sniper`
- `fnzero-examples/pumpfun_shredstream_sniper`
- `examples/pumpswap_trading`
