# ADR-006: Process and Thread Model

**Status**: Accepted
**Date**: 2026-04-04

## Context

The process model determines how programs are isolated, identified, and managed. It affects scheduling, IPC, job control, and the service manager.

## Decision

Processes have: PID (unique), PPID (parent), session ID, process group, state (Running/Ready/Blocked/Zombie/Stopped), own address space, file descriptor table, capabilities, uid/gid.

- PID 1 = RacInit (always)
- Process creation via `sys_spawn` (combined fork+exec) for MVP; traditional `fork` considered post-MVP
- Threads: kernel threads from the start; user threads (clone-like) post-MVP
- Sessions and process groups support job control (foreground/background)
- Wait semantics: parent collects child exit status via `sys_wait`

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| fork() from start | Complex COW implementation needed; sys_spawn is simpler for MVP |
| No process groups | Breaks job control, shell cannot manage foreground/background |
| Flat PID space (no sessions) | Insufficient for TTY/terminal session management |

## Consequences

- Shell can implement job control (fg/bg/Ctrl-Z)
- RacInit can track service processes by PID/PPID
- No fork() in MVP means some POSIX patterns won't work initially
- Process groups are needed for signal delivery to foreground group

## Risks

- sys_spawn may not cover all use cases (mitigate: add fork/clone later if needed)
- Orphan process handling must be correct (reparent to PID 1)

## Rollback

Adding fork/clone later is additive; existing sys_spawn remains.
