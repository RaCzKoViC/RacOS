#![no_std]
#![no_main]

use libc_lite;

/// tee — read from stdin and write to stdin + file(s)
/// Usage: tee [file...]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut fds = [0i32; 8];
    let mut fd_count = 0;

    // Open all output files
    for i in 1..argc {
        if fd_count >= 8 {
            break;
        }
        let file_ptr = unsafe { *argv.add(i as usize) };
        if file_ptr.is_null() { continue; }
        let mut len = 0;
        unsafe { while *file_ptr.add(len) != 0 { len += 1; } }
        let path = unsafe { core::slice::from_raw_parts(file_ptr, len + 1) };
        match libc_lite::open(path, 0x0601, 0o644) {
            Ok(fd) => {
                fds[fd_count] = fd;
                fd_count += 1;
            }
            Err(_) => {
                let _ = libc_lite::write(2, b"tee: ");
                let _ = libc_lite::write(2, &path[..len]);
                let _ = libc_lite::write(2, b": cannot open for writing\n");
            }
        }
    }

    // Copy stdin to stdout and files
    let mut buf = [0u8; 512];
    loop {
        match libc_lite::read(0, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                // Write to stdout
                let _ = libc_lite::write(1, &buf[..n]);
                // Write to each file
                for i in 0..fd_count {
                    let _ = libc_lite::write(fds[i as usize], &buf[..n]);
                }
            }
            Err(_) => break,
        }
    }

    // Close files
    for i in 0..fd_count {
        let _ = libc_lite::close(fds[i as usize]);
    }

    0
}
