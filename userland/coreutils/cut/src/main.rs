#![no_std]
#![no_main]

use libc_lite;

/// cut — remove sections from each line
/// Usage: cut -f FIELD [-d DELIM] [file...]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut field = 1usize;
    let mut delim = '\t'; // Default tab
    let mut file_idx = 0;

    // Parse arguments
    let mut i = 1;
    while i < argc {
        let arg_ptr = unsafe { *argv.add(i as usize) };
        if arg_ptr.is_null() {
            i += 1;
            continue;
        }
        let mut arg_len = 0usize;
        unsafe {
            while *arg_ptr.add(arg_len) != 0 {
                arg_len += 1;
            }
        }
        let arg_bytes = unsafe { core::slice::from_raw_parts(arg_ptr, arg_len) };

        if arg_bytes.len() > 0 && arg_bytes[0] == b'-' {
            match arg_bytes.get(1) {
                Some(&b'f') => {
                    // Next argument is field number
                    if i + 1 < argc {
                        i += 1;
                        let num_ptr = unsafe { *argv.add(i as usize) };
                        let mut num_len = 0;
                        unsafe {
                            while *num_ptr.add(num_len) != 0 {
                                num_len += 1;
                            }
                        }
                        if let Some(n) =
                            parse_int(unsafe { core::slice::from_raw_parts(num_ptr, num_len) })
                        {
                            field = n as usize;
                        }
                    }
                }
                Some(&b'd') => {
                    // Next argument is delimiter
                    if i + 1 < argc {
                        i += 1;
                        let delim_ptr = unsafe { *argv.add(i as usize) };
                        if !delim_ptr.is_null() && unsafe { *delim_ptr != 0 } {
                            delim = unsafe { *delim_ptr } as char;
                        }
                    }
                }
                _ => {}
            }
        } else {
            file_idx = i;
            break;
        }
        i += 1;
    }

    // Process files or stdin
    if file_idx == 0 || file_idx >= argc {
        cut_fd(0, field, delim);
    } else {
        for idx in file_idx..argc {
            let file_ptr = unsafe { *argv.add(idx as usize) };
            if file_ptr.is_null() {
                continue;
            }
            let mut len = 0;
            unsafe {
                while *file_ptr.add(len) != 0 {
                    len += 1;
                }
            }
            let path = unsafe { core::slice::from_raw_parts(file_ptr, len + 1) };
            match libc_lite::open(path, 0, 0) {
                Ok(fd) => {
                    cut_fd(fd, field, delim);
                    let _ = libc_lite::close(fd);
                }
                Err(_) => {
                    let _ = libc_lite::write(2, b"cut: ");
                    let _ = libc_lite::write(2, &path[..len]);
                    let _ = libc_lite::write(2, b": No such file or directory\n");
                }
            }
        }
    }

    0
}

fn cut_fd(fd: i32, field: usize, delim: char) {
    let mut buf = [0u8; 512];
    let mut line_buf = [0u8; 512];
    let mut line_pos = 0usize;

    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' {
                        // Process line
                        if line_pos > 0 {
                            process_line(&line_buf[..line_pos], field, delim);
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

    // Process final line if no newline
    if line_pos > 0 {
        process_line(&line_buf[..line_pos], field, delim);
        let _ = libc_lite::write(1, b"\n");
    }
}

fn process_line(line: &[u8], field_num: usize, delim: char) {
    let delim_byte = delim as u8;
    let mut field_count = 1;
    let mut field_start = 0;

    for (pos, &b) in line.iter().enumerate() {
        if b == delim_byte {
            if field_count == field_num {
                let _ = libc_lite::write(1, &line[field_start..pos]);
                return;
            }
            field_count += 1;
            field_start = pos + 1;
        }
    }

    // Last field
    if field_count == field_num {
        let _ = libc_lite::write(1, &line[field_start..]);
    }
}

fn parse_int(s: &[u8]) -> Option<u32> {
    let mut result: u32 = 0;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            result = result * 10 + (b - b'0') as u32;
        } else {
            return None;
        }
    }
    Some(result)
}
