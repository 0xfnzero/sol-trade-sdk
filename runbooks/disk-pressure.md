# Runbook: Disk Pressure

## Overview
Disk space on the VPS is critically low, threatening log writing, state persistence, and system stability.

## Detection
- **Alert**: `DiskSpaceLow` (Warning) or condition #14 triggered
- **Monitoring**: Disk usage monitoring detects >90% utilization
- **System**: `df -h` shows filesystem at >90% capacity

## Severity
**High** — can lead to state corruption, log loss, and system instability.

## Immediate Action (0-5 minutes)
1. Check disk usage: `df -h`
2. Identify largest directories: `du -sh /var/log/solbot /opt/solbot /tmp`
3. If disk is >95% full: rotate and compress logs immediately
   ```bash
   logrotate -f /etc/logrotate.d/solbot
   ```
4. If state files are large: evaluate whether to archive old state checkpoints

## Investigation (5-30 minutes)
1. Identify what is consuming space:
   - Log files: `du -sh /var/log/solbot/*`
   - State persistence: `du -sh /opt/solbot/state/*`
   - Core dumps: `du -sh /var/lib/systemd/coredump/*`
   - Temporary files: `du -sh /tmp/*`
2. Check for unexpected large files (debug dumps, captured traffic)
3. Verify log rotation configuration is working
4. If bot is still running: check if state files are growing unbounded

## Resolution
1. **Log cleanup**: compress/archive old logs
   ```bash
   find /var/log/solbot -name "*.log" -mtime +7 -exec gzip {} \;
   ```
2. **State cleanup**: if configured, remove old state checkpoints
3. **Increase disk**: if trend shows continuous growth, plan disk expansion
4. **Log rotation**: ensure logrotate configuration is correct
5. **Bot state retention**: configure max state file age in config
6. If growth is unbounded: fix the source (log spam, state leak)

## Recovery Verification
- [ ] Disk usage below 80%
- [ ] Log rotation is working
- [ ] State files are within configured limits
- [ ] Bot continues to function normally

## Post-Mortem
- What caused disk pressure
- Whether log rotation was configured correctly
- Whether state file growth is expected
- Need for automated cleanup policies

## References
- [Emergency Shutdown](emergency-shutdown.md)
- [Deployment — Systemd](../docs/deployment.md#2-systemd-service-design)