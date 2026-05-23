#![no_std]
#![no_main]

use libc_lite;

/// uniq — remove duplicate adjacent lines
/// Usage: uniq [file]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let fd = if argc > 1 {
        let file_ptr = unsafe { *argv.add(1) };
        if file_ptr.is_null() {
            0
        } else {
            let mut len = 0;
            unsafe {
                while *file_ptr.add(len) != 0 {
                    len += 1;
                }
            }
            let path = unsafe { core::slice::from_raw_parts(file_ptr, len + 1) };
            match libc_lite::open(path, 0, 0) {
                Ok(fd) => fd,
                Err(_) => {
                    let _ = libc_lite::write(2, b"uniq: ");
                    let _ = libc_lite::write(2, &path[..len]);
                    let _ = libc_lite::write(2, b": No such file or directory\n");
                    return 1;
                }
            }
        }
    } else {
        0
    };

    uniq_fd(fd);

    if fd != 0 {
        let _ = libc_lite::close(fd);
    }

    0
}

fn uniq_fd(fd: i32) {
    let mut buf = [0u8; 512];
    let mut prev_line = [0u8; 512];
    let mut prev_len = 0usize;
    let mut line_buf = [0u8; 512];
    let mut line_pos = 0usize;

    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => {
                if line_pos > 0 {
                    output_if_unique(&line_buf[..line_pos], &mut prev_line, &mut prev_len);
                }
                break;
            }
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' {
                        if line_pos > 0 {
                            output_if_unique(&line_buf[..line_pos], &mut prev_line, &mut prev_len);
                        }
                        let _ = libc_lite::write(1, b"\n");
                        line_pos = 0;
                    } else if line_pos < line_buf.len() {
                        line_buf[line_pos] = b;
                        line_pos += 1;
                    }
                }
            }
            Err(_) => break,
        }
    }
}

fn output_if_unique(line: &[u8], prev_line: &mut [u8], prev_len: &mut usize) {
    let is_unique = if *prev_len != line.len() {
        true
    } else {
        !lines_equal(&prev_line[..*prev_len], line)
    };

    if is_unique {
        let _ = libc_lite::write(1, line);

        // Update previous line
        for (i, &b) in line.iter().enumerate() {
            if i < prev_line.len() {
                prev_line[i] = b;
            }
        }
        *prev_len = line.len();
    }
}

fn lines_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        if a[i] != b[i] {
            return false;
        }
    }
    true
}
