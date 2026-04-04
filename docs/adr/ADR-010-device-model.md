# ADR-010: Device Model and /dev

**Status**: Accepted
**Date**: 2026-04-04

## Context

The kernel needs a device model for user space to access hardware and pseudo-devices. Devices must be accessible through the VFS as file-like objects in /dev.

## Decision

- RaCore has a **device registry** in the kernel
- Devices are either **character devices** (byte stream: serial, TTY, random) or **block devices** (fixed-size blocks: disk)
- Each device has a major/minor number pair
- `/dev` is a special filesystem (**devfs**) where device nodes appear automatically when drivers register devices
- User space accesses devices via standard `open/read/write/ioctl/close` syscalls
- Device-specific behavior is handled by `ioctl`

### MVP Devices

| Device | Type | Major | Path |
|--------|------|-------|------|
| Serial COM1 | char | 1 | /dev/serial0 |
| Null | char | 2 | /dev/null |
| Zero | char | 3 | /dev/zero |
| Console | char | 4 | /dev/console |
| TTY | char | 5 | /dev/ttyN |
| PTY master | char | 6 | /dev/ptmx |
| PTY slave | char | 7 | /dev/pts/N |
| Random | char | 8 | /dev/random |

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| Static /dev (pre-created nodes) | Inflexible, doesn't reflect actual hardware |
| udev-like userland device manager | Over-complex for MVP; small device set manageable in kernel |
| No /dev (special syscalls per device) | Breaks file abstraction, poor composability |

## Consequences

- Drivers register with the device registry → devfs creates nodes automatically
- ioctl interface is device-specific and documented per driver
- PTY devices support the terminal/shell stack
- /dev/null and /dev/zero are trivial but essential for shell/scripting

## Risks

- Major/minor number conflicts (mitigate: central allocation in this ADR)
- ioctl sprawl (mitigate: document each ioctl, prefer well-defined interfaces)

## Rollback

Device model is internal. Individual devices can be added/removed by registering/unregistering drivers.
