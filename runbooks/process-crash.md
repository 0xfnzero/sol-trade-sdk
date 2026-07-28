# Runbook: Process Crash

## Overview
The sol-trade-bot process exits unexpectedly due to panic, out-of-memory (OOM), segfault, or external kill.

## Detection
- **Alert**: `BotStopped` — `solbot_uptime_seconds == 0`
- **System**: systemd reports process exited with non-zero code
- **Logs**: "Panic" or "Aborted" in journal, or sudden end of log output

## Severity
**High** — trading stops; potential for state corruption or missed opportunities.

## Immediate Action (0-5 minutes)
1. Check process status: `systemctl status solbot`
2. Get last output: `journalctl -u solbot --since "10 minutes ago" -n 100`
3. If systemd restart policy is active: note whether bot restarted
4. Check for core dump: `coredumpctl list 2>/dev/null | grep solbot`
5. Assess if crash caused incomplete submissions or state corruption

## Investigation (5-30 minutes)
1. **If panic**: find the panic message and backtrace
   ```bash
   journalctl -u solbot | grep -A 20 "panicked at"
   ```
2. **If OOM**: check dmesg for OOM killer
   ```bash
   dmesg | grep -i "oom\|killed"
   ```
3. **If segfault**: check core dump
   ```bash
   coredumpctl info solbot
   ```
4. **If killed externally**: check auth.log for ssh/sudo activity
5. Review recent changes (deploy, config change, dependency update)

## Resolution
1. **Panic bug**: file bug report, develop fix, deploy fix
2. **OOM**: increase memory limit in systemd unit; investigate memory leak
3. **Segfault**: check hardware, kernel, and Rust version; may be hardware error
4. **External kill**: investigate unauthorized access; run secret-exposure runbook
5. Temporary workaround: increase memory limit and restart

## Recovery Verification
- [ ] Bot restarts cleanly
- [ ] State recovery completes
- [ ] No corrupted state files
- [ ] Memory usage stabilizes at normal levels

## Post-Mortem
- Root cause of crash
- Whether automatic restart helped or hindered
- State corruption (if any)
- Improvements to crash resilience

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Secret Exposure](secret-exposure.md)