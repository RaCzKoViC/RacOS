// RaCore — Serial output (COM1)
//
// Provides early diagnostic output over serial port COM1 at 115200 baud.
// This is available before any other subsystem and is the primary debug channel.

use core::fmt;

const COM1_PORT: u16 = 0x3F8;

/// Initialize COM1 serial port at 115200 baud.
pub fn init() {
    // SAFETY: Writing to x86 I/O ports for COM1 initialization.
    // These ports are standard PC serial controller registers.
    unsafe {
        outb(COM1_PORT + 1, 0x00); // Disable interrupts
        outb(COM1_PORT + 3, 0x80); // Enable DLAB (set baud rate divisor)
        outb(COM1_PORT + 0, 0x01); // Divisor low byte: 115200 baud
        outb(COM1_PORT + 1, 0x00); // Divisor high byte
        outb(COM1_PORT + 3, 0x03); // 8 bits, no parity, one stop bit
        outb(COM1_PORT + 2, 0xC7); // Enable FIFO, clear, 14-byte threshold
        outb(COM1_PORT + 4, 0x0B); // IRQs enabled, RTS/DSR set
    }
}

/// Write a single byte to COM1, waiting for transmit buffer to be empty.
fn write_byte(byte: u8) {
    // SAFETY: Reading/writing COM1 I/O ports for serial transmission.
    unsafe {
        // Wait for transmit holding register to be empty
        while (inb(COM1_PORT + 5) & 0x20) == 0 {}
        outb(COM1_PORT, byte);
    }
}

/// Serial writer implementing `core::fmt::Write`.
pub struct SerialWriter;

impl fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}

/// Print to serial output.
macro_rules! serial_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!(crate::serial::SerialWriter, $($arg)*);
    }};
}

/// Print to serial output with newline.
macro_rules! serial_println {
    () => {{
        crate::serial::serial_print!("\n");
    }};
    ($($arg:tt)*) => {{
        crate::serial::serial_print!($($arg)*);
        crate::serial::serial_print!("\n");
    }};
}

pub(crate) use serial_print;
pub(crate) use serial_println;

// --- Low-level I/O port access ---

/// Write a byte to an x86 I/O port.
///
/// # Safety
/// Caller must ensure the port number is valid and the write is safe.
#[inline(always)]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

/// Read a byte from an x86 I/O port.
///
/// # Safety
/// Caller must ensure the port number is valid.
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
