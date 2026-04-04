# ADR-001: x86_64 + UEFI as Starting Platform

**Status**: Accepted
**Date**: 2026-04-04

## Context

RacOS needs a target platform for v1.0. Supporting multiple architectures from the start would multiply complexity without proportional benefit. We need a platform with mature tooling, widespread emulation support, and clear documentation.

## Decision

Target exclusively **x86_64** with **UEFI** firmware for v1.0. Development and testing performed in **QEMU/KVM** with OVMF firmware.

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| x86 (32-bit) | Obsolete, no benefit in starting with 32-bit |
| ARM64 (AArch64) | Requires separate arch layer from day one, doubles boot code |
| RISC-V | Immature tooling, QEMU support less stable |
| Legacy BIOS | UEFI is the modern standard; BIOS adds complexity for no gain |
| Multi-arch from start | Spreads resources too thin; arch layer abstraction can be added later |

## Consequences

- All boot code targets UEFI protocols
- All assembly targets x86_64 instruction set
- Kernel uses 4-level paging (PML4)
- QEMU/KVM is the primary test environment
- Hardware testing deferred until post-1.0
- ARM/RISC-V support requires future arch layer work (HAL abstraction planned)

## Risks

- UEFI specification complexity (mitigate: use `uefi` Rust crate)
- QEMU behavior may differ from real hardware (mitigate: document known differences)

## Rollback

Adding another architecture later requires implementing a new `kernel/arch/<arch>/` module. The HAL layer is designed for this. No changes to upper layers needed.
