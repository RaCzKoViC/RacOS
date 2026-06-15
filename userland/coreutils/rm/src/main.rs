#![no_std]
#![no_main]

use libc_lite;

/// rm — remove files or empty directories (-d).
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let _ = libc_lite::write(2, b"rm: missing operand\n");
        return 1;
    }

    let mut status = 0i32;
    for i in 1..argc {
        let arg = unsafe { arg_bytes(argv, i as usize) };
        if libc_lite::unlink(arg).is_err() {
            let _ = libc_lite::write(2, b"rm: cannot remove '");
            let name = unsafe { arg_name(argv, i as usize) };
            let _ = libc_lite::write(2, name);
            let _ = libc_lite::write(2, b"'\n");
            status = 1;
        }
    }
    status
}

/// Get arg as bytes including null terminator (for syscalls).
unsafe fn arg_bytes(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len + 1)
}

/// Get arg as bytes WITHOUT null terminator (for display).
unsafe fn arg_name(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}
