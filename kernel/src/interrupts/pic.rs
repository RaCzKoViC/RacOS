// RaCore — 8259 PIC (Programmable Interrupt Controller)
//
// Remaps the dual 8259 PIC to IRQ vectors 32-47.
// Vector 32 = IRQ0 (PIT timer)
// Vector 33 = IRQ1 (keyboard)
// Vector 40 = IRQ8 (CMOS RTC)
// etc.

/// PIC1 (master) command and data ports.
const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
/// PIC2 (slave) command and data ports.
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

/// IRQ vector offset for PIC1 (master): IRQ0 = vector 32.
pub const PIC1_OFFSET: u8 = 32;
/// IRQ vector offset for PIC2 (slave): IRQ8 = vector 40.
pub const PIC2_OFFSET: u8 = 40;

/// End-of-interrupt command.
const EOI: u8 = 0x20;

/// ICW1: initialization + ICW4 needed.
const ICW1_INIT: u8 = 0x11;
/// ICW4: 8086 mode.
const ICW4_8086: u8 = 0x01;

/// Initialize and remap the 8259 PIC.
///
/// After this, IRQ0-7 map to vectors 32-39 and IRQ8-15 map to vectors 40-47.
pub fn init() {
    // SAFETY: Standard 8259 PIC initialization sequence.
    // Writing these I/O ports configures the PIC hardware.
    unsafe {
        // Save masks
        let mask1 = inb(PIC1_DATA);
        let mask2 = inb(PIC2_DATA);

        // Start initialization sequence (ICW1)
        outb(PIC1_COMMAND, ICW1_INIT);
        io_wait();
        outb(PIC2_COMMAND, ICW1_INIT);
        io_wait();

        // ICW2: vector offsets
        outb(PIC1_DATA, PIC1_OFFSET);
        io_wait();
        outb(PIC2_DATA, PIC2_OFFSET);
        io_wait();

        // ICW3: master/slave wiring
        outb(PIC1_DATA, 4); // Slave PIC on IRQ2
        io_wait();
        outb(PIC2_DATA, 2); // Slave ID = 2
        io_wait();

        // ICW4: 8086 mode
        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();

        // Restore masks (all masked except what we explicitly enable)
        outb(PIC1_DATA, mask1);
        outb(PIC2_DATA, mask2);
    }

    crate::serial::serial_println!(
        "[  0.000080] RACORE: PIC remapped (IRQ0={}, IRQ8={})",
        PIC1_OFFSET,
        PIC2_OFFSET
    );
}

/// Enable a specific IRQ line.
pub fn enable_irq(irq: u8) {
    let port = if irq < 8 { PIC1_DATA } else { PIC2_DATA };
    let line = if irq < 8 { irq } else { irq - 8 };
    // SAFETY: Reading/writing PIC data port to unmask an IRQ line.
    unsafe {
        let mask = inb(port);
        outb(port, mask & !(1 << line));
    }
}

/// Disable a specific IRQ line.
pub fn disable_irq(irq: u8) {
    let port = if irq < 8 { PIC1_DATA } else { PIC2_DATA };
    let line = if irq < 8 { irq } else { irq - 8 };
    unsafe {
        let mask = inb(port);
        outb(port, mask | (1 << line));
    }
}

/// Send end-of-interrupt for the given IRQ.
pub fn send_eoi(irq: u8) {
    // SAFETY: Writing EOI command to PIC ports.
    unsafe {
        if irq >= 8 {
            outb(PIC2_COMMAND, EOI);
        }
        outb(PIC1_COMMAND, EOI);
    }
}

/// Mask all IRQs (disable all).
pub fn disable_all() {
    unsafe {
        outb(PIC1_DATA, 0xFF);
        outb(PIC2_DATA, 0xFF);
    }
}

// --- I/O port helpers ---

#[inline(always)]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!(
        "in al, dx",
        in("dx") port,
        out("al") value,
        options(nomem, nostack, preserves_flags)
    );
    value
}

/// Small I/O delay for PIC initialization.
#[inline(always)]
unsafe fn io_wait() {
    // Write to an unused port to create a small delay.
    outb(0x80, 0);
}
