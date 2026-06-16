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

## Implementation status (2026-06-16)

The §Decision section called `sys_spawn` the MVP creation path with fork and user threads as post-MVP work. Both have since shipped:

* `sys_spawn` (syscall #12) — still the common path racsh uses for external commands.
* `sys_fork` (syscall #26) — full fork with per-process address-space copy. No copy-on-write yet, so a fork pays a page-table-copy cost up front. CoW is still tracked on ADR-008.
* `sys_clone` (syscall #77) — used by libc-lite's threading primitive; `CLONE_THREAD` shares the address space + fd table while allocating a fresh kernel stack and PID-aliased TID.
* `sys_exec` (#11), `sys_wait`/`sys_waitpid` (#13/#63) — wired through racsh and the init service manager (PR #8). Orphan reparenting to PID 1 + SIGCHLD-on-exit was completed in PRs #5 and #6.

So the "POSIX patterns won't work initially" caveat under §Consequences is outdated — fork/exec/wait is the path racsh's job control already uses today.
