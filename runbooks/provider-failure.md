# Runbook: Provider Failure

## Overview
One or more SWQOS transaction submission providers (Jito, Bloxroute, NextBlock, FlashBlock) fail to accept or confirm transactions.

## Detection
- **Alert**: `SubmissionFailureRate{provider=...} > 0.1`
- **Metrics**: `solbot_submissions_total{status="failed"}` increasing
- **Logs**: ERROR-level in `sol_trade_sdk::swqos`

## Severity
**High** — reduced submission throughput; may affect trade success rate.

## Immediate Action (0-5 minutes)
1. Acknowledge alert
2. Check provider status: `solbot status providers`
3. Review recent submissions: `journalctl -u solbot | grep -i "jito\|bloxroute\|nextblock\|flashblock"`

## Investigation (5-30 minutes)
1. Test each provider endpoint in isolation
2. Verify provider API keys are valid and not expired
3. Check provider status pages and social media for outages
4. Review rate limits and quota usage
5. Test submission directly: `solbot ops test-provider --provider <name>`

## Resolution
1. If single provider failure: rely on remaining providers
2. If all providers fail: follow Emergency Shutdown
3. If API key expired: rotate and update environment file
4. If rate-limited: wait or upgrade provider plan
5. If provider-specific issue: file support ticket with provider

## Recovery Verification
- [ ] Failed provider(s) come back online
- [ ] Submission success rate returns to ≥ 95%
- [ ] Confirmation rate returns to normal
- [ ] No trade failures attributed to submission

## Post-Mortem
- Which provider(s) failed
- Root cause (API key, rate limit, provider outage)
- How failover worked
- Need for additional provider redundancy

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [RPC Failure](rpc-failure.md)