# ADR-005: Versioned Syscall ABI Policy

**Status**: Accepted
**Date**: 2026-04-04

## Context

The syscall interface is the contract between kernel and user space. Unversioned, undocumented syscall changes break user programs silently. A clear policy is needed from the start.

## Decision

- Every syscall has a **fixed number**, **specified arguments/return values**, **error codes**, and a **stability level** (Stable / Unstable / Experimental)
- ABI is versioned as `ABI_MAJOR.ABI_MINOR`
- Stable syscalls cannot be removed without a major ABI version bump and 2-version deprecation period
- Every ABI change requires a new ADR
- x86_64 calling convention: `syscall` instruction, args in RDI/RSI/RDX/R10/R8/R9, number in RAX, return in RAX

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| Unversioned ABI | Silent breakage, impossible to reason about compatibility |
| Message-passing instead of syscalls | Performance overhead, complexity, microkernel territory |
| Version per-syscall | Over-engineered for v1; global ABI version is sufficient |

## Consequences

- User programs compiled against ABI v1 will work with any kernel ABI v1.x
- Adding new syscalls bumps ABI_MINOR
- Removing syscalls bumps ABI_MAJOR
- libc-lite wraps syscalls with stable function signatures
- Syscall specification (SYSCALL_SPEC.md) is the source of truth

## Risks

- ABI design mistakes are expensive to fix (mitigate: thorough review before marking Stable)
- New to Unstable → Stable promotion needs formal process

## Rollback

Individual syscalls can be deprecated (2-version period). ABI major version bump is a hard reset for compatibility.
