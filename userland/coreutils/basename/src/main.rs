#![no_std]
#![no_main]

use libc_lite;

/// basename — strip directory and optional suffix from path.
///
/// Usage: basename PATH [SUFFIX]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let _ = libc_lite::write(2, b"basename: missing operand\n");
        return 1;
    }

    let path = unsafe { arg_name(argv, 1) };
    let suffix = if argc >= 3 {
        Some(unsafe { arg_name(argv, 2) })
    } else {
        None
    };

    // Find last '/'
    let mut base_start = 0;
    for i in 0..path.len() {
        if path[i] == b'/' {
            base_start = i + 1;
        }
    }
    let mut base = &path[base_start..];

    // Strip trailing '/' (for paths like "/foo/bar/")
    while base.len() > 1 && base[base.len() - 1] == b'/' {
        base = &base[..base.len() - 1];
    }

    // Strip suffix
    if let Some(suf) = suffix {
        if base.len() > suf.len() && base.ends_with(suf) {
            base = &base[..base.len() - suf.len()];
        }
    }

    let _ = libc_lite::write(1, base);
    let _ = libc_lite::write(1, b"\n");
    0
}

unsafe fn arg_name(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}
