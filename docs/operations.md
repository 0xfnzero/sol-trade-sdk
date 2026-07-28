# Operations Design — sol-trade-sdk

> **Reference**: Sections 43, 44, 47 of the pre-flight requirements
> **Scope**: Shadow mode, canary gate, monitoring, runbook planning
> **Status**: Design — implementation pending

---

## 1. Shadow Mode Gate

### Reference: Section 43

Shadow mode is a read-only deployment that receives the full live ShredStream, reconstructs events, maintains internal state, and produces trade intents — but does **not** sign or submit any transactions. It serves as a validation layer before real capital is deployed.

### Architecture

```mermaid
flowchart LR
    subgraph Live [Mainnet Production]
        SS[ShredStream\n(gRPC)]
    end

    subgraph Shadow [Shadow Instance]
        EC[Event Consumer]
        SM[State Machine]
        IP[Intent Producer]
        HM[Hypothetical Matcher]
        TP[Trade Pipeline\n(read-only)]
    end

    subgraph Observability
        M[Metrics\nPrometheus]
        L[Latency Tracker]
        FP[False Positive\nAnalyzer]
    end

    SS --> EC
    EC --> SM
    SM --> IP
    IP --> TP
    TP -->|no signing| HM
    HM --> M
    HM --> L
    HM --> FP
```

### Shadow Instance Configuration

```toml
[environment]
mode = "shadow"
name = "shadow-nyc-01"

[risk]
signing_enabled = false
submit_enabled = false

[wallet]
# Read-only keypair — only used for account lookups, not signing
keypair_path = "/etc/solbot/shadow/wallet.key"
signing_allowed = false

[submission]
# No submission in shadow mode — all providers disabled
providers = []

[shredstream]
enabled = true
# Same feed as production
endpoint = "mainnet.rpc.shredstream.example.com:443"
```

### Shadow Metrics

| Metric | Description | Alert Threshold |
|--------|-------------|-----------------|
| `shadow.events.processed` | Total ShredStream events processed | Track trend |
| `shadow.intents.produced` | Hypothetical trade intents | N/A (informational) |
| `shadow.state.slot` | Current slot | N/A |
| `shadow.false_positives.count` | Intents that would have been rejected | >10/hour |
| `shadow.latency.p50` | Event → intent latency (p50) | >100ms |
| `shadow.latency.p99` | Event → intent latency (p99) | >500ms |
| `shadow.state.reconstruction_gap` | Missing slot count | >5 |
| `shadow.opportunities.captured` vs `shadow.opportunities.simulated` | Known-opportunity match rate | <90% |

### Shadow Validation Criteria

Before canary deployment, shadow must demonstrate:
1. Event detection accuracy ≥ 99.5% (known events detected)
2. False positive rate ≤ 1% (intents produced that are not real opportunities)
3. State reconstruction gap ≤ 5 slots in any 24h window
4. Event → intent latency p50 ≤ 100ms, p99 ≤ 500ms
5. Successful run of ≥ 72 hours without crash or restart

---

## 2. Canary Gate

### Reference: Section 44

The canary deployment phases incrementally increase real-money exposure. Each phase has explicit go/no-go criteria.

### 2.1 Phase 1 — Manual Transaction

**Duration**: 1-2 days
**Setup**: Canary instance configured identical to production, but bot does not auto-trade
**Process**:
- Operator manually triggers individual trades via `solbot trade --dry-run=false`
- Each trade is pre-flighted with `simulateTransaction`
- Trades are signed and submitted using production SWQOS providers
- Confirmation latency, success rate, and fee cost are recorded

**Go/No-Go Criteria**:
- ✅ Manual trades confirm within expected window (≤ 10s)
- ✅ Submission success rate ≥ 95%
- ✅ Fee cost within expected range
- ✅ No unexpected errors or edge cases found

### 2.2 Phase 2 — Automatic Transaction (Single Trade Type)

**Duration**: 2-3 days
**Setup**: Bot configured to auto-trade one protocol (e.g., PumpFun buys only)
**Process**:
- Bot operates autonomously for one protocol direction
- Position tracking is validated against on-chain state
- Risk limits are enforced (max 0.1 SOL/trade, 10 trades/day)
- All trades are logged with full trace context

**Go/No-Go Criteria**:
- ✅ ≥ 50 automatic trades executed
- ✅ Trade success rate ≥ 90%
- ✅ No position tracking discrepancies
- ✅ Latency within performance budget

### 2.3 Phase 3 — Limited Session

**Duration**: 3-7 days
**Setup**: Bot operates all protocols, all directions, with strict volume caps
**Process**:
- All protocols enabled
- Max 1 SOL/day total trade volume
- Session window: 4 hours/day (e.g., 10:00-14:00 UTC)
- Full monitoring and alerting active

**Go/No-Go Criteria**:
- ✅ ≥ 200 trades executed across all protocols
- ✅ P&L within expected range (no unexplained losses)
- ✅ All stop conditions tested via simulated injection
- ✅ Recovery from process restart verified

### 2.4 Phase 4 — Sustained Operation

**Duration**: 7-14 days
**Setup**: Full trading parameters, 24/7 operation
**Process**:
- Full risk limits apply (as configured for production)
- All strategies enabled
- 24/7 operation

**Go/No-Go Criteria**:
- ✅ ≥ 7 consecutive days without unplanned stop
- ✅ P&L within ±10% of benchmark
- ✅ No security incidents
- ✅ Operator handoff procedure validated

### 2.5 Phase 5 — Restart Recovery

**Duration**: 3 days
**Setup**: Simulate crashes and restarts during Phase 4
**Process**:
- Operator triggers `systemctl restart solbot` during active trading
- Operator kills the process with SIGKILL during submission
- Operator simulates VPS reboot
- Verify state reconstruction, pending tx recovery, and nonce management

**Go/No-Go Criteria**:
- ✅ All restart scenarios recover without data loss
- ✅ No double-spends from restored pending transactions
- ✅ Recovery completes within 30 seconds of bot restart

### 2.6 Phase 6 — Provider Failure

**Duration**: 2 days
**Setup**: Simulate SWQOS/RPC provider failures during Phase 4
**Process**:
- Operator disables Jito provider — verify automatic failover
- Operator disables all but one provider — verify reduced throughput
- Operator introduces RPC latency > 5s — verify timeout handling
- Operator returns failed providers — verify automatic recovery

**Go/No-Go Criteria**:
- ✅ Automatic failover works for each provider pair
- ✅ Submission continues without gap (may be slower)
- ✅ Recovered providers rejoin automatically

### 2.7 Phase 7 — Feed Interruption

**Duration**: 2 days
**Setup**: Simulate ShredStream disconnection
**Process**:
- Operator disconnects ShredStream for 5s, 30s, 5min
- Operator corrupts event stream (malformed data)
- Operator replays stale events

**Go/No-Go Criteria**:
- ✅ Event queue recovers after reconnection
- ✅ Stale events are correctly rejected
- ❌ No phantom trades from replayed events
- ✅ State catch-up completes within 2x gap duration

### 2.8 Phase 8 — Scale

**Duration**: 7 days
**Setup**: Production-scale configuration
**Process**:
- Full wallet, full risk limits, all strategies
- 24/7 operation
- All monitoring and alerting active
- Operator on-call rotation active

**Go/No-Go Criteria**:
- Production declaration after 7 days of stable operation
- ✅ All SLAs met
- ✅ Runbook validated for every documented scenario
- ✅ Post-mortem completed for any incidents

### Canary Progression Gate

```mermaid
stateDiagram-v2
    [*] --> Shadow: Deploy shadow instance
    Shadow --> Manual: 72h shadow validation
    Manual --> Auto: Go/No-Go Phase 1
    Auto --> Limited: Go/No-Go Phase 2
    Limited --> Sustained: Go/No-Go Phase 3
    Sustained --> RestartRecovery: Go/No-Go Phase 4
    RestartRecovery --> ProviderFailure: Go/No-Go Phase 5
    ProviderFailure --> FeedInterruption: Go/No-Go Phase 6
    FeedInterruption --> Scale: Go/No-Go Phase 7
    Scale --> Production: Go/No-Go Phase 8
    Production --> [*]: Full production operation
    Production --> Rollback: Any P0 incident
    Rollback --> Shadow: Rollback to shadow mode
```

---

## 3. Monitoring Design

### 3.1 Metrics (Prometheus Endpoint)

The bot exposes a `/metrics` HTTP endpoint with the following metric families:

**Trading**
- `solbot_trades_initiated_total{protocol, direction, provider}` — Counter
- `solbot_trades_confirmed_total{protocol, direction, status}` — Counter
- `solbot_trades_latency_ms{protocol, direction}` — Histogram (50ms buckets)
- `solbot_trades_value_sol{protocol, direction}` — Histogram
- `solbot_trade_errors_total{protocol, error_type}` — Counter

**Submission**
- `solbot_submissions_total{provider, status}` — Counter
- `solbot_submission_latency_ms{provider}` — Histogram
- `solbot_submission_queue_depth` — Gauge
- `solbot_submission_queue_timeout_total{provider}` — Counter

**ShredStream**
- `solbot_shredstream_events_total` — Counter
- `solbot_shredstream_latency_ms` — Histogram
- `solbot_shredstream_reconnects_total` — Counter
- `solbot_shredstream_slot_gap` — Gauge

**System**
- `solbot_memory_bytes` — Gauge
- `solbot_cpu_usage_ratio` — Gauge
- `solbot_open_fds` — Gauge
- `solbot_uptime_seconds` — Gauge

**Wallet**
- `solbot_wallet_balance_sol{token}` — Gauge
- `solbot_wallet_position_count` — Gauge

**Risk**
- `solbot_risk_stop_conditions_triggered_total{condition}` — Counter
- `solbot_risk_daily_loss_sol` — Gauge
- `solbot_risk_position_limit_remaining` — Gauge

### 3.2 Alerting Rules

| Alert | Condition | Severity | Runbook |
|-------|-----------|----------|---------|
| BotStopped | `solbot_uptime_seconds == 0` | Critical | emergency-shutdown |
| SubmissionFailureRate | `rate(solbot_submissions_total{status="failed"}[5m]) > 0.1` | Critical | provider-failure |
| TradeFailureRate | `rate(solbot_trades_confirmed_total{status="failed"}[5m]) > 0.2` | Warning | ambiguous-submission |
| ShredStreamDisconnected | `solbot_shredstream_reconnects_total[1h] > 5` | Critical | packet-loss |
| WalletLowBalance | `solbot_wallet_balance_sol < 1.0` | Critical | emergency-shutdown |
| HighMemory | `solbot_memory_bytes > 6e9` (6GB) | Warning | process-crash |
| SlotDrift | `solbot_shredstream_slot_gap > 5` | Warning | stale-state |
| DailyLoss | `solbot_risk_daily_loss_sol > threshold` | Critical | emergency-shutdown |
| QueueBackup | `solbot_submission_queue_depth > 100` | Warning | rpc-failure |

### 3.3 Health Endpoint

```
GET /health

Response:
{
  "status": "ok" | "degraded" | "stopped",
  "uptime_seconds": 123456,
  "current_slot": 284512345,
  "shredstream_connected": true,
  "rpc_connected": true,
  "providers_active": 4,
  "queue_depth": 5,
  "last_trade_seconds_ago": 12,
  "stop_reason": null
}
```

### 3.4 Distributed Tracing (OpenTelemetry)

- Trace every trade end-to-end: event → intent → submission → confirmation
- Service name: `sol-trade-sdk`
- Trace sampling: 100% during canary, 10% in production (100% for errors)
- Export to configured OTel collector or Honeycomb/Datadog

---

## 4. Runbook Planning

### 4.1 Required Runbooks

The following runbooks must be created and stored in `runbooks/`:

| Runbook | Trigger | Priority | Owner |
|---------|---------|----------|-------|
| `packet-loss.md` | ShredStream packet loss / disconnection | High | Platform |
| `rpc-failure.md` | RPC primary + fallback unreachable | Critical | Platform |
| `provider-failure.md` | SWQOS provider failures | High | Platform |
| `ambiguous-submission.md` | Transaction submitted but confirmation unknown | Critical | Trading |
| `duplicate-position.md` | Same position opened twice | Critical | Trading |
| `stale-state.md` | Bot state diverges from on-chain state | Critical | Trading |
| `secret-exposure.md` | Wallet key or API key potentially exposed | Critical | Security |
| `protocol-upgrade.md` | Protocol IDL/address change detected | High | Trading |
| `process-crash.md` | Bot process exits unexpectedly | High | Platform |
| `disk-pressure.md` | Disk space critically low | High | Platform |
| `emergency-shutdown.md` | Any condition requiring immediate stop | Critical | All |

### 4.2 Runbook Template

Each runbook follows this structure:
```
# Runbook: <name>

## Overview
Brief description of the scenario.

## Detection
How this condition is detected (metrics, logs, alerts).

## Severity
Critical / High / Medium / Low

## Immediate Action (0-5 minutes)
Step-by-step containment actions.

## Investigation (5-30 minutes)
Diagnostic steps to determine root cause.

## Resolution
Steps to restore normal operation.

## Recovery Verification
How to confirm the system is healthy.

## Post-Mortem
Data to collect for post-incident analysis.

## References
Links to related runbooks, dashboards, and documentation.
```

### 4.3 Runbook Automation

For any runbook step that can be automated:
1. Implement as a CLI subcommand: `solbot ops <action>`
2. Expose as a health endpoint: `POST /ops/<action>`
3. Integrate with runbook tool (e.g., FireHydrant, PagerDuty Runbook)

---

## 5. Logging Design

### 5.1 Log Format

```json
{
  "timestamp": "2026-07-28T12:34:56.789Z",
  "level": "INFO",
  "target": "sol_trade_sdk::trading",
  "message": "Trade submitted",
  "fields": {
    "protocol": "pumpfun",
    "direction": "buy",
    "amount_sol": 0.1,
    "signature": "5KL...",
    "provider": "jito",
    "latency_ms": 45
  },
  "span": {
    "trade_id": "abc-123",
    "trace_id": "xyz-456"
  }
}
```

### 5.2 Log Levels by Environment

| Environment | Default Level | SDK Level | Streamer Level |
|-------------|---------------|-----------|----------------|
| Development | debug | debug | debug |
| Test | info | debug | debug |
| Shadow | info | debug | info |
| Canary | info | info | info |
| Production | warn | info | warn |

### 5.3 Sensitive Data Redaction

The following fields are automatically redacted from logs:
- `api_key`, `token`, `secret`, `password`, `keypair` — replaced with `***`
- `signature` — first 8 chars only (e.g., `5KL...abc`)
- `wallet` — first 4 chars only (e.g., `ABC...xyz`)
- Transaction data — hex dump only, no decoded instructions