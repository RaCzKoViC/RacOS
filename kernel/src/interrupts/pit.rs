// RaCore — PIT (Programmable Interval Timer) driver
//
// Configures the 8253/8254 PIT channel 0 to fire IRQ0 at ~1000 Hz.
// Provides the kernel tick counter for the scheduler time quantum.
//
// Design decisions (ADR-007):
// - PIT for MVP (simple, universally available in QEMU)
// - HPET/TSC for higher precision post-MVP

use core::sync::atomic::{AtomicU64, Ordering};

/// PIT I/O ports.
const PIT_CHANNEL_0: u16 = 0x40;
const PIT_COMMAND: u16 = 0x43;

/// PIT base frequency: 1,193,182 Hz.
const PIT_BASE_FREQ: u32 = 1_193_182;

/// Target frequency: 1000 Hz (1 ms per tick).
const TARGET_FREQ: u32 = 1000;

/// PIT divisor for target frequency.
const DIVISOR: u16 = (PIT_BASE_FREQ / TARGET_FREQ) as u16;

/// Global tick counter, incremented by the timer IRQ handler.
static TICK_COUNT: AtomicU64 = AtomicU64::new(0);

/// Initialize the PIT to fire at ~1000 Hz.
///
/// First disables HPET LegacyReplacement so PIT IRQ0 isn't intercepted/replaced
/// by HPET's own counter. Without this, on QEMU `-machine q35` (HPET enabled
/// by default) IRQ0 fires at HPET's compat rate (~18.2 Hz, the PIT default)
/// instead of our configured 1000 Hz — /proc/uptime reads ~0.06s after 3s of
/// real time. Same code path is a no-op on hardware without HPET (vendor
/// reads as 0 or 0xFFFF and we bail).
pub fn init() {
    unsafe { disable_hpet_legacy(); }

    // Command: channel 0, lo/hi byte, rate generator (mode 2)
    let command: u8 = 0x34; // 00 11 010 0 = channel 0, lo/hi, mode 2, binary
    // SAFETY: Writing PIT I/O ports to configure the timer.
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") PIT_COMMAND,
            in("al") command,
            options(nomem, nostack, preserves_flags)
        );
        // Low byte of divisor
        core::arch::asm!(
            "out dx, al",
            in("dx") PIT_CHANNEL_0,
            in("al") (DIVISOR & 0xFF) as u8,
            options(nomem, nostack, preserves_flags)
        );
        // High byte of divisor
        core::arch::asm!(
            "out dx, al",
            in("dx") PIT_CHANNEL_0,
            in("al") ((DIVISOR >> 8) & 0xFF) as u8,
            options(nomem, nostack, preserves_flags)
        );
    }

    crate::serial::serial_println!(
        "[  0.000300] RACORE: PIT initialized ({} Hz, divisor {})",
        TARGET_FREQ,
        DIVISOR
    );
}

/// Called from the timer IRQ handler (vector 32).
/// Increments the global tick counter.
pub fn tick() {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Get the current tick count.
pub fn ticks() -> u64 {
    TICK_COUNT.load(Ordering::Relaxed)
}

/// Approximate milliseconds since boot.
pub fn uptime_ms() -> u64 {
    ticks() // Each tick ≈ 1 ms at 1000 Hz
}

/// HPET MMIO base on QEMU q35 and most physical hardware that exposes HPET
/// at all (it's chipset-dependent; ICH9 and successors put it here). On
/// future systems we should look this up via the ACPI HPET table.
const HPET_BASE: u64 = 0xFED00000;
const HPET_GEN_CAPS_OFFSET: u64 = 0x000;
const HPET_GEN_CFG_OFFSET: u64 = 0x010;
const HPET_CFG_ENABLE_CNF: u64 = 1 << 0;
const HPET_CFG_LEG_RT_CNF: u64 = 1 << 1;

/// Probe and disable HPET so the PIT can own IRQ0 at the divisor we set.
///
/// QEMU q35 (the machine type we target) has HPET enabled by default in
/// LegacyReplacement mode, which routes HPET timer 0 to IRQ0 instead of the
/// PIT. The HPET counter is configured by firmware at ~18.2 Hz, so our PIT
/// `out` to set divisor=1193 (1000 Hz) ran but nothing on the bus actually
/// generated those interrupts — kernel tick counter accumulated ~20/sec.
///
/// # Safety
/// Reads/writes raw MMIO. The HPET region is identity-mapped by UEFI before
/// kernel handover; if it weren't, the volatile read here would page-fault
/// and we'd see it in the kernel exception path (not silently corrupt).
unsafe fn disable_hpet_legacy() {
    let caps_ptr = (HPET_BASE + HPET_GEN_CAPS_OFFSET) as *const u64;
    let caps = core::ptr::read_volatile(caps_ptr);
    // Vendor ID lives in bits 16..32. All-zeros or all-ones means "no
    // device responded" — either no HPET, or this MMIO region isn't mapped.
    let vendor = (caps >> 16) & 0xFFFF;
    if vendor == 0 || vendor == 0xFFFF {
        crate::serial::serial_println!(
            "[  0.000290] RACORE: HPET not present (caps=0x{:X}, skipping disable)",
            caps,
        );
        return;
    }

    let cfg_ptr = (HPET_BASE + HPET_GEN_CFG_OFFSET) as *mut u64;
    let cfg = core::ptr::read_volatile(cfg_ptr);
    let new_cfg = cfg & !(HPET_CFG_ENABLE_CNF | HPET_CFG_LEG_RT_CNF);
    core::ptr::write_volatile(cfg_ptr, new_cfg);

    crate::serial::serial_println!(
        "[  0.000290] RACORE: HPET disabled (vendor=0x{:X}, cfg 0x{:X} -> 0x{:X})",
        vendor, cfg, new_cfg,
    );
}
