# ADR-011: Init/Service Manager Model (RacInit)

**Status**: Accepted
**Date**: 2026-04-04

## Context

Every OS needs an init process (PID 1) that bootstraps user space and manages services. RacOS needs its own service manager that is predictable, dependency-aware, and debuggable.

## Decision

RacInit is an original init/service manager functionally inspired by systemd's organizational model but with its own code, format, and semantics. It runs as PID 1 and manages services through unit files with a dependency graph.

Key properties:
- Unit types: service, target, timer, mount, device
- Dependency resolution via DAG with cycle detection
- Restart policies: no, on-failure, on-abnormal, always
- Timeout enforcement with SIGTERM → SIGKILL escalation
- Log routing to journal files
- Admin CLI: `servicectl`
- Socket activation deferred to post-MVP

## Consequences

- System boot is deterministic (dependency-ordered)
- Service failures are handled automatically per policy
- Unit file format is documented and testable
- servicectl provides operational visibility

## Risks

- Dependency graph bugs can block boot (mitigate: cycle detection, timeout-and-skip)
- PID 1 crash = system crash (mitigate: minimal code in PID 1 hot path, extensive testing)

## Rollback

Replacing RacInit requires writing a new init that conforms to the kernel's PID 1 expectations (receives orphans, handles signals). Unit file format is independent.
