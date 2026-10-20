# Dexra Risk Gate

[中文](README_CN.md)

A [pre-buy risk gate](../../docs/PRE_BUY_RISK_GATE.md) whose verdicts come from a hosted audit API, [Dexra](https://dexra.dev). It follows the architecture the guide recommends: a background task polls the API and publishes an immutable snapshot, and `check_buy` only reads local memory.

Dexra's launch feed (`GET /new_launches`) returns fresh pump.fun, PumpSwap and LetsBonk launches with a verdict each, `ok`, `caution`, `avoid` or `unknown`, and the reasons behind it: mint and freeze authorities, liquidity and whether it is locked or was pulled, how many of the creator's earlier launches collapsed, and whether the first buyers bought in the launch block, were funded by the creator or by one shared wallet, or are known snipers.

The gate allows a buy only for a mint rated `ok` in a fresh snapshot. `caution`, `avoid`, `unknown`, a mint the feed has not returned, and a snapshot older than three poll intervals all reject. When a mint has several pools (a pump.fun curve and its PumpSwap pool), the worst verdict decides.

```bash
export DEXRA_API_KEY=...        # free key: https://dexra.dev/app/keys
cargo run --package dexra_risk_gate -- <MINT> [<MINT> ...]
```

The example prints each launch as it arrives and, every poll, the gate's decision for the mints on the command line. It does not sign or send transactions. In a bot, attach the same gate to the client:

```rust
let client = SolanaTrade::new(Arc::new(payer), trade_config).await.with_risk_gate(gate.clone());
```

| Variable | Default | |
|---|---|---|
| `DEXRA_API_KEY` | required | API key |
| `DEXRA_POLL_SECS` | `60` | seconds between feed polls |
| `DEXRA_PLATFORMS` | `pump_fun,pump_swap,letsbonk` | also `raydium_launchlab`, `meteora_dbc`, `raydium_amm_v4`, `raydium_cpmm` |
| `DEXRA_LIMIT` | `20` | launches per poll; above 20 needs a paid plan |
| `DEXRA_URL` | `https://api.dexra.dev` | API base URL |

Each poll is one paid call. A free key covers about a hundred a day, so poll every 15 minutes or so on it; polling every minute needs a plan (https://dexra.dev/pricing). Without a key the API answers `402` with an [x402](https://x402.org) USDC payment challenge instead.

Timing: the creator's history is known from a launch's first second, while the first-buyer analysis completes seconds to a minute after launch, and until it does the verdict is usually `unknown`. The gate suits strategies that buy after the first trades, not in the creation block. For mints you already hold or found elsewhere, `POST /new_launches/batch` with `{"mints": [...]}` returns the same checks.
