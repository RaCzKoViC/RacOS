#![no_std]
#![no_main]

use libc_lite;

/// find — search for files in a directory hierarchy
/// Usage: find [path] [-name pattern]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut start_path = ".";
    let mut name_pattern: Option<&str> = None;

    // Parse arguments
    let mut i = 1;
    while i < argc {
        let arg_ptr = unsafe { *argv.add(i as usize) };
        if arg_ptr.is_null() {
            i += 1;
            continue;
        }
        let mut arg_len = 0;
        unsafe {
            while *arg_ptr.add(arg_len) != 0 {
                arg_len += 1;
            }
        }
        let arg_bytes = unsafe { core::slice::from_raw_parts(arg_ptr, arg_len) };

        if arg_bytes.len() > 0 && arg_bytes[0] == b'-' {
            if arg_bytes.len() > 1 && arg_bytes[1] == b'n' && arg_bytes.len() > 5 {
                // -name pattern
                if i + 1 < argc {
                    i += 1;
                    let pat_ptr = unsafe { *argv.add(i as usize) };
                    let mut pat_len = 0;
                    unsafe {
                        while *pat_ptr.add(pat_len) != 0 {
                            pat_len += 1;
                        }
                    }
                    if let Ok(s) = core::str::from_utf8(unsafe {
                        core::slice::from_raw_parts(pat_ptr, pat_len)
                    }) {
                        name_pattern = Some(s);
                    }
                }
            }
        } else {
            if let Ok(s) = core::str::from_utf8(arg_bytes) {
                start_path = s;
            }
        }
        i += 1;
    }

    // Perform find
    if let Ok(path) = core::str::from_utf8(start_path.as_bytes()) {
        find_recursive(path, name_pattern);
    }

    0
}

fn find_recursive(path: &str, name_pattern: Option<&str>) {
    // Build null-terminated path for open
    let mut path_buf = [0u8; 512];
    if path.len() >= path_buf.len() - 1 {
        return;
    }
    for (i, &b) in path.as_bytes().iter().enumerate() {
        path_buf[i] = b;
    }
    path_buf[path.len()] = 0;

    match libc_lite::open(&path_buf, 0, 0) {
        Ok(fd) => {
            let mut buf = [0u8; 4096];
            match libc_lite::getdents(fd, &mut buf) {
                Ok(nbytes) => {
                    let mut offset = 0;
                    while offset + 10 <= nbytes {
                        let file_type = buf[offset + 8];
                        let name_len = buf[offset + 9] as usize;
                        if offset + 10 + name_len > nbytes {
                            break;
                        }
                        let name_bytes = &buf[offset + 10..offset + 10 + name_len];
                        let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_len);

                        if name_end > 0 && name_end <= name_len {
                            if let Ok(name) = core::str::from_utf8(&name_bytes[..name_end]) {
                                // Skip . and ..
                                if name != "." && name != ".." {
                                    // Build full path
                                    let mut full_path_buf = [0u8; 512];
                                    let full_len = if path.ends_with('/') {
                                        path.len() + name.len()
                                    } else {
                                        path.len() + 1 + name.len()
                                    };

                                    if full_len < full_path_buf.len() {
                                        let mut pos = 0;
                                        for &b in path.as_bytes() {
                                            full_path_buf[pos] = b;
                                            pos += 1;
                                        }
                                        if !path.ends_with('/') {
                                            full_path_buf[pos] = b'/';
                                            pos += 1;
                                        }
                                        for &b in name.as_bytes() {
                                            full_path_buf[pos] = b;
                                            pos += 1;
                                        }

                                        let full_path = core::str::from_utf8(&full_path_buf[..pos])
                                            .unwrap_or("");

                                        // Match pattern if provided
                                        let matches = if let Some(pat) = name_pattern {
                                            glob_simple_match(name, pat)
                                        } else {
                                            true
                                        };

                                        if matches {
                                            let _ = libc_lite::write(1, full_path.as_bytes());
                                            let _ = libc_lite::write(1, b"\n");
                                        }

                                        // Recurse into directories
                                        if file_type == 2 {
                                            // DT_DIR
                                            find_recursive(full_path, name_pattern);
                                        }
                                    }
                                }
                            }
                        }

                        offset += 10 + name_len;
                    }
                }
                Err(_) => {}
            }
            let _ = libc_lite::close(fd);
        }
        Err(_) => {}
    }
}

fn glob_simple_match(name: &str, pattern: &str) -> bool {
    let name_bytes = name.as_bytes();
    let pat_bytes = pattern.as_bytes();
    glob_match_recursive(name_bytes, pat_bytes)
}

fn glob_match_recursive(name: &[u8], pattern: &[u8]) -> bool {
    if pattern.is_empty() {
        return name.is_empty();
    }
    if pattern.len() == 1 && pattern[0] == b'*' {
        return true;
    }

    match pattern[0] {
        b'*' => {
            for i in 0..=name.len() {
                if glob_match_recursive(&name[i..], &pattern[1..]) {
                    return true;
                }
            }
            false
        }
        b'?' => {
            if name.is_empty() {
                false
            } else {
                glob_match_recursive(&name[1..], &pattern[1..])
            }
        }
        c => {
            if name.is_empty() || name[0] != c {
                false
            } else {
                glob_match_recursive(&name[1..], &pattern[1..])
            }
        }
    }
}
