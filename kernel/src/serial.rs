// RaCore — Serial I/O (COM1)
//
// Provides diagnostic output and input over serial port COM1 at 115200 baud.
// Output is available before any other subsystem and is the primary debug channel.
// Input is buffered via IRQ4 and made available to userland via /dev/console.

use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

const COM1_PORT: u16 = 0x3F8;

/// Ring buffer for serial input (filled by IRQ4 handler).
const INPUT_BUF_SIZE: usize = 1024;
static mut INPUT_BUF: [u8; INPUT_BUF_SIZE] = [0; INPUT_BUF_SIZE];
static INPUT_HEAD: AtomicUsize = AtomicUsize::new(0); // write position (IRQ handler)
static INPUT_TAIL: AtomicUsize = AtomicUsize::new(0); // read position (consumer)

/// Initialize COM1 serial port at 115200 baud.
pub fn init() {
    // SAFETY: Writing to x86 I/O ports for COM1 initialization.
    // These ports are standard PC serial controller registers.
    unsafe {
        crate::arch::outb(COM1_PORT + 1, 0x00); // Disable interrupts
        crate::arch::outb(COM1_PORT + 3, 0x80); // Enable DLAB (set baud rate divisor)
        crate::arch::outb(COM1_PORT + 0, 0x01); // Divisor low byte: 115200 baud
        crate::arch::outb(COM1_PORT + 1, 0x00); // Divisor high byte
        crate::arch::outb(COM1_PORT + 3, 0x03); // 8 bits, no parity, one stop bit
        crate::arch::outb(COM1_PORT + 2, 0xC7); // Enable FIFO, clear, 14-byte threshold
        crate::arch::outb(COM1_PORT + 4, 0x0B); // IRQs enabled, RTS/DSR set
                                                // Enable Received Data Available interrupt
        crate::arch::outb(COM1_PORT + 1, 0x01);
    }
}

/// Check if there is data available in the serial input buffer.
pub fn input_available() -> bool {
    INPUT_HEAD.load(Ordering::Acquire) != INPUT_TAIL.load(Ordering::Acquire)
}

/// Read one byte from the serial input buffer. Returns None if empty.
pub fn read_byte_nonblocking() -> Option<u8> {
    let tail = INPUT_TAIL.load(Ordering::Acquire);
    let head = INPUT_HEAD.load(Ordering::Acquire);
    if tail == head {
        return None;
    }
    // SAFETY: single consumer, IRQ handler is the single producer
    let byte = unsafe { INPUT_BUF[tail % INPUT_BUF_SIZE] };
    INPUT_TAIL.store(tail.wrapping_add(1), Ordering::Release);
    Some(byte)
}

/// Read bytes from the serial input buffer into `buf`.
/// Returns the number of bytes read (may be 0 if empty).
pub fn read_input(buf: &mut [u8]) -> usize {
    let mut count = 0;
    for slot in buf.iter_mut() {
        match read_byte_nonblocking() {
            Some(b) => {
                *slot = b;
                count += 1;
            }
            None => break,
        }
    }
    count
}

/// Inject one input byte into the console input ring.
///
/// Used by non-serial input sources (e.g. PS/2 keyboard) so userland reading
/// from /dev/console receives interactive keystrokes.
pub fn push_input_byte(byte: u8) {
    let head = INPUT_HEAD.load(Ordering::Relaxed);
    let tail = INPUT_TAIL.load(Ordering::Relaxed);
    if head.wrapping_sub(tail) < INPUT_BUF_SIZE {
        // SAFETY: single-byte store into ring slot selected from atomics.
        unsafe {
            INPUT_BUF[head % INPUT_BUF_SIZE] = byte;
        }
        INPUT_HEAD.store(head.wrapping_add(1), Ordering::Release);
    }
}

/// Called from the IRQ4 handler to receive serial data from hardware.
pub fn handle_irq() {
    // SAFETY: Reading COM1 I/O ports in interrupt context.
    unsafe {
        // Drain the FIFO
        while (crate::arch::inb(COM1_PORT + 5) & 0x01) != 0 {
            let byte = crate::arch::inb(COM1_PORT);

            // Ctrl-C (0x03) → send SIGINT to foreground process
            if byte == 0x03 {
                crate::task::scheduler::signal_foreground(crate::task::signal::Signal::SIGINT);
                continue;
            }

            let head = INPUT_HEAD.load(Ordering::Relaxed);
            let tail = INPUT_TAIL.load(Ordering::Relaxed);
            // Drop bytes if buffer full
            if head.wrapping_sub(tail) < INPUT_BUF_SIZE {
                INPUT_BUF[head % INPUT_BUF_SIZE] = byte;
                INPUT_HEAD.store(head.wrapping_add(1), Ordering::Release);
            }
        }
    }
}

/// Write a single byte to COM1, waiting for transmit buffer to be empty.       
fn write_byte(byte: u8) {
    // SAFETY: Reading/writing COM1 I/O ports for serial transmission.
    unsafe {
        // Wait for transmit holding register to be empty
        while (crate::arch::inb(COM1_PORT + 5) & 0x20) == 0 {}
        crate::arch::outb(COM1_PORT, byte);
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
