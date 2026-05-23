#![no_std]
#![no_main]

use libc_lite;

/// head — output the first N lines of a file (default 10).
///
/// Usage: head [-n N] [FILE]
/// Reads from stdin if no FILE is given.
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
                    let _ = libc_lite::write(2, b"head: cannot open file\n");
                    return 1;
                }
            }
        }
        None => 0, // stdin
    };

    let mut lines = 0usize;
    let mut buf = [0u8; 512];

    'outer: loop {
        let n = match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        for j in 0..n {
            let _ = libc_lite::write(1, &buf[j..j + 1]);
            if buf[j] == b'\n' {
                lines += 1;
                if lines >= max_lines {
                    break 'outer;
                }
            }
        }
    }

    if fd != 0 {
        let _ = libc_lite::close(fd);
    }
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
