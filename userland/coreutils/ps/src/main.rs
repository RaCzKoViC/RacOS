// ps — list running processes by walking /proc and reading
// /proc/<pid>/status for each numeric-named entry.
//
// Output: a single header line "PID PPID STATE NAME" then one row per
// process. State and name are read from /proc/<pid>/status fields
// (Name: / State: / PPid:), which kernel/src/vfs/procfs.rs already
// serves. No flags supported in v0.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

const PROC_DIR: &[u8] = b"/proc\0";

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = match libc_lite::open(PROC_DIR, 0, 0) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc_lite::write(2, b"ps: cannot open /proc\n");
            return 1;
        }
    };

    let _ = libc_lite::write(1, b"  PID   PPID  STATE     NAME\n");

    // sys_getdents in the current kernel emits ALL entries on a single
    // call (no cursor / position state — see
    // kernel/src/syscall/handlers.rs:sys_getdents). Looping over getdents
    // would re-emit the same entries forever and hang the process. Match
    // ls's pattern: one call, parse the whole buffer, done.
    let mut buf = [0u8; 4096];
    let n = match libc_lite::getdents(fd, &mut buf) {
        Ok(n) => n,
        Err(_) => {
            let _ = libc_lite::write(2, b"ps: getdents failed\n");
            let _ = libc_lite::close(fd);
            return 1;
        }
    };

    // Kernel dirent layout:
    //   [0..8)   ino: u64 LE
    //   [8]      file_type: u8  (currently always 0 — kernel-side bug
    //                            where FileType::Directory = 0o040000
    //                            gets truncated to u8 = 0; rely on the
    //                            name being numeric instead)
    //   [9]      name_len: u8
    //   [10..]   name bytes (name_len of them)
    let mut off = 0usize;
    while off + 10 <= n {
        let name_len = buf[off + 9] as usize;
        let entry_size = 10 + name_len;
        if off + entry_size > n {
            break;
        }
        let name = &buf[off + 10..off + 10 + name_len];
        if let Some(pid) = parse_pid(name) {
            print_process_info(pid);
        }
        off += entry_size;
    }

    let _ = libc_lite::close(fd);
    0
}

/// Parse the name as ASCII decimal. Returns None for `.`, `..`, `self`,
/// or anything else that isn't a pure-digit string.
fn parse_pid(name: &[u8]) -> Option<u32> {
    if name.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in name {
        if !(b'0'..=b'9').contains(&b) {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

fn print_process_info(pid: u32) {
    // Build "/proc/<pid>/status\0" without dragging alloc::format in.
    let mut path = [0u8; 32];
    let mut pos = 0usize;
    for &b in b"/proc/" {
        path[pos] = b;
        pos += 1;
    }
    pos += write_u32_into(pid, &mut path[pos..]);
    for &b in b"/status\0" {
        path[pos] = b;
        pos += 1;
    }

    let fd = match libc_lite::open(&path[..pos], 0, 0) {
        Ok(fd) => fd,
        Err(_) => return,
    };

    let mut buf = [0u8; 512];
    let n = libc_lite::read(fd, &mut buf).unwrap_or(0);
    let _ = libc_lite::close(fd);
    if n == 0 {
        return;
    }

    // Parse the Name:/State:/PPid: lines we care about. procfs format
    // (kernel/src/vfs/procfs.rs:generate_content) is tab-separated:
    //   Name:\tinit\nState:\trunning\nPid:\t1\nPPid:\t0\n...
    let mut name_bytes: &[u8] = b"?";
    let mut state_bytes: &[u8] = b"?";
    let mut ppid: u32 = 0;

    for line in buf[..n].split(|&b| b == b'\n') {
        if let Some(rest) = line.strip_prefix(b"Name:\t") {
            name_bytes = rest;
        } else if let Some(rest) = line.strip_prefix(b"State:\t") {
            state_bytes = rest;
        } else if let Some(rest) = line.strip_prefix(b"PPid:\t") {
            ppid = parse_pid(rest).unwrap_or(0);
        }
    }

    let mut row = [b' '; 64];
    let mut p = 0usize;
    p += pad_u32(pid, 5, &mut row[p..]);
    row[p] = b' ';
    p += 1;
    p += pad_u32(ppid, 5, &mut row[p..]);
    row[p] = b' ';
    p += 1;
    p += pad_bytes(state_bytes, 9, &mut row[p..]);
    p += pad_bytes(name_bytes, name_bytes.len().min(16), &mut row[p..]);
    row[p] = b'\n';
    p += 1;
    let _ = libc_lite::write(1, &row[..p]);
}

/// Write `n` as ASCII decimal into `dst`. Returns bytes written.
fn write_u32_into(mut n: u32, dst: &mut [u8]) -> usize {
    if n == 0 {
        dst[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in 0..i {
        dst[j] = tmp[i - 1 - j];
    }
    i
}

/// Right-align `n` into `width` columns, padding with spaces on the left.
fn pad_u32(n: u32, width: usize, dst: &mut [u8]) -> usize {
    let mut tmp = [b' '; 16];
    let len = write_u32_into(n, &mut tmp[..]);
    let pad = width.saturating_sub(len);
    for i in 0..pad {
        dst[i] = b' ';
    }
    dst[pad..pad + len].copy_from_slice(&tmp[..len]);
    pad + len
}

/// Left-align `src` into `width` columns, padding with spaces on the right.
fn pad_bytes(src: &[u8], width: usize, dst: &mut [u8]) -> usize {
    let len = src.len().min(width);
    dst[..len].copy_from_slice(&src[..len]);
    let pad = width.saturating_sub(len);
    for i in 0..pad {
        dst[len + i] = b' ';
    }
    len + pad
}

// Touch alloc to keep the `extern crate alloc` import meaningful and
// silence any unused-import lint; we don't actually allocate.
#[allow(dead_code)]
fn _alloc_touch() -> Vec<u8> {
    Vec::new()
}
