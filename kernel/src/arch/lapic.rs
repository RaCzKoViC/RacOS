// RaCore - Local APIC (xAPIC) MMIO driver + IPI primitives (Phase G.2)
//
// G.1 parsed the MADT and listed the LAPICs. G.2 actually programs the
// running CPU's LAPIC: enable it via the Spurious Interrupt Vector
// Register, read its own ID, and expose IPI helpers that G.3 will use to
// bring up the APs.
//
// We stay in xAPIC mode (MMIO at 0xFEE00000) — x2APIC (MSR access) is
// a Phase G.5 concern. Most QEMU/Intel configurations let us drive APs
// from xAPIC just fine.

#![allow(static_mut_refs)]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

// ── Register offsets (relative to LAPIC base) ─────────────────────────────

const LAPIC_REG_ID:       usize = 0x020;
const LAPIC_REG_VERSION:  usize = 0x030;
const LAPIC_REG_TPR:      usize = 0x080;
const LAPIC_REG_EOI:      usize = 0x0B0;
const LAPIC_REG_SVR:      usize = 0x0F0;
const LAPIC_REG_ESR:      usize = 0x280;
const LAPIC_REG_ICR_LOW:  usize = 0x300;
const LAPIC_REG_ICR_HIGH: usize = 0x310;

// SVR bit 8 = APIC software enable. Spurious vector usually 0xFF.
const SVR_ENABLE: u32 = 1 << 8;
const SPURIOUS_VECTOR: u32 = 0xFF;

// ICR low encoding helpers — kept as standalone consts so the IPI helpers
// read like the Intel SDM tables they mirror.
const ICR_DELIVERY_FIXED:   u32 = 0b000 << 8;
const ICR_DELIVERY_INIT:    u32 = 0b101 << 8;
const ICR_DELIVERY_STARTUP: u32 = 0b110 << 8;
const ICR_LEVEL_ASSERT:     u32 = 1 << 14;
const ICR_LEVEL_DEASSERT:   u32 = 0;
const ICR_TRIGGER_EDGE:     u32 = 0;
const ICR_TRIGGER_LEVEL:    u32 = 1 << 15;
const ICR_STATUS_PENDING:   u32 = 1 << 12;

// MSR for the LAPIC base — used to confirm the hardware-side enable bit
// is set. UEFI almost always leaves it enabled, but we check to keep the
// failure mode explicit.
const IA32_APIC_BASE_MSR:    u32 = 0x1B;
const APIC_BASE_GLOBAL_ENABLE: u64 = 1 << 11;

// ── Module state ──────────────────────────────────────────────────────────

static LAPIC_BASE: AtomicU64 = AtomicU64::new(0);
static BSP_ID:     AtomicU32 = AtomicU32::new(0);

// ── Public API ────────────────────────────────────────────────────────────

/// Initialise the BSP's LAPIC: confirm the global enable bit in the MSR is
/// on, then turn the software enable bit on via the Spurious Interrupt
/// Vector Register. Reads back the LAPIC ID and stashes it so G.3 / smoke
/// tests can cross-check against the MADT.
///
/// # Safety
/// Must run on the BSP, after `acpi::init()` populated `lapic_addr`. The
/// returned base address is assumed identity-mapped (true for the standard
/// 0xFEE00000 region on x86_64 with the kernel's bootloader-supplied
/// identity map).
pub unsafe fn init_bsp() {
    let base = match crate::arch::acpi::get_info() {
        Some(info) => info.lapic_addr,
        None => {
            crate::serial::serial_println!(
                "[  0.000700] RACORE: LAPIC - no ACPI info, falling back to 0xFEE00000"
            );
            0xFEE0_0000
        }
    };
    LAPIC_BASE.store(base, Ordering::SeqCst);

    // 1. Sanity-check the hardware-global enable bit. We don't try to fix
    //    it because writing IA32_APIC_BASE can relocate the LAPIC and we'd
    //    rather hard-fail than silently move it under us.
    let apic_base_msr = rdmsr(IA32_APIC_BASE_MSR);
    let global_on = apic_base_msr & APIC_BASE_GLOBAL_ENABLE != 0;
    if !global_on {
        crate::serial::serial_println!(
            "[  0.000700] RACORE: LAPIC - WARNING: APIC_BASE.global_enable is OFF (MSR=0x{:016X})",
            apic_base_msr,
        );
    }

    // 2. Read raw LAPIC ID (xAPIC stores it in bits [31:24] of the ID reg).
    let raw_id = read_reg(LAPIC_REG_ID);
    let id = (raw_id >> 24) & 0xFF;
    BSP_ID.store(id, Ordering::SeqCst);

    // 3. Clear any stale error status by writing it once (W1C is per-bit
    //    on real hardware; QEMU treats it as a normal clear).
    write_reg(LAPIC_REG_ESR, 0);

    // 4. Turn on the LAPIC and route spurious interrupts to vector 0xFF.
    let svr = read_reg(LAPIC_REG_SVR);
    write_reg(LAPIC_REG_SVR, svr | SVR_ENABLE | SPURIOUS_VECTOR);

    // 5. Mask all external interrupts via TPR=0 (accept everything that
    //    isn't masked elsewhere). We'll wire LVT entries properly when we
    //    enable timer/error IRQs through the LAPIC.
    write_reg(LAPIC_REG_TPR, 0);

    let version = read_reg(LAPIC_REG_VERSION);
    let max_lvt = (version >> 16) & 0xFF;
    crate::serial::serial_println!(
        "[  0.000710] RACORE: LAPIC enabled - base=0x{:08X} id={} version=0x{:02X} max_lvt={} global_enable={}",
        base, id, version & 0xFF, max_lvt, global_on,
    );
}

/// True iff the BSP's LAPIC software enable bit is currently set.
pub fn is_enabled() -> bool {
    let base = LAPIC_BASE.load(Ordering::SeqCst);
    if base == 0 { return false; }
    unsafe { read_reg(LAPIC_REG_SVR) & SVR_ENABLE != 0 }
}

/// Read the running CPU's LAPIC ID. On the BSP this matches `bsp_id()`;
/// on an AP it returns that AP's id (useful from the trampoline's Rust
/// entry to identify which CPU just came online).
pub fn current_apic_id() -> u32 {
    let base = LAPIC_BASE.load(Ordering::SeqCst);
    if base == 0 { return 0; }
    unsafe { (read_reg(LAPIC_REG_ID) >> 24) & 0xFF }
}

pub fn bsp_id() -> u32 {
    BSP_ID.load(Ordering::SeqCst)
}

/// Signal end-of-interrupt to the LAPIC. Required after every LAPIC-routed
/// IRQ (timer, IPI, spurious, etc).
pub fn eoi() {
    let base = LAPIC_BASE.load(Ordering::SeqCst);
    if base == 0 { return; }
    unsafe { write_reg(LAPIC_REG_EOI, 0); }
}

// ── IPI primitives (consumed by G.3 AP boot) ──────────────────────────────

/// Spin until the LAPIC clears the ICR delivery-status bit, meaning the
/// previously-issued IPI has actually been sent on the bus. Times out
/// after ~1M spins so a wedged controller can't hang the kernel.
fn wait_delivery() -> Result<(), &'static str> {
    for _ in 0..1_000_000u32 {
        let v = unsafe { read_reg(LAPIC_REG_ICR_LOW) };
        if v & ICR_STATUS_PENDING == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err("LAPIC IPI delivery timed out")
}

/// Send a level-triggered INIT IPI to the given APIC ID. This is the
/// first step of the INIT-SIPI-SIPI sequence; the AP enters its reset
/// state and is ready to receive a Startup IPI.
///
/// # Safety
/// `apic_id` must be a real CPU id from the MADT and the LAPIC must be
/// initialised (`init_bsp` ran). Caller controls timing — Intel SDM
/// recommends ~10 ms after this before the Startup IPI.
pub unsafe fn send_init_ipi(apic_id: u32) -> Result<(), &'static str> {
    write_icr(apic_id, ICR_DELIVERY_INIT | ICR_LEVEL_ASSERT | ICR_TRIGGER_LEVEL);
    wait_delivery()
}

/// Issue the INIT de-assert (some chipsets require it after the assert).
/// Modern hardware/QEMU often tolerates skipping this, but keeping the
/// full sequence makes us more portable.
pub unsafe fn send_init_deassert(apic_id: u32) -> Result<(), &'static str> {
    write_icr(apic_id, ICR_DELIVERY_INIT | ICR_LEVEL_DEASSERT | ICR_TRIGGER_LEVEL);
    wait_delivery()
}

/// Send a STARTUP IPI. `vector` is the page number (0x00..0xFF) where
/// the AP's real-mode trampoline lives — the AP will start executing at
/// `vector << 12`. The trampoline must live below 1 MiB.
pub unsafe fn send_startup_ipi(apic_id: u32, vector: u8) -> Result<(), &'static str> {
    write_icr(apic_id, ICR_DELIVERY_STARTUP | ICR_LEVEL_ASSERT | ICR_TRIGGER_EDGE | vector as u32);
    wait_delivery()
}

/// Compose + dispatch an ICR write. ICR_HIGH must be written first so the
/// destination latches before the LOW write actually fires the IPI.
unsafe fn write_icr(apic_id: u32, low: u32) {
    let base = LAPIC_BASE.load(Ordering::SeqCst);
    debug_assert!(base != 0, "LAPIC base not initialised");
    write_reg(LAPIC_REG_ICR_HIGH, (apic_id & 0xFF) << 24);
    write_reg(LAPIC_REG_ICR_LOW,  low);
}

// ── MMIO + MSR helpers ────────────────────────────────────────────────────

#[inline]
unsafe fn read_reg(off: usize) -> u32 {
    let base = LAPIC_BASE.load(Ordering::SeqCst) as *mut u8;
    read_volatile(base.add(off) as *const u32)
}

#[inline]
unsafe fn write_reg(off: usize, val: u32) {
    let base = LAPIC_BASE.load(Ordering::SeqCst) as *mut u8;
    write_volatile(base.add(off) as *mut u32, val);
}

#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let (high, low): (u32, u32);
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nomem, nostack, preserves_flags),
    );
    ((high as u64) << 32) | low as u64
}
