# Runbook: Ambiguous Submission

## Overview
A transaction was submitted to at least one SWQOS provider but the confirmation status is unknown. This creates uncertainty about whether the trade executed on-chain.

## Detection
- **Alert**: `TradeFailureRate` (Warning)
- **Logs**: "Submission confirmed: timeout" or "Signature status: unknown"
- **Bot state**: Pending transaction count stops decreasing

## Severity
**Critical** — risk of double-spend or missed trade.

## Immediate Action (0-5 minutes)
1. Confirm if bot has stopped (condition #7). If so, stop is correct.
2. Do NOT restart the bot yet
3. Check the ambiguous signature on-chain:
   ```bash
   solana confirm --url <rpc_url> <signature>
   solana transaction-status --url <rpc_url> <signature>
   ```
4. Check if the transaction was already executed

## Investigation (5-30 minutes)
1. Check all RPC endpoints for the signature status
2. Check block explorer for the signature
3. Review bot logs: `journalctl -u solbot | grep <signature>`
4. Determine if the transaction was:
   - Confirmed (on-chain) → record the trade
   - Failed (on-chain) → no further action
   - Dropped (never on-chain) → may need to resubmit
   - Pending (still in mempool) → wait and recheck

## Resolution
1. **If confirmed**: record trade result, clear pending status, resume normal ops
2. **If failed**: clear pending status, resume normal ops (no funds lost)
3. **If dropped**: determine if the trade is still needed; resubmit if yes
4. **If pending**: wait up to 2 minutes, then treat as dropped if still unconfirmed
5. After resolving: `solbot ops clear-pending --signature <sig>`

## Recovery Verification
- [ ] No pending ambiguous transactions remain
- [ ] Wallet balance reflects correct state
- [ ] Position tracking matches on-chain state
- [ ] Bot can resume trading

## Post-Mortem
- How many signatures were ambiguous
- Why confirmation polling failed
- Whether failover RPC found the signature
- Need for additional RPC redundancy for confirmation

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Duplicate Position](duplicate-position.md)