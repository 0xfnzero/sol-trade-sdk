# Pre-Buy Risk Gate

[中文](PRE_BUY_RISK_GATE_CN.md) | English

The pre-buy risk gate lets your strategy reject a buy before the SDK builds or submits its transaction. Use it to apply cached mint-authority, freeze-authority, holder-concentration, insider-cluster, allowlist, or blocklist decisions without adding a network request to the trade hot path.

## What it does

`TradingClient::buy` processes a request in this order:

```text
validate request -> run TradeRiskGate -> build instructions -> sign -> submit
```

If `TradeRiskGate::check_buy` returns an error, `buy` returns that error immediately. The SDK does not construct or submit the transaction, and no transaction signature is produced.

The gate:

- is optional and disabled by default;
- applies to both `buy` and `buy_simple`;
- runs for simulations as well as live buys;
- does not run for sells;
- does not fetch risk data or maintain a risk cache for you.

The last point is intentional. Your application owns the data source, refresh schedule, expiry policy, and decision rules. The SDK only provides the final synchronous decision point before a buy.

## Recommended architecture

Fetch RPC or audit-provider data in a background task, build an immutable risk snapshot, and atomically publish that snapshot to the gate. The trade thread then performs only a local lookup.

```text
RPC / audit API -> background refresher -> immutable local snapshot
                                               |
market event -> build buy params -> risk gate lookup -> build/sign/submit
```

Do not call RPC or a remote audit API directly from `check_buy`. The method is synchronous because it executes inline on the latency-sensitive buy path.

## Quick start

Add the SDK and `arc-swap` to your application. `arc-swap` allows readers to access the current immutable snapshot without taking a lock while a background task publishes replacements.

```toml
[dependencies]
anyhow = "1"
arc-swap = "1.7"
sol-trade-sdk = "5"
solana-sdk = "3"
```

The following snippets are illustrative and are intended to be combined inside your application's setup and refresh functions. Define the cached verdicts and gate:

```rust
use arc_swap::ArcSwap;
use sol_trade_sdk::{TradeBuyParams, TradeRiskGate, TradingClient};
use solana_sdk::pubkey::Pubkey;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy)]
enum RiskVerdict {
    Allow,
    Deny,
}

type RiskSnapshot = HashMap<Pubkey, RiskVerdict>;

struct CachedRiskGate {
    snapshot: Arc<ArcSwap<RiskSnapshot>>,
    fail_closed_on_miss: bool,
}

impl TradeRiskGate for CachedRiskGate {
    fn check_buy(&self, params: &TradeBuyParams) -> anyhow::Result<()> {
        match self.snapshot.load().get(&params.mint) {
            Some(RiskVerdict::Allow) => Ok(()),
            Some(RiskVerdict::Deny) => {
                anyhow::bail!("risk gate rejected mint {}", params.mint)
            }
            None if self.fail_closed_on_miss => {
                anyhow::bail!("risk verdict missing for mint {}", params.mint)
            }
            None => Ok(()),
        }
    }
}

let risk_snapshot = Arc::new(ArcSwap::from_pointee(RiskSnapshot::new()));
let risk_gate = Arc::new(CachedRiskGate {
    snapshot: risk_snapshot.clone(),
    fail_closed_on_miss: true,
});

let client: TradingClient = client.with_risk_gate(risk_gate);
```

Publish refreshed data from a background task or event consumer. Replacing the snapshot automatically affects all clients that share the gate; you do not need to rebuild the clients.

```rust
let mut next_snapshot = RiskSnapshot::new();
next_snapshot.insert(safe_mint, RiskVerdict::Allow);
next_snapshot.insert(risky_mint, RiskVerdict::Deny);
risk_snapshot.store(Arc::new(next_snapshot));
```

Then submit buys normally:

```rust
match client.buy_simple(buy_params).await {
    Ok(result) => {
        // The request passed the gate and reached normal trade execution.
        handle_trade_result(result);
    }
    Err(error) => {
        // This includes risk rejection and normal validation/build errors.
        handle_rejected_buy(error);
    }
}
```

The snippets use application-specific placeholders such as `client`, `safe_mint`, `risky_mint`, `buy_params`, and the result handlers. Replace them with values from your trading application.

The [`dexra_risk_gate`](../examples/dexra_risk_gate/README.md) example is a complete gate of this shape whose snapshot is refreshed from a hosted audit API.

## Cache policy

A snapshot should contain enough metadata for your application to decide whether it is still trustworthy. Common metadata includes the source slot, fetch time, expiry time, provider version, and the checks that produced the verdict.

Choose the behavior for missing or stale entries explicitly:

| Policy | Behavior | Suitable for |
|---|---|---|
| Fail closed | Reject when no fresh verdict exists | Safety-first execution, new or unknown mints |
| Fail open | Continue when no verdict exists | Availability-first execution where another control covers the risk |
| Allowlist | Only explicitly approved mints pass | Restricted strategies and managed token universes |
| Blocklist | Only explicitly denied mints fail | Broad token coverage with independent monitoring |

Do not silently treat a stale `Allow` verdict as fresh. A background refresher can publish a snapshot that marks stale entries as denied, removes them so the miss policy applies, or switches the gate into a global unavailable state.

## Hot-path requirements

The SDK's disabled path adds one `Option` check. When a gate is installed, the SDK borrows it and calls `check_buy`; it does not clone the gate, allocate a future, take a lock, or await.

Your gate implementation determines the remaining cost. For low latency:

- read only local memory in `check_buy`;
- publish immutable snapshots instead of mutating a shared map entry by entry;
- keep logging, serialization, RPC, HTTP, and database work in the refresher;
- avoid allocating on the allowed path;
- record detailed rejection diagnostics outside the critical path when possible.

Returning an `anyhow::Error` on rejection may allocate, which is acceptable because the transaction is being stopped. The frequently executed allowed path should return `Ok(())` without allocation.

## API reference

### `TradeRiskGate`

```rust
pub trait TradeRiskGate: Send + Sync {
    fn check_buy(&self, params: &TradeBuyParams) -> Result<(), anyhow::Error>;
}
```

The `Send + Sync` bounds allow one gate to be shared by concurrent trading clients. `params` exposes the mint, DEX, amount, quote-token type, slippage, protocol parameters, and other buy settings needed by strategy-specific rules.

### `TradingClient::with_risk_gate`

```rust
pub fn with_risk_gate(self, risk_gate: Arc<dyn TradeRiskGate>) -> TradingClient;
```

This builder-style method returns the configured client. Cloning that client also clones the gate's `Arc`, so every clone observes the same underlying cache.

## What it does not protect against

The gate is one decision point, not a complete trading security system. It does not replace:

- slippage and exact-input/exact-output limits;
- fresh pool reserves and fee data;
- transaction simulation when your strategy requires it;
- MEV-aware submission and priority-fee configuration;
- post-submit signature and position reconciliation;
- ongoing monitoring after a previously allowed token changes state.

See the [Low-Latency Bot Integration Checklist](LOW_LATENCY_BOTS.md) for the surrounding event, state-freshness, blockhash, account, and submission workflow.
