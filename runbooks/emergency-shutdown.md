# Runbook: Emergency Shutdown

## Overview

Immediate, controlled shutdown of the sol-trade-sdk bot triggered by any condition that could cause financial loss, security breach, or systemic failure. This is the final escalation for stop conditions that cannot be resolved via other runbooks.

## Detection

- **Alert**: `BotStopped` (critical) — `solbot_uptime_seconds == 0`
- **Monitoring**: Prometheus alert `EmergencyShutdownTriggered` fires
- **Health**: `/health` returns `{"status": "stopped", "stop_reason": "..."}`
- **Logs**: ERROR-level log `Stop condition triggered: <condition_name>`
- **Active triggers**: Any of the conditions listed in [docs/deployment.md §4](../docs/deployment.md#4-production-stop-conditions):
  1. RPC primary + all fallbacks unreachable (>N consecutive)
  2. Blockhash service unavailable (>5s)
  3. Slot drift (>5 slots)
  4. ShredStream disconnected (reconnect timeout)
  5. All SWQOS providers failing (>N consecutive)
  6. Submission queue overflow (not clearing)
  7. Confirmation timeout (>N consecutive)
  8. Duplicate signature across providers
  9. Cumulative daily loss > threshold (default: 10%)
  10. Single trade loss > max loss per trade
  11. Wallet balance below minimum operating threshold
  12. Position limit exceeded
  13. Memory usage > configured limit
  14. Disk space < 10%
  15. Clock skew > 5s
  16. Config file changed during runtime
  17. IDL hash mismatch (protocol upgrade)
  18. simulateTransaction unexpected error
  19. Unsupported token extension on hot path
  20. Secret/key exposure
  21. Unauthorized config access
  22. Sudden unexpected wallet activity

## Severity

**Critical** — requires immediate operator attention.

## Immediate Action (0-5 minutes)

### 1. Verify Shutdown

Ensure the bot has stopped:

```bash
# Check process status
systemctl status solbot

# If still running, force stop
systemctl stop solbot

# Confirm no zombie processes
pgrep -a sol-trade-bot
```

### 2. Identify Stop Reason

```bash
# Read the last stop reason from journal
journalctl -u solbot --since "5 minutes ago" -p err | grep "Stop condition"
# OR
journalctl -u solbot --since "5 minutes ago" | grep -i "stop condition\|fatal"

# Check the stop reason file (persisted by the bot)
cat /var/log/solbot/stop_reason
```

### 3. Assess Financial Impact

```bash
# Check current wallet balance
solbot wallet balance --config /etc/solbot/production.toml

# Check pending transactions
solbot status pending --config /etc/solbot/production.toml

# Review recent P&L
solbot report pnl --last 24h --config /etc/solbot/production.toml
```

### 4. Contain Exposure (if applicable)

- If the stop was due to **secret exposure** (condition 20): immediately rotate keys.
- If the stop was due to **financial loss** (conditions 9-11): consider transferring remaining funds to a cold wallet.
- If the stop was due to **position limit** (condition 12): verify no open positions remain.

```bash
# Emergency fund transfer (if needed)
solbot ops transfer-funds --recipient <SAFE_WALLET> --amount all --config /etc/solbot/production.toml
```

### 5. Notify

- **Primary**: PagerDuty / Opsgenie (automatically triggered by alert)
- **Secondary**: Slack #ops-critical channel
- **Message template**:
  ```
  EMERGENCY SHUTDOWN: <bot_instance>
  Reason: <stop condition>
  Time: <timestamp>
  Impact: <financial impact assessment>
  Action taken: <what was done>
  ```

## Investigation (5-30 minutes)

### 1. Collect Evidence

```bash
# Save journal logs since shutdown
journalctl -u solbot --since "1 hour ago" > /tmp/solbot_shutdown_$(date +%Y%m%d_%H%M%S).log

# Collect configuration
cp /etc/solbot/production.toml /tmp/solbot_config_backup.toml

# Collect state dump (if bot supports it)
solbot ops dump-state --output /tmp/solbot_state_dump.json --config /etc/solbot/production.toml

# Collect metrics snapshot
curl -s http://localhost:9090/metrics > /tmp/solbot_metrics_$(date +%Y%m%d_%H%M%S).txt
```

### 2. Determine Root Cause

Refer to the specific runbook for the triggered condition:
- **[packet-loss.md](packet-loss.md)** — ShredStream issues
- **[rpc-failure.md](rpc-failure.md)** — RPC connectivity
- **[provider-failure.md](provider-failure.md)** — SWQOS provider failures
- **[ambiguous-submission.md](ambiguous-submission.md)** — Transaction status unknown
- **[duplicate-position.md](duplicate-position.md)** — Duplicate trade detection
- **[stale-state.md](stale-state.md)** — State divergence
- **[secret-exposure.md](secret-exposure.md)** — Key/credential exposure
- **[protocol-upgrade.md](protocol-upgrade.md)** — Protocol changes
- **[process-crash.md](process-crash.md)** — Bot crash
- **[disk-pressure.md](disk-pressure.md)** — Storage issues

### 3. Check Dependencies

```bash
# Verify network connectivity
curl -v --max-time 5 <RPC_PRIMARY_URL>
curl -v --max-time 5 <RPC_FALLBACK_URL>

# Check DNS resolution
dig <RPC_ENDPOINT>

# Check system resources
free -h
df -h
ntpq -p
```

## Resolution

### Option A: Fix and Restart (if root cause is transient)

```bash
# Fix the identified issue (e.g., restart RPC proxy)
# ...

# Restart the bot
systemctl start solbot

# Verify it starts correctly
journalctl -u solbot -f --since "30 seconds ago"

# Expected startup log: "solbot ready on slot X"
```

### Option B: Rollback to Previous Version

```bash
# If the issue is a software bug:
# 1. Stop current version
systemctl stop solbot
# 2. Rollback binary
cp /opt/solbot/bin/sol-trade-bot.prev /opt/solbot/bin/sol-trade-bot
# 3. Rollback config
cp /etc/solbot/production.toml.prev /etc/solbot/production.toml
# 4. Start
systemctl start solbot
```

### Option C: Stand Down Completely

If the issue cannot be resolved quickly:

```bash
# Ensure the bot won't auto-restart
systemctl disable solbot

# Lock the config
chmod 000 /etc/solbot/production.toml

# Notify team
# File a P0 incident ticket
```

## Recovery Verification

- [ ] Bot process is running: `systemctl status solbot` shows active
- [ ] Health endpoint returns 200: `curl -s http://localhost:8080/health`
- [ ] ShredStream is connected: `solbot status` shows connected
- [ ] RPC is available: `solbot status` shows RPC online
- [ ] Wallet balance is adequate: `solbot wallet balance`
- [ ] No pending unconfirmed transactions: `solbot status pending`
- [ ] Last trade was processed correctly: check logs
- [ ] All SWQOS providers are active: `solbot status providers`
- [ ] Metrics are flowing: check Prometheus target
- [ ] Alerts are cleared: check PagerDuty / Opsgenie

## Post-Mortem

Collect the following for the post-incident review:

### Required Data
- Stop condition name and timestamp
- Root cause analysis
- Financial impact (if any)
- Time to detection (TTD)
- Time to acknowledgement (TTA)
- Time to resolution (TTR)
- Actions taken and by whom

### Analysis Questions
1. Was the stop condition correctly detected? (false positive / true positive)
2. Did the automatic shutdown work as expected?
3. Were there any gaps in coverage (undetected conditions)?
4. What monitoring/alert improvements could reduce TTD?
5. What automation could reduce TTR?
6. Should this condition be hardened in code (preventive) or in operations (reactive)?

### Artifacts to Attach
- Journal logs from shutdown window
- Configuration at time of incident
- State dump
- Metrics snapshot
- Chat/incident timeline

## References

- [Deployment Architecture — Stop Conditions](../docs/deployment.md#4-production-stop-conditions)
- [Operations — Alerting Rules](../docs/operations.md#32-alerting-rules)
- [Other Runbooks](.) — runbook directory
- PagerDuty Service: `sol-trade-sdk-production`