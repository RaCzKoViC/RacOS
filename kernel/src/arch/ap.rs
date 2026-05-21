// RaCore - AP bring-up (Phase G.3)
//
// Brings every enabled Application Processor from the BIOS reset state
// (16-bit real mode, CS:IP = vector:0) all the way to a Rust idle halt
// loop. The transition real -> protected -> long mode lives in a
// trampoline blob we copy into low physical memory (page 0x8000 = 32
// KiB) so the STARTUP IPI can target it with the 8-bit page-number
// vector the spec gives us.
//
// One AP at a time: the trampoline page can only host one bring-up
// sequence (it carries the per-AP stack pointer + Rust entry as
// parameters). BSP fires INIT-SIPI-SIPI, polls the per-CPU `started`
// flag with a timeout, then moves to the next AP.

#![allow(static_mut_refs)]

use core::sync::atomic::Ordering;

use crate::arch::{lapic, smp};
use crate::mm::phys::{self, FRAME_SIZE};

/// Fixed low-memory page where the trampoline lives. STARTUP IPI vector
/// 0x08 -> CPU resets to CS=0x0800, IP=0x0000 -> linear 0x8000.
const TRAMPOLINE_PHYS: u64 = 0x8000;

/// Page-vector form the STARTUP IPI expects: physical address >> 12.
const TRAMPOLINE_VECTOR: u8 = (TRAMPOLINE_PHYS >> 12) as u8;

/// Per-AP stack: 16 KiB (4 frames) is plenty for the early idle loop and
/// any IRQ that fires before we hand the AP a real task stack later.
const AP_STACK_FRAMES: usize = 4;
const AP_STACK_BYTES: usize  = AP_STACK_FRAMES * FRAME_SIZE;

// ── Trampoline blob (real -> protected -> long mode) ──────────────────────
//
// Position-DEPENDENT: every load/jump that resolves a label is computed
// as (TRAMPOLINE_PHYS + (label - trampoline_start)) so the bytes have to
// be copied to exactly TRAMPOLINE_PHYS to run correctly. Parameter
// slots (cr3_value / ap_stack_top / ap_entry_addr) are written by the
// BSP between IPIs.

core::arch::global_asm!(r#"
.section .rodata.ap_trampoline, "a"
.global ap_trampoline_start
.global ap_trampoline_end
.global ap_trampoline_cr3_off
.global ap_trampoline_stack_off
.global ap_trampoline_entry_off

ap_trampoline_start:

// LLVM's Intel-syntax mem-operand parser rejects (label - label) inside
// [...], so every label-relative offset is precomputed as a single .set
// constant. Effective address is then [0x8000 + CONSTANT].
.set OFF_PM32,      ap_pm32_entry - ap_trampoline_start
.set OFF_LM64,      ap_lm64_entry - ap_trampoline_start
.set OFF_GDT32_PTR, ap_gdt32_ptr  - ap_trampoline_start
.set OFF_GDT64_PTR, ap_gdt64_ptr  - ap_trampoline_start
.set OFF_GDT_TABLE, ap_gdt_table  - ap_trampoline_start
.set OFF_CR3_VAL,   ap_cr3_value  - ap_trampoline_start
.set OFF_STACK_TOP, ap_stack_top  - ap_trampoline_start
.set OFF_ENTRY,     ap_entry_addr - ap_trampoline_start

.code16
    cli
    cld
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    // Load the 32-bit GDT (6-byte descriptor at gdt32_ptr).
    lgdt [0x8000 + OFF_GDT32_PTR]

    // Enable PE: CR0.bit0
    mov eax, cr0
    or  al, 1
    mov cr0, eax

    // Far jump to 32-bit code segment 0x08.
    .byte 0x66, 0xEA                  // 32-bit operand-size ljmp imm32:imm16
    .long 0x8000 + OFF_PM32
    .word 0x08

.code32
ap_pm32_entry:
    // Load 32-bit data segments
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    // Enable PAE: CR4.bit5
    mov eax, cr4
    or  eax, (1 << 5)
    mov cr4, eax

    // Load BSP's CR3 (BSP wrote it into cr3_value before STARTUP IPI).
    mov eax, [0x8000 + OFF_CR3_VAL]
    mov cr3, eax

    // Set IA32_EFER.LME (bit 8) to arm long mode.
    mov ecx, 0xC0000080
    rdmsr
    or  eax, (1 << 8)
    wrmsr

    // Enable paging: CR0.bit31 | CR0.bit0 (PE stays on).
    mov eax, cr0
    or  eax, (1 << 31) | (1 << 0)
    mov cr0, eax

    // Switch to the 64-bit GDT (10-byte descriptor for long mode).
    lgdt [0x8000 + OFF_GDT64_PTR]

    // Far jump to 64-bit code segment 0x18.
    .byte 0xEA                        // ljmp imm32:imm16
    .long 0x8000 + OFF_LM64
    .word 0x18

.code64
ap_lm64_entry:
    // Long mode data segments — values are largely cosmetic but the
    // selectors must be valid.
    mov ax, 0x20
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    // Per-AP stack pointer (BSP wrote it into ap_stack_top before IPI).
    mov rsp, [0x8000 + OFF_STACK_TOP]

    // Tail-call into the Rust AP entry (also BSP-supplied).
    mov rax, [0x8000 + OFF_ENTRY]
    jmp rax

// ── Data area ────────────────────────────────────────────────────────────

.align 8
ap_gdt_table:
    .quad 0x0000000000000000   // 0x00: null
    .quad 0x00CF9A000000FFFF   // 0x08: 32-bit code (G=1, D=1, exec/read)
    .quad 0x00CF92000000FFFF   // 0x10: 32-bit data (G=1, D=1, read/write)
    .quad 0x00AF9A000000FFFF   // 0x18: 64-bit code (G=1, L=1)
    .quad 0x00AF92000000FFFF   // 0x20: 64-bit data
ap_gdt_end:

.set GDT_LIMIT, ap_gdt_end - ap_gdt_table - 1

.align 8
ap_gdt32_ptr:
    .word GDT_LIMIT
    .long 0x8000 + OFF_GDT_TABLE

.align 8
ap_gdt64_ptr:
    .word GDT_LIMIT
    .quad 0x8000 + OFF_GDT_TABLE

.align 8
ap_cr3_value:
ap_trampoline_cr3_off:
    .quad 0
ap_stack_top:
ap_trampoline_stack_off:
    .quad 0
ap_entry_addr:
ap_trampoline_entry_off:
    .quad 0

ap_trampoline_end:
"#);

extern "C" {
    static ap_trampoline_start: u8;
    static ap_trampoline_end:   u8;
    static ap_trampoline_cr3_off:   u8;
    static ap_trampoline_stack_off: u8;
    static ap_trampoline_entry_off: u8;
}

// ── Public entry ──────────────────────────────────────────────────────────

/// Bring every enabled-but-not-yet-started AP up to its Rust idle loop.
/// BSP-only. Returns the number of APs that signalled alive.
///
/// # Safety
/// Must run on the BSP after `acpi::init`, `smp::init`, `lapic::init_bsp`,
/// and after the kernel has set up its page tables (CR3 read here is the
/// one APs will inherit).
pub unsafe fn bring_up_all() -> usize {
    let total = smp::cpu_count();
    if total <= 1 {
        crate::serial::serial_println!("[  0.000720] RACORE: AP bring-up - single CPU, nothing to do");
        return 0;
    }

    install_trampoline();

    // Snapshot BSP's page-table root once — every AP gets the same.
    let cr3 = read_cr3();
    let bsp_apic = smp::bsp_apic_id();

    let mut booted = 0usize;
    smp::for_each_cpu::<(), _>(|cpu| {
        if cpu.is_bsp { return None; }
        if cpu.started.load(Ordering::SeqCst) { return None; }

        match bring_up_one(cpu.apic_id, cr3) {
            Ok(()) => {
                crate::serial::serial_println!(
                    "[  0.000730] RACORE: AP apic_id={} alive (BSP was apic_id={})",
                    cpu.apic_id, bsp_apic,
                );
                booted += 1;
            }
            Err(why) => {
                crate::serial::serial_println!(
                    "[  0.000730] RACORE: AP apic_id={} FAILED to start: {}",
                    cpu.apic_id, why,
                );
            }
        }
        None
    });

    booted
}

/// Rust entry point each AP jumps into from the trampoline. Long mode,
/// per-AP stack already loaded, kernel page table active. Just mark
/// ourselves alive and halt.
#[no_mangle]
unsafe extern "C" fn ap_entry() -> ! {
    // We do NOT re-init lapic globally — the static base is already set by
    // the BSP. But the AP needs the SVR enable bit set on ITS OWN
    // controller, which is a per-CPU MMIO write. enable_for_this_ap()
    // exists for exactly that.
    enable_lapic_for_this_ap();
    let id = lapic::current_apic_id();
    // G.4 foundation: bind GS to this CPU's PerCpu slot so any future
    // per-CPU code (scheduler runqueue, IRQ tick counters, ...) just
    // works via `percpu::current()`.
    crate::arch::percpu::init_for_this_cpu(id);
    smp::mark_started(id);

    // Park the AP. Interrupts stay off — we have no per-CPU IDT/TSS yet,
    // taking an IRQ here would either re-enter the BSP's handlers on the
    // wrong stack or just triple-fault on the missing IST entries.
    loop {
        core::arch::asm!("hlt", options(nomem, nostack));
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Copy the trampoline blob to the fixed low-memory page. Idempotent.
unsafe fn install_trampoline() {
    let src = &ap_trampoline_start as *const u8;
    let end = &ap_trampoline_end as *const u8;
    let len = end.offset_from(src) as usize;
    debug_assert!(len <= FRAME_SIZE, "trampoline {} > 4 KiB", len);
    core::ptr::copy_nonoverlapping(src, TRAMPOLINE_PHYS as *mut u8, len);
}

/// Where (cr3_value / ap_stack_top / ap_entry_addr) live inside the copied
/// trampoline page. Offsets are computed at link time; we re-derive them
/// against the source blob and apply them to the destination.
unsafe fn slot_addr(slot: &u8) -> *mut u64 {
    let off = (slot as *const u8).offset_from(&ap_trampoline_start as *const u8) as usize;
    (TRAMPOLINE_PHYS as usize + off) as *mut u64
}

unsafe fn bring_up_one(apic_id: u32, cr3: u64) -> Result<(), &'static str> {
    // Per-AP kernel stack (16 KiB). Top points one past the last byte so
    // the first `push` writes a fully-aligned 16-byte slot.
    let frame = phys::alloc_contiguous(AP_STACK_FRAMES).map_err(|_| "no frames for AP stack")?;
    let stack_top = frame.addr() + AP_STACK_BYTES as u64;

    // Write per-AP parameters into the trampoline page.
    *slot_addr(&ap_trampoline_cr3_off)   = cr3;
    *slot_addr(&ap_trampoline_stack_off) = stack_top;
    *slot_addr(&ap_trampoline_entry_off) = ap_entry as u64;
    // Make sure the writes are visible before the AP starts spinning.
    core::sync::atomic::fence(Ordering::SeqCst);

    // Intel SDM Section 8.4.4 "Multiple-Processor Initialization": send
    // INIT, wait ~10 ms, send STARTUP, wait ~200 us, send STARTUP again.
    lapic::send_init_ipi(apic_id)?;
    busy_wait_us(10_000);
    lapic::send_startup_ipi(apic_id, TRAMPOLINE_VECTOR)?;
    busy_wait_us(200);
    lapic::send_startup_ipi(apic_id, TRAMPOLINE_VECTOR)?;

    // Poll for the AP marking itself started. Generous timeout — TCG
    // QEMU runs much slower than real silicon.
    for _ in 0..50 {
        if let Some(cpu) = find_cpu(apic_id) {
            if cpu.started.load(Ordering::SeqCst) {
                return Ok(());
            }
        }
        busy_wait_us(10_000);
    }
    Err("AP did not signal started within 500 ms")
}

fn find_cpu(apic_id: u32) -> Option<&'static smp::CpuState> {
    smp::for_each_cpu::<&smp::CpuState, _>(|cpu| {
        if cpu.apic_id == apic_id {
            // SAFETY: smp owns the slot for the kernel lifetime.
            Some(unsafe { &*(cpu as *const smp::CpuState) })
        } else {
            None
        }
    })
}

/// Read this CPU's CR3.
unsafe fn read_cr3() -> u64 {
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
    cr3
}

/// Coarse, calibration-free busy wait. We assume "at least a million
/// instructions / ms" on any post-2000 host; QEMU TCG is much slower than
/// that, which only makes the SDM-mandated delays longer (safe).
fn busy_wait_us(us: u32) {
    let iters = us.saturating_mul(1_000);
    for _ in 0..iters {
        core::hint::spin_loop();
    }
}

/// AP-side LAPIC enable. The BSP's `lapic::init_bsp` set the module
/// statics + put the spurious vector together; here on the AP we only
/// have to flip SVR.APIC_ENABLE on the controller we're physically
/// running on. Lives here (not in lapic.rs) because it's intimately tied
/// to AP boot and uses the module's MMIO base.
unsafe fn enable_lapic_for_this_ap() {
    const LAPIC_BASE: u64 = 0xFEE0_0000;
    const SVR_OFFSET: usize = 0x0F0;
    const SVR_ENABLE: u32   = 1 << 8;
    const SPURIOUS:   u32   = 0xFF;
    let svr = (LAPIC_BASE as *mut u8).add(SVR_OFFSET) as *mut u32;
    let cur = core::ptr::read_volatile(svr);
    core::ptr::write_volatile(svr, cur | SVR_ENABLE | SPURIOUS);
}
