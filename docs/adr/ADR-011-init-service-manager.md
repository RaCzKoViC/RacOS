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

## Implementation status (2026-06-16)

The engine shipped in T1.3. RacInit runs as PID 1 with a real dependency graph and restart policy; `servicectl` and socket activation are still deferred.

**Shipped:**
* Unit file parser for `.service` / `.target` / `.timer` / `.mount` / `.device` in `init/src/lib.rs`. Round-trip tested by host suite `init/tests/engine.rs`.
* Dependency resolution via Kahn's topological sort with cycle detection — `Engine::resolve_start_order` returns `ResolveResult { order, cycle }`; cycles, self-edges, linear, and diamond graphs are all covered by host tests.
* Restart policies (`no` / `on-failure` / `on-abnormal` / `always`) plus a burst limiter: 5 restarts within a 30-second window flips the unit to `Failed`. Window-decay tracker is host-tested.
* Wired to PID 1 in `userland/coreutils/init/main.rs` — the engine path runs if `/etc/racinit/units/` has any unit files; falls back to the legacy spawn-shell loop otherwise.
* Default unit files shipped in `initramfs-root/etc/racinit/units/`: `base.target` + `shell.service` (`Restart=always`).
* Orphan reparenting + SIGCHLD delivery in `kernel/src/task/scheduler.rs:exit_current`; verified by `racos-test::test_sigchld_waitpid` (PHASE21-SIGCHLD-WAIT-OK).
* CI smoke `T13-INIT-ENGINE-OK` boots the engine path in QEMU and asserts the shell came up.
* SIGTERM → SIGKILL escalation is delivered through the existing signal queue (`sys_kill` → `send_signal_to`); per-unit timeout enforcement is not wired yet.

**Still deferred:**
* **`servicectl` CLI** — spec lives in `ARCHITECTURE.md §8.4` but no userland binary exists. Operational visibility today is via `serial` logs only.
* **`.timer` scheduling** — units parse but the engine doesn't have a clock-tick driver to fire them.
* **Socket activation** — still post-MVP per §Decision.
* **Per-unit log routing** — RacInit doesn't capture child stdout/stderr into journal files (see ADR-013 status).
* **Per-unit timeout enforcement** — there's no watchdog that turns a hung start into SIGTERM+SIGKILL. Services run forever once spawned; the existing init watchdog (`kernel/src/main.rs:534`) only catches early crashes of PID 100 (the boot init).
