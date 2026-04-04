// RacOS UEFI Bootloader — Placeholder
//
// This crate will contain the UEFI bootloader that:
// 1. Loads the RaCore kernel ELF64
// 2. Loads the initramfs
// 3. Gathers BootInfo (memory map, framebuffer, RSDP)
// 4. Calls ExitBootServices
// 5. Jumps to kernel entry point
//
// Implementation begins in Phase B (Sprint 2).
// Target: x86_64-unknown-uefi
//
// For now, this is a library placeholder so the workspace compiles.

#![no_std]

/// Bootloader version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
