#![no_std]
#![no_main]

use libc_lite;

/// mkdir — create directories.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let _ = libc_lite::write(2, b"mkdir: missing operand\n");
        return 1;
    }

    let mut status = 0i32;
    for i in 1..argc {
        let arg = unsafe { arg_str(argv, i as usize) };
        if libc_lite::mkdir(arg, 0o755).is_err() {
            let _ = libc_lite::write(2, b"mkdir: cannot create directory '");
            let _ = libc_lite::write(2, arg);
            let _ = libc_lite::write(2, b"'\n");
            status = 1;
        }
    }
    status
}

unsafe fn arg_str(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 { len += 1; }
    // Include null terminator for syscall
    core::slice::from_raw_parts(ptr, len + 1)
}
