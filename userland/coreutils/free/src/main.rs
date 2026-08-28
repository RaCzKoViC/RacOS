// free — report memory usage from /proc/meminfo (ROADMAP v0.2 §2.1).
//
// The kernel already publishes everything needed:
//
//   MemTotal:    <n> kB
//   MemFree:     <n> kB
//   MemUsed:     <n> kB
//   Buffers:     0 kB
//   Cached:      0 kB
//
// so this is a parser and a formatter, nothing more. Values are read by name
// rather than by line position: procfs is free to add fields, and a positional
// parser would start reporting the wrong numbers the day it does.
//
// Usage: free [-k | -m]     (-k kibibytes, the default; -m mebibytes)

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;
use alloc::vec::Vec;

fn read_meminfo() -> Option<String> {
    let fd = libc_lite::open(b"/proc/meminfo\0", 0, 0).ok()?;
    let mut out: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        match libc_lite::read(fd, &mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > 8192 {
                    break;
                }
            }
        }
    }
    let _ = libc_lite::close(fd);
    core::str::from_utf8(&out).ok().map(String::from)
}

/// Value of `key:` in kB, or None when the field is absent.
fn field(text: &str, key: &str) -> Option<u64> {
    for line in text.split('\n') {
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        if name.trim_end_matches(':') != key {
            continue;
        }
        return parts.next().and_then(parse_u64);
    }
    None
}

fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut n: u64 = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(n)
}

fn push_num(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut d = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        d[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        out.push(d[i] as char);
    }
}

/// Right-align `n` in a 12-column field, the way the header is spaced.
fn push_col(out: &mut String, n: u64) {
    let mut num = String::new();
    push_num(&mut num, n);
    for _ in num.len()..12 {
        out.push(' ');
    }
    out.push_str(&num);
}

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut divisor: u64 = 1;
    let mut unit = "kB";

    let n = libc_lite::arg_count(argc);
    let mut i = 1;
    while i < n {
        match libc_lite::arg(argv, i) {
            Some(b"-k") => {}
            Some(b"-m") => {
                divisor = 1024;
                unit = "MB";
            }
            Some(other) => {
                let _ = libc_lite::write(2, b"free: unknown option: ");
                let _ = libc_lite::write(2, other);
                let _ = libc_lite::write(2, b"\nusage: free [-k | -m]\n");
                return 2;
            }
            None => break,
        }
        i += 1;
    }

    let text = match read_meminfo() {
        Some(t) => t,
        None => {
            let _ = libc_lite::write(2, b"free: cannot read /proc/meminfo\n");
            return 1;
        }
    };

    let total = field(&text, "MemTotal");
    let free_kb = field(&text, "MemFree");
    if total.is_none() || free_kb.is_none() {
        let _ = libc_lite::write(2, b"free: /proc/meminfo is missing MemTotal or MemFree\n");
        return 1;
    }
    let total = total.unwrap_or(0);
    let free_kb = free_kb.unwrap_or(0);
    // Prefer the kernel's own MemUsed; fall back to the difference if a future
    // procfs drops the field.
    let used = field(&text, "MemUsed").unwrap_or(total.saturating_sub(free_kb));
    let buffers = field(&text, "Buffers").unwrap_or(0);
    let cached = field(&text, "Cached").unwrap_or(0);

    let mut out = String::new();
    out.push_str("             ");
    out.push_str(unit);
    out.push_str("       total        used        free     buffers      cached\n");
    out.push_str("Mem:   ");
    push_col(&mut out, total / divisor);
    push_col(&mut out, used / divisor);
    push_col(&mut out, free_kb / divisor);
    push_col(&mut out, buffers / divisor);
    push_col(&mut out, cached / divisor);
    out.push('\n');

    let _ = libc_lite::write(1, out.as_bytes());
    0
}
