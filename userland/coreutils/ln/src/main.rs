// ln — create a hard link (ROADMAP v0.2 §2.1).
//
// Usage: ln TARGET LINK_NAME
//        ln TARGET... DIRECTORY
//
// Hard links only. `-s` is accepted just so the error explains itself:
// sys_symlink is still a stub, and silently making a hard link when the user
// asked for a symbolic one would be worse than refusing.
//
// A link is a second directory entry pointing at one inode, so both paths must
// live on the same filesystem — the kernel returns EXDEV otherwise. Directories
// cannot be linked: with no `..` fixups and no cycle detection in the VFS, a
// directory link turns the tree into a graph that find and du would walk
// forever.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;
use alloc::vec::Vec;

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;

fn cstr(s: &str) -> String {
    let mut c = String::from(s);
    c.push('\0');
    c
}

fn describe(errno: i64) -> &'static str {
    match -errno {
        1 => "Operation not permitted (hard links to directories are not allowed)",
        2 => "No such file or directory",
        13 => "Permission denied",
        17 => "File exists",
        18 => "Invalid cross-device link (both paths must be on one filesystem)",
        38 => "Not supported by this filesystem",
        _ => "cannot create link",
    }
}

fn fail(target: &str, name: &str, errno: i64) {
    let _ = libc_lite::write(2, b"ln: ");
    let _ = libc_lite::write(2, name.as_bytes());
    let _ = libc_lite::write(2, b" -> ");
    let _ = libc_lite::write(2, target.as_bytes());
    let _ = libc_lite::write(2, b": ");
    let _ = libc_lite::write(2, describe(errno).as_bytes());
    let _ = libc_lite::write(2, b"\n");
}

fn is_dir(path: &str) -> bool {
    let mut st = [0u8; 80];
    if libc_lite::stat(cstr(path).as_bytes(), &mut st).is_err() {
        return false;
    }
    // StatBuf is repr(C): st_mode is a u32 at offset 16.
    let mode = u32::from_le_bytes([st[16], st[17], st[18], st[19]]);
    mode & S_IFMT == S_IFDIR
}

fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

fn join(dir: &str, name: &str) -> String {
    let mut p = String::from(dir.trim_end_matches('/'));
    p.push('/');
    p.push_str(name);
    p
}

fn link_one(target: &str, name: &str) -> bool {
    match libc_lite::link(cstr(target).as_bytes(), cstr(name).as_bytes()) {
        Ok(()) => true,
        Err(e) => {
            fail(target, name, e);
            false
        }
    }
}

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let n = libc_lite::arg_count(argc);
    let mut operands: Vec<String> = Vec::new();

    let mut i = 1;
    while i < n {
        match libc_lite::arg(argv, i) {
            Some(b"-s") => {
                let _ = libc_lite::write(
                    2,
                    b"ln: -s: symbolic links are not supported yet (sys_symlink is unimplemented)\n",
                );
                return 1;
            }
            Some(arg) if arg.starts_with(b"-") && arg.len() > 1 => {
                let _ = libc_lite::write(2, b"ln: unknown option: ");
                let _ = libc_lite::write(2, arg);
                let _ = libc_lite::write(2, b"\nusage: ln TARGET LINK_NAME | ln TARGET... DIR\n");
                return 2;
            }
            Some(arg) => {
                if let Ok(s) = core::str::from_utf8(arg) {
                    operands.push(String::from(s));
                }
            }
            None => break,
        }
        i += 1;
    }

    if operands.len() < 2 {
        let _ = libc_lite::write(2, b"usage: ln TARGET LINK_NAME | ln TARGET... DIR\n");
        return 2;
    }

    // Last operand is the destination. If it is a directory, every target is
    // linked into it under its own basename.
    let dest = operands[operands.len() - 1].clone();
    let targets = &operands[..operands.len() - 1];

    let mut ok = true;
    if is_dir(&dest) {
        for t in targets {
            let name = join(&dest, basename(t));
            ok &= link_one(t, &name);
        }
    } else {
        if targets.len() > 1 {
            let _ = libc_lite::write(2, b"ln: target is not a directory: ");
            let _ = libc_lite::write(2, dest.as_bytes());
            let _ = libc_lite::write(2, b"\n");
            return 1;
        }
        ok = link_one(&targets[0], &dest);
    }

    if ok {
        0
    } else {
        1
    }
}
