#![no_std]
#![no_main]

use libc_lite;

/// echo — wypisz argumenty na stdout.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // Print argv[1..] separated by spaces, followed by newline
    for i in 1..argc {
        if i > 1 {
            let _ = libc_lite::write(1, b" ");
        }
        let arg_ptr = unsafe { *argv.add(i as usize) };
        if !arg_ptr.is_null() {
            // Find length of null-terminated string
            let mut len = 0usize;
            unsafe {
                while *arg_ptr.add(len) != 0 {
                    len += 1;
                }
                let _ = libc_lite::write(1, core::slice::from_raw_parts(arg_ptr, len));
            }
        }
    }
    let _ = libc_lite::write(1, b"\n");
    0
}
