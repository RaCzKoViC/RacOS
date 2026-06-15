// RaCore — PS/2 Keyboard Driver
//
// Simple PS/2 keyboard driver for x86_64.
// Processes scancodes and handles VT switching (Alt+F1-F6).

use crate::tty::vt;
use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

/// Data port.
const PS2_DATA: u16 = 0x60;
/// Status/Command port.
const PS2_STATUS: u16 = 0x64;

/// Scancodes for Alt keys.
const SC_LALT: u8 = 0x38;
const SC_LALT_RELEASE: u8 = 0xB8;

/// Scancodes for F1-F12.
const SC_F1: u8 = 0x3B;
const SC_F2: u8 = 0x3C;
const SC_F3: u8 = 0x3D;
const SC_F4: u8 = 0x3E;
const SC_F5: u8 = 0x3F;
const SC_F6: u8 = 0x40;

/// State of special keys.
static mut LALT_PRESSED: bool = false;
static mut LSHIFT_PRESSED: bool = false;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    Irq = 0,
    Polling = 1,
}

static INPUT_MODE: AtomicU8 = AtomicU8::new(InputMode::Irq as u8);
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(true);
static IRQ_COUNT: AtomicU64 = AtomicU64::new(0);

/// Simple US-English QWERTY mapping (Set 1).
const SCANCODE_MAP: [u8; 128] = [
    0, 27, b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'0', b'-', b'=', 8, b'\t', b'q',
    b'w', b'e', b'r', b't', b'y', b'u', b'i', b'o', b'p', b'[', b']', b'\n', 0, b'a', b's', b'd',
    b'f', b'g', b'h', b'j', b'k', b'l', b';', b'\'', b'`', 0, b'\\', b'z', b'x', b'c', b'v', b'b',
    b'n', b'm', b',', b'.', b'/', 0, b'*', 0, b' ', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    b'-', 0, 0, 0, b'+', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const SCANCODE_MAP_SHIFT: [u8; 128] = [
    0, 27, b'!', b'@', b'#', b'$', b'%', b'^', b'&', b'*', b'(', b')', b'_', b'+', 8, b'\t', b'Q',
    b'W', b'E', b'R', b'T', b'Y', b'U', b'I', b'O', b'P', b'{', b'}', b'\n', 0, b'A', b'S', b'D',
    b'F', b'G', b'H', b'J', b'K', b'L', b':', b'"', b'~', 0, b'|', b'Z', b'X', b'C', b'V', b'B',
    b'N', b'M', b'<', b'>', b'?', 0, b'*', 0, b' ', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    b'-', 0, 0, 0, b'+', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const SC_LSHIFT: u8 = 0x2A;
const SC_RSHIFT: u8 = 0x36;
const SC_LSHIFT_RELEASE: u8 = 0xAA;
const SC_RSHIFT_RELEASE: u8 = 0xB6;

/// Handle a scancode received from the PS/2 controller.
///
/// # Safety
/// Called from interrupt context.
pub unsafe fn handle_scancode(code: u8) {
    match code {
        SC_LALT => LALT_PRESSED = true,
        SC_LALT_RELEASE => LALT_PRESSED = false,
        SC_LSHIFT | SC_RSHIFT => LSHIFT_PRESSED = true,
        SC_LSHIFT_RELEASE | SC_RSHIFT_RELEASE => LSHIFT_PRESSED = false,
        _ => {
            if LALT_PRESSED {
                match code {
                    SC_F1 => vt::get_manager().switch_to(0),
                    SC_F2 => vt::get_manager().switch_to(1),
                    SC_F3 => vt::get_manager().switch_to(2),
                    SC_F4 => vt::get_manager().switch_to(3),
                    SC_F5 => vt::get_manager().switch_to(4),
                    SC_F6 => vt::get_manager().switch_to(5),
                    _ => {}
                }
            } else if code < 128 {
                let ascii = if LSHIFT_PRESSED {
                    SCANCODE_MAP_SHIFT[code as usize]
                } else {
                    SCANCODE_MAP[code as usize]
                };

                if ascii != 0 {
                    // Feed keyboard input into the shared console input stream.
                    // Rendering/echo is handled by the shell line editor.
                    crate::serial::push_input_byte(ascii);
                }
            }
        }
    }

    if debug_enabled() {
        crate::serial::serial_println!(
            "[ KBD ] mode={:?} irq_count={} scancode=0x{:02X}",
            input_mode(),
            irq_count(),
            code
        );
    }
}

pub fn set_debug(enabled: bool) {
    DEBUG_ENABLED.store(enabled, Ordering::Release);
}

pub fn debug_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::Acquire)
}

pub fn irq_count() -> u64 {
    IRQ_COUNT.load(Ordering::Acquire)
}

pub fn input_mode() -> InputMode {
    match INPUT_MODE.load(Ordering::Acquire) {
        1 => InputMode::Polling,
        _ => InputMode::Irq,
    }
}

pub fn set_input_mode(mode: InputMode) {
    INPUT_MODE.store(mode as u8, Ordering::Release);
    match mode {
        InputMode::Irq => crate::interrupts::pic::enable_irq(1),
        InputMode::Polling => crate::interrupts::pic::disable_irq(1),
    }
    crate::serial::serial_println!("[ KBD ] Input mode set to {:?}", mode);
}

/// Handle keyboard input when running in IRQ mode.
pub fn handle_irq_input() {
    if input_mode() != InputMode::Irq {
        return;
    }

    IRQ_COUNT.fetch_add(1, Ordering::AcqRel);
    unsafe {
        if is_data_available() {
            let scancode = read_scancode();
            handle_scancode(scancode);
        }
    }
}

/// Poll PS/2 input when running in polling mode.
///
/// # Safety
/// Must be called from a non-interrupt context.
pub unsafe fn poll_input() {
    if input_mode() != InputMode::Polling {
        return;
    }

    while is_data_available() {
        let scancode = read_scancode();
        handle_scancode(scancode);
    }
}

/// Read a scancode directly from the hardware.
pub unsafe fn read_scancode() -> u8 {
    let mut code: u8;
    asm!("in al, dx", out("al") code, in("dx") PS2_DATA, options(nomem, nostack));
    code
}

/// Helper to check if data is available.
pub unsafe fn is_data_available() -> bool {
    let mut status: u8;
    asm!("in al, dx", out("al") status, in("dx") PS2_STATUS, options(nomem, nostack));
    (status & 1) != 0
}
