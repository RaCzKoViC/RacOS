#![no_std]
#![no_main]

use libc_lite;

/// grep — print lines from input matching a pattern (literal string).
///
/// Usage: grep [-i] PATTERN [FILE]
/// Simple literal string matching (no regex). -i for case-insensitive.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let _ = libc_lite::write(2, b"grep: missing pattern\n");
        return 1;
    }

    let mut case_insensitive = false;
    let mut pattern_idx = 1;
    let mut file_idx = None;

    if argc > 1 {
        let first = unsafe { arg_name(argv, 1) };
        if first == b"-i" {
            case_insensitive = true;
            pattern_idx = 2;
        }
    }

    if (pattern_idx as i32) >= argc {
        let _ = libc_lite::write(2, b"grep: missing pattern\n");
        return 1;
    }

    let pattern = unsafe { arg_name(argv, pattern_idx) };

    if (pattern_idx as i32 + 1) < argc {
        file_idx = Some(pattern_idx + 1);
    }

    let fd = match file_idx {
        Some(idx) => {
            let path = unsafe { arg_bytes(argv, idx) };
            match libc_lite::open(path, 0, 0) {
                Ok(fd) => fd,
                Err(_) => {
                    let _ = libc_lite::write(2, b"grep: cannot open file\n");
                    return 1;
                }
            }
        }
        None => 0,
    };

    let mut buf = [0u8; 512];
    let mut line = [0u8; 512];
    let mut line_len = 0usize;

    loop {
        let n = match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        for i in 0..n {
            if buf[i] == b'\n' {
                if buf_contains(&line[..line_len], pattern, case_insensitive) {
                    let _ = libc_lite::write(1, &line[..line_len]);
                    let _ = libc_lite::write(1, b"\n");
                }
                line_len = 0;
            } else if line_len < 512 - 1 {
                line[line_len] = buf[i];
                line_len += 1;
            }
        }
    }

    if fd != 0 {
        let _ = libc_lite::close(fd);
    }
    0
}

fn buf_contains(haystack: &[u8], needle: &[u8], case_insensitive: bool) -> bool {
    // Simple regex matching: * matches any sequence, ? matches any char
    simple_regex_match(haystack, needle, case_insensitive)
}

fn simple_regex_match(haystack: &[u8], pattern: &[u8], case_insensitive: bool) -> bool {
    if pattern.is_empty() {
        return true;
    }

    let mut h_idx = 0;
    let mut p_idx = 0;

    while h_idx < haystack.len() && p_idx < pattern.len() {
        let h = if case_insensitive {
            to_lower(haystack[h_idx])
        } else {
            haystack[h_idx]
        };
        let p = if case_insensitive {
            to_lower(pattern[p_idx])
        } else {
            pattern[p_idx]
        };

        match p {
            b'*' => {
                // * matches zero or more of any char
                if p_idx + 1 < pattern.len() {
                    let next_p = if case_insensitive {
                        to_lower(pattern[p_idx + 1])
                    } else {
                        pattern[p_idx + 1]
                    };
                    // Skip to next matching char
                    while h_idx < haystack.len() {
                        let h_next = if case_insensitive {
                            to_lower(haystack[h_idx])
                        } else {
                            haystack[h_idx]
                        };
                        if h_next == next_p {
                            break;
                        }
                        h_idx += 1;
                    }
                    p_idx += 1; // Move past *
                } else {
                    // * at end matches rest
                    return true;
                }
            }
            b'?' => {
                // ? matches any single char
                h_idx += 1;
                p_idx += 1;
            }
            _ => {
                if h != p {
                    return false;
                }
                h_idx += 1;
                p_idx += 1;
            }
        }
    }

    // If pattern has trailing *, it's ok
    while p_idx < pattern.len() && pattern[p_idx] == b'*' {
        p_idx += 1;
    }

    p_idx == pattern.len()
}

fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + 32
    } else {
        b
    }
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
