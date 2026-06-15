#![no_std]
#![no_main]
#![deny(unsafe_code)]

use libc_lite::{close, open, print, println, read, write};

fn print_u64_right(n: u64, width: usize) {
    let mut digits = [0u8; 20];
    let mut i = 0;
    let mut v = n;
    if v == 0 {
        digits[0] = b'0';
        i = 1;
    } else {
        while v > 0 {
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            i += 1;
        }
    }
    // Pad with spaces on the left.
    let pad = width.saturating_sub(i);
    for _ in 0..pad {
        let _ = write(1, b" ");
    }
    let mut buf = [0u8; 20];
    for j in 0..i {
        buf[j] = digits[i - 1 - j];
    }
    let _ = write(1, &buf[..i]);
}

fn print_str_left(s: &str, width: usize) {
    let _ = write(1, s.as_bytes());
    let pad = width.saturating_sub(s.len());
    for _ in 0..pad {
        let _ = write(1, b" ");
    }
}

/// df — show per-mount block usage. Reads /proc/diskstats.
///
/// Output columns (block = 512 B):
///   Mountpoint     1K-blocks       Used      Available   Use%   Inodes_free
#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = match open(b"/proc/diskstats\0", 0, 0) {
        Ok(fd) => fd,
        Err(_) => {
            println("df: cannot open /proc/diskstats");
            return 1;
        }
    };

    let mut raw = [0u8; 2048];
    let mut total_read = 0usize;
    loop {
        match read(fd, &mut raw[total_read..]) {
            Ok(0) => break,
            Ok(n) => {
                total_read += n;
                if total_read >= raw.len() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = close(fd);

    let text = match core::str::from_utf8(&raw[..total_read]) {
        Ok(s) => s,
        Err(_) => {
            println("df: invalid utf8 in /proc/diskstats");
            return 1;
        }
    };

    // Header.
    print_str_left("Mountpoint", 14);
    print_str_left("Blocks", 12);
    print_str_left("Used", 12);
    print_str_left("Free", 12);
    print_str_left("Use%", 8);
    print_str_left("Inodes-free", 14);
    print("\n");

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Fields: mount total used free total_ino free_ino
        let mut it = line.split_ascii_whitespace();
        let mount = match it.next() {
            Some(s) => s,
            None => continue,
        };
        let total: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let used: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let free: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let _ti: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let fi: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);

        print_str_left(mount, 14);
        print_u64_right(total, 11);
        print(" ");
        print_u64_right(used, 11);
        print(" ");
        print_u64_right(free, 11);
        print(" ");
        let pct = if total > 0 { (used * 100) / total } else { 0 };
        print_u64_right(pct, 5);
        print("%  ");
        print_u64_right(fi, 12);
        print("\n");
    }
    0
}
