# Runbook: Protocol Upgrade

## Overview
A DEX protocol (PumpFun, Raydium, etc.) has been upgraded — detected via IDL hash mismatch, program address change, or unexpected simulateTransaction results.

## Detection
- **Alert**: `IDL hash mismatch` from CI scheduled job or runtime check
- **Logs**: "IDL hash mismatch: expected X, got Y"
- **simulateTransaction**: returns unexpected byte layout error
- **Manual**: Protocol team announces upgrade on social media / Discord

## Severity
**High** — instruction building may produce invalid transactions.

## Immediate Action (0-5 minutes)
1. Check if bot has already stopped (condition #17)
2. If still running: stop the bot manually
3. Verify the upgrade status via protocol's official channels

## Investigation (5-30 minutes)
1. Compare new IDL with stored IDL to identify changes
2. Extract new discriminator values and account layouts
3. Identify which instructions are affected
4. Check if the upgrade is backward-compatible (old instructions still work)
5. Test new instruction building with simulateTransaction

## Resolution
1. Update IDL files in `idl/` directory
2. Update instruction builders in `src/instruction/`
3. Update golden fixtures in `tests/golden/`
4. Update REFERENCE_HASHES.txt
5. Run full test suite: `cargo test --workspace`
6. Deploy to canary first, then production
7. Restart bot with updated binary

## Recovery Verification
- [ ] IDL hash check passes
- [ ] All instruction builders produce correct byte layout
- [ ] Golden fixtures match new IDL
- [ ] simulateTransaction succeeds for all protocols
- [ ] Canary operations successful for 24h

## Post-Mortem
- Which protocol was upgraded
- What changed (instruction layout, accounts, discriminators)
- Detection time relative to upgrade time
- Whether tests caught the change
- Time to update SDK and deploy

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Test Architecture — Golden Fixtures](../artifacts/test-evidence.md#2-golden-fixtures)
- [CI — IDL Hash Check](../.github/workflows/ci.yml)