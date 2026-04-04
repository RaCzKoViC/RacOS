# ADR-013: Logging and Journaling Model

**Status**: Accepted
**Date**: 2026-04-04

## Context

System logging is essential for debugging, monitoring, and incident response. The logging model must work from early boot (serial) through full operation (structured journal).

## Decision

### Boot-time logging
- Serial output (COM1, 115200 baud)
- Structured format: `[timestamp] COMPONENT: message`
- Kernel ring buffer (fixed-size circular buffer in memory)

### Runtime logging
- RacInit captures stdout/stderr of managed services
- Writes to journal files in `/var/log/racinit/`
- Entries tagged with: timestamp, unit name, PID, priority level
- Journal files are plain text (binary journal format deferred to post-MVP)

### Log levels
- EMERG, CRIT, ERR, WARN, INFO, DEBUG, TRACE

## Consequences

- All boot messages available via serial even if system fails to start
- Service logs centralized and queryable via `servicectl log`
- No dependency on external syslog daemon
- Text-based journal is simple but may need rotation (logrotate-like timer unit)

## Risks

- Disk space growth from logs (mitigate: log rotation timer unit)
- Lost logs on crash (mitigate: flush frequently, serial always available)

## Rollback

Switching to binary journal format can be done transparently from RacInit's perspective by changing the journal writer module.
