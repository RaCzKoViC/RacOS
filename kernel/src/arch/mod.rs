// RaCore — Architecture-specific code (x86_64)
//
// This module provides the arch layer for x86_64:
// GDT, IDT, boot entry point, and CPU primitives.

pub mod acpi;
pub mod ap;
pub mod gdt;
pub mod idt;
pub mod lapic;
pub mod percpu;
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
    enable_smep_smap();
}

/// CPUID feature bits we care about for ring-0 / ring-3 hardening:
/// CPUID.(EAX=07h, ECX=0):EBX bit 7  → SMEP support
/// CPUID.(EAX=07h, ECX=0):EBX bit 20 → SMAP support
#[derive(Clone, Copy)]
struct CpuFeatures {
    smep: bool,
    smap: bool,
}

fn detect_features() -> CpuFeatures {
    // CPUID.0:EAX gives the maximum standard leaf. Bail out early if leaf 7
    // isn't supported (very old CPUs / minimal hypervisor models).
    let max_leaf: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {tmp:e}, ebx",
            "pop rbx",
            inout("eax") 0u32 => max_leaf,
            out("ecx") _, out("edx") _,
            tmp = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
    if max_leaf < 7 {
        return CpuFeatures { smep: false, smap: false };
    }
    // CPUID leaf 7, sub-leaf 0 → feature bits in EBX.
    let ebx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx:e}, ebx",
            "pop rbx",
            inout("eax") 7u32 => _,
            inout("ecx") 0u32 => _,
            out("edx") _,
            ebx = out(reg) ebx,
            options(nostack, preserves_flags),
        );
    }
    CpuFeatures {
        smep: (ebx & (1 << 7))  != 0,
        smap: (ebx & (1 << 20)) != 0,
    }
}

/// Enable SMEP (Supervisor Mode Execution Prevention, CR4.bit20) and
/// SMAP (Supervisor Mode Access Prevention, CR4.bit21) when the host CPU
/// supports them.
///
/// - SMEP causes a #PF when ring-0 tries to execute a page mapped U=1 (the
///   classic "ret2usr" defense). Free win — we never intentionally jump
///   into user pages from kernel mode, and SYSRET / IRETQ to ring 3 don't
///   count as "ring-0 execution of a user page".
///
/// - SMAP causes a #PF when ring-0 reads/writes a U=1 page without
///   AC=1 in RFLAGS. The syscall entry stub wraps the dispatcher in
///   STAC/CLAC so handlers can still touch user buffers; everywhere else
///   in the kernel (IRQ handlers, scheduler, network stack) is now blocked
///   from touching user memory by accident.
fn enable_smep_smap() {
    let f = detect_features();
    if !f.smep && !f.smap {
        crate::serial::serial_println!(
            "[  0.000660] RACORE: CPU exposes neither SMEP nor SMAP (CPUID.7:EBX), staying in bring-up mode"
        );
        return;
    }
    let mut bits: u64 = 0;
    if f.smep { bits |= 1 << 20; }
    if f.smap { bits |= 1 << 21; }
    unsafe {
        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
        cr4 |= bits;
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack));
    }
    crate::serial::serial_println!(
        "[  0.000660] RACORE: CR4 hardening: SMEP={} SMAP={}",
        f.smep, f.smap,
    );
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
