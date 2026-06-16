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

## Implementation status (2026-06-16)

Boot-time logging is shipped; the runtime/journal half is still spec-only. The current state covers debugging end-to-end via serial, which is enough for the v0 development loop but not for an operator running the system.

**Shipped:**
* Serial output on COM1 at 115200 baud — `kernel/src/serial.rs`. Bit-for-bit ordering preserved (no buffering races) because writes are guarded by `cli/sti` around `outb`. The serial subsystem also handles input IRQ (Ctrl-C → SIGINT on the foreground PG) in `serial::handle_irq`.
* `println!` and `serial_println!` macros for component-tagged output. Format used in practice: `[ COMPONENT ] message` (e.g. `[  SCHED  ]`, `[ USERPROC ]`, `[ AHCI ]`, `[ SYSCALL ]`). The §Decision-prescribed `[timestamp] COMPONENT: message` was not adopted because the kernel doesn't have wall-clock time yet (RTC is a TODO); timestamps would only be uptime-since-PIT-init, and that turned out to clutter the log without adding value for the current debug workflow.
* Boot-time log is also pushed to the framebuffer console (`tty/vt.rs` + `fb_console.rs`).
* No kernel ring buffer yet — output goes straight to serial and framebuffer. CI relies on capturing the full serial log to a file, which makes a ring buffer redundant for tests; it'll be needed when a real `dmesg` syscall is added.

**Still deferred:**
* **Structured timestamps** — `[timestamp]` prefix is omitted because there's no RTC source. Once T4.x adds CMOS-RTC reads, the macro can be upgraded crate-wide without touching call sites.
* **Log levels** (EMERG/CRIT/ERR/WARN/INFO/DEBUG/TRACE) — not implemented. Today, severity is encoded informally in the component prefix (e.g. `[FAIL]` vs `[PASS]` in racos-test, `!!!` for panics). A real `log!(level=...)` macro would let CI filter on level instead of grepping prefixes.
* **`/var/log/racinit/` journal files** — RacInit doesn't capture service stdout/stderr (see ADR-011 status). Service output goes straight to the foreground TTY today.
* **Rotation** — N/A while there's no journal to rotate.
* **Binary journal format** — pre-rotation, pre-journal. Plain text is fine for v0.
* **`servicectl log`** — depends on `servicectl` (see ADR-011 status) plus the journal capture above.
