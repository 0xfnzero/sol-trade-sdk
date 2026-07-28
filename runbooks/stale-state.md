# Runbook: Stale State

## Overview
The bot's internal state has diverged from on-chain reality. This can lead to incorrect trade decisions, missed opportunities, or phantom positions.

## Detection
- **Alert**: `SlotDrift` (slot gap > 5)
- **Metrics**: `solbot_shredstream_slot_gap > 0`
- **Logs**: "State inconsistency detected" or "Slot mismatch"
- **Bot behavior**: Trades executed but on-chain state doesn't match expectations

## Severity
**Critical** — trading based on incorrect information risks financial loss.

## Immediate Action (0-5 minutes)
1. Stop the bot: `systemctl stop solbot`
2. Determine extent of state drift
3. Compare bot state with on-chain state:
   ```bash
   solbot ops compare-state --rpc <rpc_url>
   ```

## Investigation (5-30 minutes)
1. Check when state last matched on-chain
2. Analyze event replay from that point
3. Check for missing events, replay gaps, or processing bugs
4. Examine state persistence files for corruption
5. Check ShredStream event sequence numbers for gaps

## Resolution
1. **Full reset**: Delete local state and let bot recatch from genesis
   ```bash
   solbot ops reset-state --confirm
   systemctl start solbot
   ```
2. **Targeted fix**: If only certain tokens affected, clear those positions
   ```bash
   solbot ops clear-position --token <mint> --confirm
   ```
3. After reset: verify state catch-up completes without errors

## Recovery Verification
- [ ] Bot slot matches RPC slot (within 1-2 slots)
- [ ] Position tracking matches on-chain
- [ ] Wallet balance matches on-chain
- [ ] Event processing resumes without errors
- [ ] No false positives from stale events

## Post-Mortem
- When state drift began
- Root cause (event gap, processing bug, data corruption)
- Detection delay (TTD)
- Whether any incorrect trades were made during drift

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Packet Loss](packet-loss.md)