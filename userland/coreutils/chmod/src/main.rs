// chmod — change file mode (v0.2 §2.1).
//
// Usage:
//   chmod MODE PATH...
//
// MODE accepted: octal digits (`644`, `0644`, `0o644`). Symbolic mode
// (`u+x`, `g-w`, `a=rwx`, …) is post-MVP.
//
// The kernel exposes `sys_chmod(path, mode)` (mode masked to 0o7777);
// libc-lite has the wrapper. The userland binary is therefore just
// argument parsing + a loop.

#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        let _ = libc_lite::write(2, b"chmod: usage: chmod MODE PATH...\n");
        return 1;
    }

    let mode_arg = match libc_lite::arg(argv, 1) {
        Some(s) => s,
        None => {
            let _ = libc_lite::write(2, b"chmod: missing MODE\n");
            return 1;
        }
    };

    let mode = match parse_octal_mode(mode_arg) {
        Some(m) => m,
        None => {
            let _ = libc_lite::write(2, b"chmod: invalid mode\n");
            return 1;
        }
    };

    let mut had_error = false;
    let mut i = 2usize;
    while let Some(arg) = libc_lite::arg(argv, i) {
        i += 1;
        let mut path = [0u8; 256];
        if arg.len() + 1 > path.len() {
            let _ = libc_lite::write(2, b"chmod: path too long\n");
            had_error = true;
            continue;
        }
        path[..arg.len()].copy_from_slice(arg);

        if libc_lite::chmod(&path[..arg.len() + 1], mode).is_err() {
            let _ = libc_lite::write(2, b"chmod: cannot change mode of '");
            let _ = libc_lite::write(2, arg);
            let _ = libc_lite::write(2, b"'\n");
            had_error = true;
        }
    }

    if had_error {
        1
    } else {
        0
    }
}

/// Parse a string of octal digits into a mode value. Accepts an
/// optional `0`/`0o` prefix. Returns `None` on any non-octal byte or
/// if the result exceeds the 12-bit mode mask.
fn parse_octal_mode(s: &[u8]) -> Option<u32> {
    let mut bytes = s;
    if bytes.starts_with(b"0o") || bytes.starts_with(b"0O") {
        bytes = &bytes[2..];
    }
    if bytes.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in bytes {
        if !(b'0'..=b'7').contains(&b) {
            return None;
        }
        n = n.checked_mul(8)?.checked_add((b - b'0') as u32)?;
        if n > 0o7777 {
            return None;
        }
    }
    Some(n)
}
