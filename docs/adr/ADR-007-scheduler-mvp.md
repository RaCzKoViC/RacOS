# ADR-007: MVP Scheduler — Round-Robin

**Status**: Accepted
**Date**: 2026-04-04

## Context

The scheduler determines how CPU time is distributed among tasks. A simple, correct scheduler is needed for MVP; sophistication comes later.

## Decision

Scheduler progression:
1. **MVP (v0.1)**: Round-robin with fixed time quantum (10ms)
2. **v0.3**: Static priority levels (0–31), higher priority preempts lower
3. **v0.5**: Fairness improvements (CFS-inspired virtual runtime tracking)
4. **Post-1.0**: Real-time scheduling class

MVP scheduler is UP (uniprocessor) only. SMP support planned for post-1.0.

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| Priority scheduler from start | Additional complexity before basic context switch works |
| CFS-like from start | Over-engineered for MVP without profiling data |
| Cooperative scheduling | Unacceptable for a general-purpose OS (one task can starve all others) |

## Consequences

- All tasks get equal CPU time in MVP
- Timer interrupt triggers context switch
- Scheduler is a replaceable module behind a trait/interface
- Performance testing needed before priority scheduler upgrade
- No SMP in v1.0 (single CPU core)

## Risks

- Round-robin may cause latency issues for interactive tasks (acceptable for MVP)
- Scheduler bugs cause system hangs (mitigate: watchdog timer, serial debug)

## Rollback

Scheduler is module-based. Upgrading from RR to priority is a module replacement, not a rewrite.

## Implementation status (2026-06-16)

The §Decision section pinned SMP to post-1.0 with the UP-only round-robin as MVP. The SMP boundary has partially moved.

**Shipped (T3.1, PR #14):**
* ACPI CPU enumeration via `arch::smp::init` populates a 32-slot CpuState table.
* `arch::ap::bring_up_all` runs the full INIT-SIPI-SIPI bring-up with a real→protected→long-mode trampoline in `kernel/src/arch/trampoline.asm`.
* Each AP lands in `ap_entry`, loads the kernel GDT/IDT, enables its LAPIC, binds its GS to its PerCpu slot, starts its own LAPIC timer (per-CPU periodic tick), and flips `smp::mark_started`.
* `/proc/cpuinfo` enumerates online CPUs (BSP + APs).
* CI boot-smoke runs QEMU with `-smp 4` and grep-gates the enumeration + bring-up logs.

**Still UP-style (T4.1 in roadmap):**
* The scheduler itself remains a single global run queue. APs run their tick handler but park (`sti; hlt`); they don't pick up tasks from a per-CPU queue.
* No IPI-based preemption (LAPIC ICR send-vector path).
* No per-CPU TSS for ring-3 IRQs on APs (today APs only handle ring-0 timer IRQs while parked).

So §Consequences "No SMP in v1.0 (single CPU core)" is partly outdated: the CPUs are alive in v0, but they're not yet load-bearing for scheduling.
