#![no_std]
#![no_main]

use libc_lite;

/// mv — move or rename file
/// Usage: mv source dest
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        let _ = libc_lite::write(2, b"mv: missing operand\n");
        return 1;
    }

    let src_ptr = unsafe { *argv.add(1) };
    let dst_ptr = unsafe { *argv.add(2) };

    if src_ptr.is_null() || dst_ptr.is_null() {
        let _ = libc_lite::write(2, b"mv: null path\n");
        return 1;
    }

    // Get lengths
    let mut src_len = 0usize;
    unsafe { while *src_ptr.add(src_len) != 0 { src_len += 1; } }
    let mut dst_len = 0usize;
    unsafe { while *dst_ptr.add(dst_len) != 0 { dst_len += 1; } }

    let src_path = unsafe { core::slice::from_raw_parts(src_ptr, src_len + 1) };
    let dst_path = unsafe { core::slice::from_raw_parts(dst_ptr, dst_len + 1) };

    // Copy source to destination
    match libc_lite::open(src_path, 0, 0) {
        Ok(src_fd) => {
            match libc_lite::open(dst_path, 0x0601, 0o644) {
                Ok(dst_fd) => {
                    // Copy data
                    let mut buf = [0u8; 4096];
                    let mut success = true;
                    loop {
                        match libc_lite::read(src_fd, &mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                match libc_lite::write(dst_fd, &buf[..n]) {
                                    Ok(_) => {}
                                    Err(_) => {
                                        success = false;
                                        break;
                                    }
                                }
                            }
                            Err(_) => {
                                success = false;
                                break;
                            }
                        }
                    }
                    let _ = libc_lite::close(src_fd);
                    let _ = libc_lite::close(dst_fd);

                    if success {
                        // Delete source after successful copy
                        match libc_lite::unlink(src_path) {
                            Ok(()) => 0,
                            Err(_) => {
                                let _ = libc_lite::write(2, b"mv: cannot unlink source\n");
                                1
                            }
                        }
                    } else {
                        let _ = libc_lite::write(2, b"mv: copy failed\n");
                        1
                    }
                }
                Err(_) => {
                    let _ = libc_lite::write(2, b"mv: ");
                    let _ = libc_lite::write(2, &dst_path[..dst_len]);
                    let _ = libc_lite::write(2, b": cannot create file\n");
                    let _ = libc_lite::close(src_fd);
                    1
                }
            }
        }
        Err(_) => {
            let _ = libc_lite::write(2, b"mv: ");
            let _ = libc_lite::write(2, &src_path[..src_len]);
            let _ = libc_lite::write(2, b": No such file or directory\n");
            1
        }
    }
}
