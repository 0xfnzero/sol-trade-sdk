# Runbook: Duplicate Position

## Overview
The bot opens the same position twice (same token, same direction) due to event duplication, state machine bug, or submission retry logic.

## Detection
- **Alert**: `DuplicateSignatureDetected` or condition #8 triggered
- **Bot state**: Position tracker shows duplicate entries for same token/side
- **Logs**: "Duplicate position detected" WARN/ERROR

## Severity
**Critical** — financial loss risk from double exposure.

## Immediate Action (0-5 minutes)
1. Stop the bot: `systemctl stop solbot`
2. Verify whether duplicate transaction(s) actually executed on-chain
3. Assess additional capital at risk from the duplicate
4. If duplicate executed: consider immediately closing one position
5. If duplicate not executed: clear the pending duplicate

## Investigation (5-30 minutes)
1. Review event sequence that triggered the duplicate
2. Check event deduplication logic for gaps
3. Examine submission retry logic — did it resubmit unnecessarily?
4. Check if multi-lane same-signature produced different signatures

## Resolution
1. **If duplicate executed**: close the duplicate position manually
2. **If duplicate not executed**: clear the duplicate state entry
3. Fix the deduplication / retry logic bug (code change required)
4. Deploy fix to canary, then production
5. Resume trading after fix is validated

## Recovery Verification
- [ ] Position tracker shows 1 position per token (no duplicates)
- [ ] Wallet balance matches expected (after closing duplicate if needed)
- [ ] Event deduplication passes increased scrutiny
- [ ] Bot restarts and trades normally

## Post-Mortem
- Root cause of duplication
- Financial impact (if any)
- Whether duplicate was caught before execution
- Improvements to deduplication logic
- Improvements to position tracking

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Ambiguous Submission](ambiguous-submission.md)