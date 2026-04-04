# ADR-014: TTY/PTTY Model

**Status**: Accepted
**Date**: 2026-04-04

## Context

TTY and pseudo-terminal (PTY) support is required for interactive shells, terminal emulators, and job control. The kernel must provide the infrastructure for terminal I/O.

## Decision

- **TTY subsystem** in the kernel with line discipline processing
- **PTY** support: master/slave pairs via `/dev/ptmx` (master allocator) and `/dev/pts/N` (slave devices)
- **Line discipline**: canonical mode (line editing) and raw mode
- **Session/process group** binding to controlling terminal
- **Job control signals**: SIGINT (Ctrl-C), SIGTSTP (Ctrl-Z), SIGQUIT (Ctrl-\)
- **ANSI/VT escape passthrough**: TTY layer passes escape sequences transparently in raw mode
- **Resize**: TIOCSWINSZ ioctl to set terminal size, SIGWINCH delivered to foreground process group

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| No PTY (direct terminal) | Breaks terminal emulator architecture; only hardware consoles would work |
| Userland-only TTY | Cannot implement job control signals and session management without kernel support |

## Consequences

- Shell (racsh) reads from PTY slave, receives signals from TTY layer
- Terminal (RacTerm) holds PTY master, sends input, receives output
- Line discipline handles backspace, echo, Ctrl-C in canonical mode
- Process groups enable signal delivery to foreground job
- TTY layer is in kernel; rendering is in userland (RacTerm)

## Risks

- TTY/PTY is historically complex (mitigate: implement minimal subset, test thoroughly)
- Line discipline bugs can break interactive input (mitigate: raw mode bypass for testing)

## Rollback

TTY subsystem is a kernel module; can be revised without ABI changes as long as ioctl interface is maintained.
