# ADR-002: Modular Monolithic Kernel Architecture

**Status**: Accepted
**Date**: 2026-04-04

## Context

The kernel architecture determines the fundamental trade-offs between performance, complexity, and maintainability. RacOS needs a practical kernel design that can reach a working state quickly while remaining extensible.

## Decision

RaCore uses a **modular monolithic kernel** architecture. Critical drivers run in kernel space. Extensibility via loadable kernel modules is planned after internal ABI stabilization.

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| Microkernel | Higher IPC overhead, vastly more complex to get working; poor fit for a first OS |
| Hybrid kernel | Ambiguous boundaries, risks worst of both worlds |
| Exokernel | Too experimental, poor tooling ecosystem |
| Unikernel | No user/kernel separation, not suitable for general-purpose OS |

## Consequences

- All kernel subsystems (mm, sched, vfs, drivers) share the same address space
- Driver bugs can crash the kernel (mitigate: Rust safety, testing)
- High performance for syscalls and driver I/O (no IPC overhead)
- Module loading deferred until ABI is stable
- Internal APIs between subsystems must be explicitly documented

## Risks

- Monolithic kernels have larger attack surface (mitigate: capability model, syscall filtering)
- Poor driver isolation (mitigate: Rust memory safety reduces class of bugs)

## Rollback

Switching to microkernel would require fundamental redesign. This is effectively a one-way decision for v1.x.
