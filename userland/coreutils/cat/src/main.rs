#![no_std]
#![no_main]

use libc_lite;

/// cat — kopiuj pliki (lub stdin) na stdout.
/// Bez argumentów czyta stdin. Z argumentami czyta podane pliki.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        // No arguments — copy stdin to stdout
        cat_fd(0);
    } else {
        for i in 1..argc {
            let arg_ptr = unsafe { *argv.add(i as usize) };
            if arg_ptr.is_null() {
                continue;
            }
            // Find length
            let mut len = 0usize;
            unsafe {
                while *arg_ptr.add(len) != 0 {
                    len += 1;
                }
            }
            if len == 1 && unsafe { *arg_ptr } == b'-' {
                cat_fd(0); // "-" means stdin
            } else {
                // Build null-terminated path
                let path = unsafe { core::slice::from_raw_parts(arg_ptr, len + 1) }; // includes null
                match libc_lite::open(path, 0, 0) {
                    Ok(fd) => {
                        cat_fd(fd);
                        let _ = libc_lite::close(fd);
                    }
                    Err(_) => {
                        let _ = libc_lite::write(2, b"cat: ");
                        let _ = libc_lite::write(2, &path[..len]);
                        let _ = libc_lite::write(2, b": No such file or directory\n");
                    }
                }
            }
        }
    }
    0
}

fn cat_fd(fd: i32) {
    let mut buf = [0u8; 512];
    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let _ = libc_lite::write(1, &buf[..n]);
            }
            Err(_) => break,
        }
    }
}
