# ADR-003: Language Stack — Rust + Assembly + C17

**Status**: Accepted
**Date**: 2026-04-04

## Context

The OS requires languages suitable for kernel development (hardware access, zero overhead) and userland (standard libraries, familiarity). Safety, tooling quality, and developer productivity are key factors.

## Decision

- **Kernel**: Rust (primary) + x86_64 assembly (boot, context switch, interrupt stubs)
- **Userland (phase 1)**: C17 with custom `libc-lite`
- **Userland (phase 2)**: Optionally Rust after syscall ABI stabilization
- **Build tools**: Cargo (Rust), clang/lld (C), nasm (assembly)

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| C for kernel | No memory safety guarantees, higher bug density |
| C++ for kernel | Complex, UB-prone, poor freestanding support |
| Zig for kernel | Immature ecosystem, fewer OS dev resources |
| Rust for initial userland | Requires stable syscall ABI and custom std; C17 is simpler to bootstrap |

## Consequences

- Kernel benefits from Rust's ownership model, reducing memory corruption bugs
- `unsafe` code required for hardware access — policy: every unsafe block must have safety comment
- Userland libc-lite is minimal: just enough to support basic programs
- Two toolchains in CI (Cargo + clang)
- Cross-language FFI minimized (kernel is pure Rust+ASM; userland is pure C)

## Risks

- Rust no_std ecosystem gaps (mitigate: implement missing functionality as needed)
- Assembly maintenance burden (mitigate: minimize assembly, document thoroughly)

## Rollback

Language choice is deeply embedded. Changing kernel language would be a rewrite. Changing userland language is lower cost due to ABI boundary.
