#![no_std]
#![no_main]

use libc_lite;

/// dirname — strip last component from path.
///
/// Usage: dirname PATH
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let _ = libc_lite::write(2, b"dirname: missing operand\n");
        return 1;
    }

    let path = unsafe { arg_name(argv, 1) };

    // Strip trailing slashes
    let mut end = path.len();
    while end > 1 && path[end - 1] == b'/' {
        end -= 1;
    }

    // Find last '/'
    let mut last_slash = None;
    for i in 0..end {
        if path[i] == b'/' {
            last_slash = Some(i);
        }
    }

    match last_slash {
        Some(0) => {
            let _ = libc_lite::write(1, b"/\n");
        }
        Some(pos) => {
            let _ = libc_lite::write(1, &path[..pos]);
            let _ = libc_lite::write(1, b"\n");
        }
        None => {
            let _ = libc_lite::write(1, b".\n");
        }
    }
    0
}

unsafe fn arg_name(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 { len += 1; }
    core::slice::from_raw_parts(ptr, len)
}
