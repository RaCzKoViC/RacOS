// du — disk usage, summed recursively (ROADMAP v0.2 §2.1).
//
// Usage: du [-s] [-b] [PATH...]
//   -s  summarise: one total per operand instead of a line per directory
//   -b  report bytes instead of 1 KiB blocks
//
// Sizes come from stat's st_size, so this measures apparent size rather than
// blocks actually allocated. racfs has no sparse files today, which makes the
// two identical; if that changes, this comment is the thing to revisit.
//
// Recursion is depth-bounded rather than unbounded: a directory cycle (which
// hard links could introduce once sys_link exists) would otherwise spin until
// the stack gives out, and a shell tool has no business taking the process
// down.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;
use alloc::vec::Vec;

const MAX_DEPTH: u32 = 32;
const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;

struct Opts {
    summarise: bool,
    bytes: bool,
}

fn cstr(path: &str) -> String {
    let mut s = String::from(path);
    s.push('\0');
    s
}

/// (is_directory, size_in_bytes), or None when the path cannot be stat'ed.
fn stat_path(path: &str) -> Option<(bool, u64)> {
    let mut st = [0u8; 80];
    libc_lite::stat(cstr(path).as_bytes(), &mut st).ok()?;
    // StatBuf is repr(C): st_mode is a u32 at 16, st_size a u64 at 32.
    let mode = u32::from_le_bytes([st[16], st[17], st[18], st[19]]);
    let size = u64::from_le_bytes([
        st[32], st[33], st[34], st[35], st[36], st[37], st[38], st[39],
    ]);
    Some((mode & S_IFMT == S_IFDIR, size))
}

fn read_dir(path: &str) -> Vec<String> {
    let fd = match libc_lite::open(cstr(path).as_bytes(), 0, 0) {
        Ok(fd) => fd,
        Err(_) => return Vec::new(),
    };
    let mut buf = [0u8; 4096];
    // sys_getdents returns every entry in one call and keeps no cursor;
    // looping would re-emit them forever.
    let n = match libc_lite::getdents(fd, &mut buf) {
        Ok(n) => n,
        Err(_) => {
            let _ = libc_lite::close(fd);
            return Vec::new();
        }
    };
    let _ = libc_lite::close(fd);

    let mut names = Vec::new();
    let mut off = 0usize;
    // Layout: [0..8) ino u64, [8] file_type u8, [9] name_len u8, [10..] name.
    while off + 10 <= n {
        let name_len = buf[off + 9] as usize;
        let entry = 10 + name_len;
        if off + entry > n {
            break;
        }
        let raw = &buf[off + 10..off + 10 + name_len];
        off += entry;
        if raw == b"." || raw == b".." {
            continue;
        }
        if let Ok(s) = core::str::from_utf8(raw) {
            names.push(String::from(s));
        }
    }
    names
}

fn join(dir: &str, name: &str) -> String {
    let mut p = String::from(dir);
    if !p.ends_with('/') {
        p.push('/');
    }
    p.push_str(name);
    p
}

fn push_num(out: &mut String, mut v: u64) {
    if v == 0 {
        out.push('0');
        return;
    }
    let mut d = [0u8; 20];
    let mut i = 0;
    while v > 0 {
        d[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        out.push(d[i] as char);
    }
}

fn report(path: &str, bytes: u64, opts: &Opts) {
    let shown = if opts.bytes {
        bytes
    } else {
        // 1 KiB blocks, rounded up: a 1-byte file occupies a block.
        bytes.div_ceil(1024)
    };
    let mut line = String::new();
    push_num(&mut line, shown);
    line.push('\t');
    line.push_str(path);
    line.push('\n');
    let _ = libc_lite::write(1, line.as_bytes());
}

/// Total bytes under `path`, printing a line per directory unless -s.
fn walk(path: &str, depth: u32, opts: &Opts) -> u64 {
    let (is_dir, size) = match stat_path(path) {
        Some(v) => v,
        None => {
            let _ = libc_lite::write(2, b"du: cannot access ");
            let _ = libc_lite::write(2, path.as_bytes());
            let _ = libc_lite::write(2, b"\n");
            return 0;
        }
    };

    if !is_dir {
        return size;
    }
    if depth >= MAX_DEPTH {
        let _ = libc_lite::write(2, b"du: max depth reached at ");
        let _ = libc_lite::write(2, path.as_bytes());
        let _ = libc_lite::write(2, b"\n");
        return size;
    }

    let mut total = size;
    for name in read_dir(path) {
        total += walk(&join(path, &name), depth + 1, opts);
    }
    if !opts.summarise {
        report(path, total, opts);
    }
    total
}

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut opts = Opts {
        summarise: false,
        bytes: false,
    };
    let mut paths: Vec<String> = Vec::new();

    let n = libc_lite::arg_count(argc);
    let mut i = 1;
    while i < n {
        match libc_lite::arg(argv, i) {
            Some(b"-s") => opts.summarise = true,
            Some(b"-b") => opts.bytes = true,
            Some(b"-sb") | Some(b"-bs") => {
                opts.summarise = true;
                opts.bytes = true;
            }
            Some(other) if other.starts_with(b"-") && other.len() > 1 => {
                let _ = libc_lite::write(2, b"du: unknown option: ");
                let _ = libc_lite::write(2, other);
                let _ = libc_lite::write(2, b"\nusage: du [-s] [-b] [PATH...]\n");
                return 2;
            }
            Some(other) => {
                if let Ok(s) = core::str::from_utf8(other) {
                    paths.push(String::from(s));
                }
            }
            None => break,
        }
        i += 1;
    }

    if paths.is_empty() {
        paths.push(String::from("."));
    }

    for p in &paths {
        let total = walk(p, 0, &opts);
        if opts.summarise {
            report(p, total, &opts);
        }
    }
    0
}
