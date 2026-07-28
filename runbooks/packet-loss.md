# Runbook: Packet Loss

## Overview
ShredStream connection experiences packet loss, disconnection, or data corruption. This affects event detection and may cause state drift.

## Detection
- **Alert**: `ShredStreamDisconnected` — reconnects >5 in 1 hour
- **Metrics**: `solbot_shredstream_reconnects_total` increasing
- **Logs**: WARN/ERROR-level in `sol_trade_sdk::shredstream`

## Severity
**High** — degrades trading performance; may cause stale state.

## Immediate Action (0-5 minutes)
1. Acknowledge alert in PagerDuty/Opsgenie
2. Check bot status: `solbot status | grep shredstream`
3. If bot is still trading with stale state: consider stopping
4. Check ShredStream provider status page

## Investigation (5-30 minutes)
1. Examine ShredStream logs for pattern: `journalctl -u solbot | grep shredstream`
2. Check network latency: `ping <shredstream_endpoint>`
3. Verify TLS certificate validity: `openssl s_client -connect <endpoint>:443`
4. Check if this is a provider-wide outage (status page, #ops channel)

## Resolution
1. If transient: wait for automatic reconnect (configurable delay)
2. If provider outage: switch to backup ShredStream provider (if available)
3. If local network issue: restart network service, verify firewall rules
4. After reconnection: verify slot catch-up completes
5. Verify state consistency: `solbot ops verify-state`

## Recovery Verification
- [ ] ShredStream status shows "connected"
- [ ] Slot number matches RPC slot reference
- [ ] Event processing resumed (metrics increasing)
- [ ] No more reconnects in the last 10 minutes

## Post-Mortem
- Duration of packet loss / disconnection
- Root cause (local network, provider, internet)
- Whether state drift occurred and was corrected
- Any missed opportunities due to the gap

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Stale State](stale-state.md)