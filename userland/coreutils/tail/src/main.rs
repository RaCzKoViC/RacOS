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
        } else if is_numeric_flag(arg) {
            // `tail -5` is the shorthand everyone actually types. Without this
            // it fell through to the FILE branch and was opened as a path.
            max_lines = parse_usize(&arg[1..]);
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

    if max_lines == 0 {
        return 0;
    }

    // A trailing newline terminates the last line rather than starting a new
    // one, so it must not be counted. Counting it made `tail -1` find its
    // boundary immediately and emit an empty slice.
    let mut end = total;
    if end > 0 && data[end - 1] == b'\n' {
        end -= 1;
    }

    // Walk back until we have crossed max_lines newlines; the byte after the
    // last one is where the output starts. Fewer newlines than asked for means
    // the whole input is the answer, so `start` stays 0.
    let mut start = 0usize;
    let mut newlines = 0usize;
    let mut i = end;
    while i > 0 {
        i -= 1;
        if data[i] == b'\n' {
            newlines += 1;
            if newlines == max_lines {
                start = i + 1;
                break;
            }
        }
    }

    let _ = libc_lite::write(1, &data[start..total]);
    0
}

/// True for `-1`, `-25`, ... -- a dash followed by at least one digit and
/// nothing else. `-n` and a bare `-` are not numeric flags.
fn is_numeric_flag(arg: &[u8]) -> bool {
    arg.len() > 1 && arg[0] == b'-' && arg[1..].iter().all(|b| b.is_ascii_digit())
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
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}

unsafe fn arg_bytes(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len + 1)
}
