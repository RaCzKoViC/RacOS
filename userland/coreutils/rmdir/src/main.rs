// rmdir — remove empty directories (ROADMAP v0.2 §2.1).
//
// Emptiness is enforced by the filesystem, not here: racfs::unlink refuses a
// directory with entries. Re-checking in userland would be a race and a lie,
// so this reports whatever the kernel says.
//
// Usage: rmdir DIR...
// Exit:  0 if every operand was removed, 1 otherwise.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;

/// EINVAL is what racfs maps ENOTEMPTY to today, and ENOTDIR/ENOENT come
/// through as themselves. Translating the common ones keeps the message
/// actionable instead of printing a bare number.
fn describe(errno: i64) -> &'static str {
    match -errno {
        2 => "No such file or directory",
        13 => "Permission denied",
        20 => "Not a directory",
        22 => "Directory not empty",
        39 => "Directory not empty",
        _ => "cannot remove",
    }
}

fn fail(path: &[u8], errno: i64) {
    let _ = libc_lite::write(2, b"rmdir: ");
    let _ = libc_lite::write(2, path);
    let _ = libc_lite::write(2, b": ");
    let _ = libc_lite::write(2, describe(errno).as_bytes());
    let _ = libc_lite::write(2, b"\n");
}

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let n = libc_lite::arg_count(argc);
    if n < 2 {
        let _ = libc_lite::write(2, b"usage: rmdir DIR...\n");
        return 2;
    }

    let mut status = 0;
    let mut i = 1;
    while let Some(arg) = libc_lite::arg(argv, i) {
        i += 1;

        // Refuse a plain file before asking the kernel, so the message names
        // the real problem rather than whatever unlink() happens to return.
        let mut st = [0u8; 80];
        let mut cpath = String::new();
        match core::str::from_utf8(arg) {
            Ok(s) => cpath.push_str(s),
            Err(_) => {
                fail(arg, -2);
                status = 1;
                continue;
            }
        }
        cpath.push('\0');

        match libc_lite::stat(cpath.as_bytes(), &mut st) {
            Ok(()) => {
                // st_mode is a u32 at offset 16; S_IFDIR is 0o040000.
                let mode = u32::from_le_bytes([st[16], st[17], st[18], st[19]]);
                if mode & 0o170000 != 0o040000 {
                    fail(arg, -20);
                    status = 1;
                    continue;
                }
            }
            Err(e) => {
                fail(arg, e);
                status = 1;
                continue;
            }
        }

        if let Err(e) = libc_lite::unlink(cpath.as_bytes()) {
            fail(arg, e);
            status = 1;
        }
    }

    status
}
