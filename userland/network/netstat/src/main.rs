// netstat — list network connections (ROADMAP v0.2 §2.3).
//
// Usage: netstat [-t] [-u]
//   -t  TCP only
//   -u  UDP only
//   (neither: both, which is the default)
//
// Reads /proc/net/tcp and /proc/net/udp. The kernel already formats those as
// aligned columns, so this concatenates rather than re-parses: a second
// parser would be one more thing to keep in step with the kernel's output for
// no gain. What it does add is the protocol filter, a count, and a clear
// message when the files are missing — a netstat that silently prints nothing
// on a kernel without /proc/net would be worse than one that says so.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;
use alloc::vec::Vec;

fn read_file(path: &[u8]) -> Option<String> {
    let fd = libc_lite::open(path, 0, 0).ok()?;
    let mut out: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        match libc_lite::read(fd, &mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > 65536 {
                    break;
                }
            }
        }
    }
    let _ = libc_lite::close(fd);
    core::str::from_utf8(&out).ok().map(String::from)
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

/// Emit the data rows of one table, skipping its `#` header. Returns how many
/// rows were printed so the caller can summarise.
fn emit_rows(text: &str) -> u64 {
    let mut rows = 0;
    for line in text.split('\n') {
        let trimmed = line.trim_end();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let _ = libc_lite::write(1, trimmed.as_bytes());
        let _ = libc_lite::write(1, b"\n");
        rows += 1;
    }
    rows
}

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut want_tcp = false;
    let mut want_udp = false;

    let n = libc_lite::arg_count(argc);
    let mut i = 1;
    while i < n {
        match libc_lite::arg(argv, i) {
            Some(b"-t") => want_tcp = true,
            Some(b"-u") => want_udp = true,
            Some(b"-tu") | Some(b"-ut") => {
                want_tcp = true;
                want_udp = true;
            }
            Some(other) => {
                let _ = libc_lite::write(2, b"netstat: unknown option: ");
                let _ = libc_lite::write(2, other);
                let _ = libc_lite::write(2, b"\nusage: netstat [-t] [-u]\n");
                return 2;
            }
            None => break,
        }
        i += 1;
    }
    // No filter given means both.
    if !want_tcp && !want_udp {
        want_tcp = true;
        want_udp = true;
    }

    // One header for the combined listing, taken from the kernel's own so the
    // columns line up with the rows below it.
    let _ = libc_lite::write(
        1,
        b"Proto Local Address           Foreign Address         State        Recv-Q\n",
    );

    let mut total = 0u64;
    let mut missing = 0;

    if want_tcp {
        match read_file(b"/proc/net/tcp\0") {
            Some(text) => total += emit_rows(&text),
            None => {
                let _ = libc_lite::write(2, b"netstat: cannot read /proc/net/tcp\n");
                missing += 1;
            }
        }
    }
    if want_udp {
        match read_file(b"/proc/net/udp\0") {
            Some(text) => total += emit_rows(&text),
            None => {
                let _ = libc_lite::write(2, b"netstat: cannot read /proc/net/udp\n");
                missing += 1;
            }
        }
    }

    if missing > 0 {
        return 1;
    }

    let mut summary = String::new();
    push_num(&mut summary, total);
    summary.push_str(if total == 1 {
        " connection\n"
    } else {
        " connections\n"
    });
    let _ = libc_lite::write(1, summary.as_bytes());
    0
}
