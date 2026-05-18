use core::fmt;
use crate::fb_console;
use crate::serial;

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::print::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use fmt::Write;
    
    // Write to Serial (Always)
    let _ = serial::SerialWriter.write_fmt(args);
    
    // Write to Framebuffer Console (If available) without heap allocation.
    let _ = KernelWriter.write_fmt(args);
}

pub struct KernelWriter;

impl fmt::Write for KernelWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        fb_console::fb_print(s);
        Ok(())
    }
}
