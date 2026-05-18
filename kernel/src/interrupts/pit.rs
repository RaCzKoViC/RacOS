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
pub fn init() {
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
