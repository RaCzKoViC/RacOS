# ADR-004: Boot Flow and Image Format

**Status**: Accepted
**Date**: 2026-04-04

## Context

RacOS needs a defined boot sequence from firmware to running kernel, with clear handoff points and a documented image format.

## Decision

Boot flow: **UEFI firmware → custom UEFI bootloader (EFI application) → RaCore kernel (ELF64) + initramfs**

The bootloader is a Rust UEFI application that loads the kernel ELF64 and initramfs into memory, gathers hardware info into a BootInfo structure, calls ExitBootServices, and jumps to the kernel entry point.

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| GRUB | External dependency, configuration complexity, not fully controlled |
| Limine | Third-party dependency; want full control of boot chain |
| Direct UEFI kernel | Mixing UEFI protocols with kernel code, poor separation |
| Multiboot2 | Requires legacy BIOS support, UEFI is preferred |

## Consequences

- Full control over boot process
- BootInfo structure is a stable contract between bootloader and kernel
- Bootloader is a separate Cargo crate targeting `x86_64-unknown-uefi`
- Kernel is a separate ELF64 binary targeting `x86_64-unknown-none`
- Two separate build targets in the workspace
- OVMF required for QEMU testing

## Risks

- ELF loading complexity (mitigate: strict validation, known good test binaries)
- BootInfo versioning (mitigate: magic + version field, kernel validates on entry)

## Rollback

Switching to GRUB/Limine possible by replacing bootloader crate. Kernel entry point remains the same if BootInfo format is adapted.
