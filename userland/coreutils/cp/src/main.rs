#![no_std]
#![no_main]

use libc_lite;

/// cp — copy file
/// Usage: cp source dest
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        let _ = libc_lite::write(2, b"cp: missing operand\n");
        return 1;
    }

    let src_ptr = unsafe { *argv.add(1) };
    let dst_ptr = unsafe { *argv.add(2) };

    if src_ptr.is_null() || dst_ptr.is_null() {
        let _ = libc_lite::write(2, b"cp: null path\n");
        return 1;
    }

    // Get lengths
    let mut src_len = 0usize;
    unsafe { while *src_ptr.add(src_len) != 0 { src_len += 1; } }
    let mut dst_len = 0usize;
    unsafe { while *dst_ptr.add(dst_len) != 0 { dst_len += 1; } }

    let src_path = unsafe { core::slice::from_raw_parts(src_ptr, src_len + 1) };
    let dst_path = unsafe { core::slice::from_raw_parts(dst_ptr, dst_len + 1) };

    // Open source file
    match libc_lite::open(src_path, 0, 0) {
        Ok(src_fd) => {
            // Create destination file (write-only, truncate, create with 0o644)
            match libc_lite::open(dst_path, 0x0601, 0o644) {  // O_WRONLY | O_CREAT | O_TRUNC
                Ok(dst_fd) => {
                    // Copy data
                    let mut buf = [0u8; 4096];
                    loop {
                        match libc_lite::read(src_fd, &mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                match libc_lite::write(dst_fd, &buf[..n]) {
                                    Ok(_) => {}
                                    Err(_) => {
                                        let _ = libc_lite::write(2, b"cp: write error\n");
                                        let _ = libc_lite::close(src_fd);
                                        let _ = libc_lite::close(dst_fd);
                                        return 1;
                                    }
                                }
                            }
                            Err(_) => {
                                let _ = libc_lite::write(2, b"cp: read error\n");
                                let _ = libc_lite::close(src_fd);
                                let _ = libc_lite::close(dst_fd);
                                return 1;
                            }
                        }
                    }
                    let _ = libc_lite::close(src_fd);
                    let _ = libc_lite::close(dst_fd);
                    0
                }
                Err(_) => {
                    let _ = libc_lite::write(2, b"cp: ");
                    let _ = libc_lite::write(2, &dst_path[..dst_len]);
                    let _ = libc_lite::write(2, b": cannot create file\n");
                    let _ = libc_lite::close(src_fd);
                    1
                }
            }
        }
        Err(_) => {
            let _ = libc_lite::write(2, b"cp: ");
            let _ = libc_lite::write(2, &src_path[..src_len]);
            let _ = libc_lite::write(2, b": No such file or directory\n");
            1
        }
    }
}
