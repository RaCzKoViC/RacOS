#![no_std]
#![no_main]

use libc_lite;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let _ = libc_lite::write(2, b"sed: missing command\n");
        return 1;
    }

    let script = unsafe { arg_name(argv, 1) };
    if script.is_empty() {
        let _ = libc_lite::write(2, b"sed: empty script\n");
        return 1;
    }

    // Simple MVP: only s/pattern/replacement/ command
    if !script.starts_with(b"s/") || !script.ends_with(b"/") {
        let _ = libc_lite::write(2, b"sed: only s/pattern/replacement/ supported\n");
        return 1;
    }

    let inner = &script[2..script.len() - 1];
    let parts: &[&[u8]] = &inner.split(|&b| b == b'/').collect::<alloc::vec::Vec<_>>();
    if parts.len() != 2 {
        let _ = libc_lite::write(2, b"sed: invalid s/pattern/replacement/\n");
        return 1;
    }

    let pattern = parts[0];
    let replacement = parts[1];

    // Read from stdin
    let mut buf = [0u8; 1024];
    loop {
        match libc_lite::read(0, &mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                let data = &buf[..n];
                let lines = data.split(|&b| b == b'\n');
                for line in lines {
                    if line.is_empty() && n > 0 && data.last() != Some(&b'\n') {
                        continue; // incomplete line
                    }
                    let processed = replace_pattern(line, pattern, replacement);
                    let _ = libc_lite::write(1, &processed);
                    let _ = libc_lite::write(1, b"\n");
                }
            }
            Err(_) => {
                let _ = libc_lite::write(2, b"sed: read error\n");
                return 1;
            }
        }
    }

    0
}

fn replace_pattern(line: &[u8], pattern: &[u8], replacement: &[u8]) -> alloc::vec::Vec<u8> {
    if pattern.is_empty() {
        return line.to_vec();
    }

    let mut result = alloc::vec::Vec::new();
    let mut i = 0;
    while i < line.len() {
        if line[i..].starts_with(pattern) {
            result.extend_from_slice(replacement);
            i += pattern.len();
        } else {
            result.push(line[i]);
            i += 1;
        }
    }
    result
}

unsafe fn arg_name(argv: *const *const u8, idx: usize) -> &'static [u8] {
    let ptr = *argv.offset(idx as isize);
    if ptr.is_null() {
        return b"";
    }
    let mut len = 0;
    while *ptr.offset(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}

extern crate alloc;