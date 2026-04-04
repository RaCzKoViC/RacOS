// RaCore — Kernel entry point
//
// This is the main kernel crate for RacOS. It targets x86_64 bare metal
// (no_std, no_main) and is loaded by the UEFI bootloader.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod arch;
mod boot;
mod serial;
mod panic;

use boot::BootInfo;

/// Kernel entry point, called from assembly stub `_start`.
///
/// # Safety
/// Called once by the bootloader with a valid BootInfo pointer.
/// Must not return.
#[no_mangle]
pub extern "C" fn kernel_main(boot_info: &'static BootInfo) -> ! {
    // Initialize serial output first — all diagnostics depend on it
    serial::init();

    serial::serial_println!("[  0.000000] RACORE: RacOS kernel starting");
    serial::serial_println!("[  0.000001] RACORE: Build {}", env!("CARGO_PKG_VERSION"));

    // Validate boot info
    boot::validate(boot_info);

    serial::serial_println!("[  0.000010] RACORE: Boot info validated (magic OK, version {})", boot_info.version);

    // Report memory
    let usable_bytes = boot::count_usable_memory(boot_info);
    serial::serial_println!(
        "[  0.000020] RACORE: Memory detected: {} MiB usable",
        usable_bytes / (1024 * 1024)
    );

    // Initialize architecture-specific structures
    arch::init();

    serial::serial_println!("[  0.000100] RACORE: Arch init complete (GDT, IDT)");

    // TODO: Phase C — Physical memory manager
    // TODO: Phase C — Virtual memory manager + higher-half remap
    // TODO: Phase C — Kernel heap allocator
    // TODO: Phase C — Timer init
    // TODO: Phase C — Scheduler init
    // TODO: Phase D — Syscall init
    // TODO: Phase E — VFS + init process creation

    serial::serial_println!("[  0.000200] RACORE: Entering idle loop");

    // Idle loop — halt until interrupt, repeat forever
    idle_loop()
}

/// Halts the CPU in a loop, waking only on interrupts.
fn idle_loop() -> ! {
    loop {
        arch::halt();
    }
}
