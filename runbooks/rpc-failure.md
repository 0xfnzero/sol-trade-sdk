# Runbook: RPC Failure

## Overview
Primary and all fallback RPC endpoints become unreachable or unresponsive. Trading cannot proceed without a functional RPC connection.

## Detection
- **Alert**: `SubmissionFailureRate` or `BotStopped` with RPC-related stop condition
- **Metrics**: `solbot_submission_latency_ms` spikes or gaps
- **Logs**: ERROR-level in `sol_trade_sdk::rpc` or `sol_trade_sdk::submission`

## Severity
**Critical** — bot cannot operate without RPC.

## Immediate Action (0-5 minutes)
1. Acknowledge alert
2. Verify RPC status: `curl -v --max-time 5 <primary_rpc>`
3. Check if bot has already stopped (condition #1)
4. If bot still running: execute `solbot ops stop` to prevent bad trades

## Investigation (5-30 minutes)
1. Test all configured RPC endpoints from the VPS
2. Check RPC provider status pages
3. Verify API keys and rate limits
4. Check DNS resolution and firewall rules
5. Check if the VPS IP has been rate-limited or blocked

## Resolution
1. If provider-wide outage: switch to backup RPC provider
2. If rate-limited: wait for rate limit window to expire
3. If local firewall issue: update firewall rules
4. Update RPC config with working endpoints: `/etc/solbot/production.toml`
5. Restart bot: `systemctl start solbot`

## Recovery Verification
- [ ] Bot connects to RPC and receives blockhash
- [ ] Health endpoint returns 200
- [ ] Submission queue drains
- [ ] Normal trading resumes

## Post-Mortem
- Duration of RPC outage
- Which provider(s) were affected
- Whether failover worked
- Any financial impact from missed trades

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Provider Failure](provider-failure.md)