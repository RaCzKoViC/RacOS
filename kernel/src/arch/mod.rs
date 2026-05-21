// RaCore — Architecture-specific code (x86_64)
//
// This module provides the arch layer for x86_64:
// GDT, IDT, boot entry point, and CPU primitives.

pub mod gdt;
pub mod idt;
pub mod acpi;
pub mod smp;

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
    and rsp, -16

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
///
/// Note: ACPI/MADT discovery is intentionally *not* invoked here — it needs
/// the bootloader's RSDP address and the kernel heap (for parsed topology
/// vectors). Call `acpi::init(rsdp_addr)` from `kernel_main` once the heap
/// is up.
pub fn init() {
    gdt::init();
    idt::init();
    crate::serial::serial_println!("[  0.000660] RACORE: SMEP/SMAP skipped (bring-up mode)");
}

/// Enable SMEP (Supervisor Mode Execution Prevention, CR4.bit20) and
/// SMAP (Supervisor Mode Access Prevention, CR4.bit21) for security.
fn enable_smep_smap() {
    unsafe {
        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        // SMEP = bit 20, SMAP = bit 21
        cr4 |= (1 << 20) | (1 << 21);
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));
    }
    crate::serial::serial_println!("[  0.000050] RACORE: SMEP and SMAP enabled");
}

/// Halt the CPU until the next interrupt.
#[inline(always)]
pub fn halt() {
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack));
    }
}

// --- Low-level I/O port access ---

/// Write a byte to an x86 I/O port.
#[inline(always)]
pub unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

/// Read a byte from an x86 I/O port.
#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!(
        "in al, dx",
        in("dx") port,
        out("al") value,
        options(nomem, nostack, preserves_flags)
    );
    value
}

/// Write a u32 to an x86 I/O port.
#[inline(always)]
pub unsafe fn outl(port: u16, value: u32) {
    core::arch::asm!(
        "out dx, eax",
        in("dx") port,
        in("eax") value,
        options(nomem, nostack, preserves_flags)
    );
}

/// Read a u32 from an x86 I/O port.
#[inline(always)]
pub unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    core::arch::asm!(
        "in eax, dx",
        in("dx") port,
        out("eax") value,
        options(nomem, nostack, preserves_flags)
    );
    value
}

/// Write a u16 to an x86 I/O port.
#[inline(always)]
pub unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!(
        "out dx, ax",
        in("dx") port,
        in("ax") value,
        options(nomem, nostack, preserves_flags)
    );
}

/// Read a u16 from an x86 I/O port.
#[inline(always)]
pub unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!(
        "in ax, dx",
        in("dx") port,
        out("ax") value,
        options(nomem, nostack, preserves_flags)
    );
    value
}
