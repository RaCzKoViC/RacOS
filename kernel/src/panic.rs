// RaCore — Panic handler
//
// On panic, prints the message to serial and halts all CPUs.
// No reboot — allow operator to inspect serial log.

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::serial::serial_println!();
    crate::serial::serial_println!("!!! KERNEL PANIC !!!");

    if let Some(location) = info.location() {
        crate::serial::serial_println!(
            "  at {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        );
    }

    crate::serial::serial_println!("  {}", info.message());

    crate::serial::serial_println!("!!! HALTING !!!");

    // Halt all CPUs: disable interrupts, then halt in loop
    loop {
        unsafe {
            core::arch::asm!("cli; hlt", options(nomem, nostack));
        }
    }
}
