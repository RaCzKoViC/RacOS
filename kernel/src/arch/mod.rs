// RaCore — Architecture-specific code (x86_64)
//
// This module provides the arch layer for x86_64:
// GDT, IDT, boot entry point, and CPU primitives.

pub mod gdt;
pub mod idt;

// Kernel entry point — assembly stub that sets up the stack and calls kernel_main
core::arch::global_asm!(
    r#"
.section .bss
.align 16
stack_bottom:
    .skip 65536
stack_top:

.section .text
.global _start
.extern kernel_main

_start:
    // RDI already contains the BootInfo pointer from the bootloader
    // Set up the kernel stack
    lea rsp, [rip + stack_top]

    // Clear RFLAGS
    push 0
    popf

    // Align stack to 16 bytes (System V ABI requirement)
    and rsp, ~0xF

    // Call Rust kernel_main(boot_info)
    call kernel_main

    // kernel_main is divergent, but just in case:
.Lhalt_loop:
    cli
    hlt
    jmp .Lhalt_loop
"#
);

/// Initialize architecture-specific structures.
pub fn init() {
    gdt::init();
    idt::init();
}

/// Halt the CPU until the next interrupt.
#[inline(always)]
pub fn halt() {
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack));
    }
}
