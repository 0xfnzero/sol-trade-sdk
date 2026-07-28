# Runbook: Secret Exposure

## Overview
Wallet private key, SWQOS API key/token, RPC API key, or other credentials may have been exposed through logs, error messages, or unauthorized access.

## Detection
- **Alert**: External security tool notification, or manual discovery
- **Logs**: Audit trail shows credential in plaintext (should be prevented by redaction)
- **Security monitor**: File integrity alert on secrets directory

## Severity
**Critical** — immediate risk of fund loss and unauthorized access.

## Immediate Action (0-5 minutes)
1. **STOP THE BOT**: `systemctl stop solbot`
2. **DISABLE NETWORK ACCESS**: `systemctl stop solbot` + firewall block
3. **ROTATE ALL AFFECTED CREDENTIALS IMMEDIATELY**:
   - Wallet keypair: Generate new keypair, transfer funds
   - SWQOS API keys: Regenerate via provider dashboards
   - RPC API keys: Regenerate via provider dashboards
4. **REVOKE COMPROMISED CREDENTIALS**: Use provider consoles

## Investigation (5-30 minutes)
1. Determine exposure vector:
   - Log file (debug level left on in production?)
   - Error message returned by provider
   - CI/CD pipeline leakage
   - Unauthorized system access
2. Review access logs for unauthorized activity
3. Check if funds were moved from the wallet
4. Scan for other exposed secrets

## Resolution
1. Update all rotated credentials in environment file
2. Verify new credentials work: `solbot ops test-credentials`
3. Perform security audit of log redaction:
   - Check all `info!`, `warn!`, `error!` calls for credential leakage
   - Ensure production runs at `warn` level minimum
4. If caused by access control issue: review and harden VPS security
5. Restart bot: `systemctl start solbot`

## Recovery Verification
- [ ] All rotated credentials work correctly
- [ ] No unauthorized transactions on wallet
- [ ] Log audit shows no credential leakage
- [ ] Security controls are verified
- [ ] Bot resumes trading

## Post-Mortem
- Exposure vector and root cause
- Duration of exposure
- Whether funds were lost
- Improvements to secret management
- Log redaction audit results

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Code Review — Security Issues](../docs/CODE_REVIEW_REPORT.md#6-security)
- VPS security hardening documentation