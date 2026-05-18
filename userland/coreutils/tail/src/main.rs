#![no_std]
#![no_main]

use libc_lite;

/// tail — output the last N lines of input (default 10).
///
/// Usage: tail [-n N] [FILE]
/// Reads from stdin if no FILE given. Buffers all input (max 8 KiB).
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut max_lines: usize = 10;
    let mut file_arg: Option<usize> = None;
    let mut i = 1usize;

    while (i as i32) < argc {
        let arg = unsafe { arg_name(argv, i) };
        if arg == b"-n" {
            i += 1;
            if (i as i32) < argc {
                let n_arg = unsafe { arg_name(argv, i) };
                max_lines = parse_usize(n_arg);
            }
        } else if arg.starts_with(b"-n") {
            max_lines = parse_usize(&arg[2..]);
        } else {
            file_arg = Some(i);
        }
        i += 1;
    }

    let fd = match file_arg {
        Some(idx) => {
            let path = unsafe { arg_bytes(argv, idx) };
            match libc_lite::open(path, 0, 0) {
                Ok(fd) => fd,
                Err(_) => {
                    let _ = libc_lite::write(2, b"tail: cannot open file\n");
                    return 1;
                }
            }
        }
        None => 0,
    };

    // Read all input into fixed buffer (8 KiB)
    let mut data = [0u8; 8192];
    let mut total = 0usize;
    let mut buf = [0u8; 512];
    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let copy = n.min(data.len() - total);
                data[total..total + copy].copy_from_slice(&buf[..copy]);
                total += copy;
                if total >= data.len() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    if fd != 0 {
        let _ = libc_lite::close(fd);
    }

    // Find start of last N lines
    let mut line_count = 0usize;
    let mut pos = total;
    while pos > 0 && line_count < max_lines {
        pos -= 1;
        if data[pos] == b'\n' {
            line_count += 1;
        }
    }
    // Adjust: if we stopped at a newline, skip it (we want content after it)
    if pos > 0 && data[pos] == b'\n' {
        pos += 1;
    }

    let _ = libc_lite::write(1, &data[pos..total]);
    0
}

fn parse_usize(s: &[u8]) -> usize {
    let mut val: usize = 0;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as usize;
        } else {
            break;
        }
    }
    val
}

unsafe fn arg_name(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 { len += 1; }
    core::slice::from_raw_parts(ptr, len)
}

unsafe fn arg_bytes(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 { len += 1; }
    core::slice::from_raw_parts(ptr, len + 1)
}
