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
