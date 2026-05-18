#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;
use alloc::string::String;
use libc_lite;

/// sort — sort lines
/// Usage: sort [-r] [file...]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut reverse = false;
    let mut file_idx = 0;

    // Parse arguments
    for i in 1..argc {
        let arg_ptr = unsafe { *argv.add(i as usize) };
        if arg_ptr.is_null() { continue; }
        let mut arg_len = 0;
        unsafe { while *arg_ptr.add(arg_len) != 0 { arg_len += 1; } }
        let arg_bytes = unsafe { core::slice::from_raw_parts(arg_ptr, arg_len) };

        if arg_bytes.len() > 0 && arg_bytes[0] == b'-' {
            if arg_bytes.len() > 1 && arg_bytes[1] == b'r' {
                reverse = true;
            }
        } else {
            file_idx = i;
            break;
        }
    }

    // Read all lines
    let mut lines = Vec::new();
    if file_idx == 0 || file_idx >= argc {
        read_lines(0, &mut lines);
    } else {
        for idx in file_idx..argc {
            let file_ptr = unsafe { *argv.add(idx as usize) };
            if file_ptr.is_null() { continue; }
            let mut len = 0;
            unsafe { while *file_ptr.add(len) != 0 { len += 1; } }
            let path = unsafe { core::slice::from_raw_parts(file_ptr, len + 1) };
            match libc_lite::open(path, 0, 0) {
                Ok(fd) => {
                    read_lines(fd, &mut lines);
                    let _ = libc_lite::close(fd);
                }
                Err(_) => {
                    let _ = libc_lite::write(2, b"sort: ");
                    let _ = libc_lite::write(2, &path[..len]);
                    let _ = libc_lite::write(2, b": No such file or directory\n");
                }
            }
        }
    }

    // Sort lines
    bubble_sort_lines(&mut lines);
    if reverse {
        lines.reverse();
    }

    // Output sorted lines
    for line in lines {
        let _ = libc_lite::write(1, line.as_bytes());
        let _ = libc_lite::write(1, b"\n");
    }

    0
}

fn read_lines(fd: i32, lines: &mut Vec<String>) {
    let mut buf = [0u8; 512];
    let mut line_buf = String::new();

    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => {
                if !line_buf.is_empty() {
                    lines.push(line_buf.clone());
                    line_buf.clear();
                }
                break;
            }
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' {
                        lines.push(line_buf.clone());
                        line_buf.clear();
                    } else {
                        line_buf.push(b as char);
                    }
                }
            }
            Err(_) => break,
        }
    }

    if !line_buf.is_empty() {
        lines.push(line_buf);
    }
}

fn bubble_sort_lines(lines: &mut Vec<String>) {
    if lines.len() <= 1 {
        return;
    }

    for i in 0..lines.len() {
        for j in 0..lines.len() - i - 1 {
            if lines[j] > lines[j + 1] {
                lines.swap(j, j + 1);
            }
        }
    }
}
