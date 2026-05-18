// RaCore — Interrupt subsystem
//
// Manages the 8259 PIC, PIT timer, and IRQ dispatching.
// PIC is remapped to vectors 32-47 to avoid conflicts with CPU exceptions (0-31).

pub mod pic;
pub mod pit;

/// Keyboard IRQ.
const IRQ_KEYBOARD: u8 = 1;

/// Initialize interrupts.
pub fn init() {
    pic::init();
    pit::init();

    // Enable keyboard interrupt
    pic::enable_irq(IRQ_KEYBOARD);
}
