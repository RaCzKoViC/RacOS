// rpkg — low-level package CLI (RacOS userland binary)
//
// MVP scope (T3.2):
//   rpkg install <file.rpk>   parse, validate, write manifest + files
//                             index + data into /var/lib/rpkg/info/<name>/
//   rpkg list                 print one installed package name per line
//   rpkg remove <name>        read files index, unlink each, drop info dir
//
// No signature verification, no dependency resolution, no /bin/ deployment
// (initramfs is read-only on a fresh boot; the data payload lands inside
// the rpkg info dir, which lives on writable racfs/ram0).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use rpkg::{
    manifest_summary, parse_files_list, parse_header, section, serialize_files_list, SectionKind,
};

const O_RDONLY: u32 = 0x0000;
const O_WRONLY: u32 = 0x0001;
const O_CREAT: u32 = 0x0040;
const O_TRUNC: u32 = 0x0200;

const INFO_ROOT: &[u8] = b"/var/lib/rpkg/info";

const MAX_RPK_BYTES: usize = 64 * 1024;

#[no_mangle]
pub extern "C" fn main(_argc: i32, argv: *const *const u8) -> i32 {
    let cmd = match libc_lite::arg(argv, 1) {
        Some(c) => c,
        None => return usage(),
    };

    match cmd {
        b"install" => match libc_lite::arg(argv, 2) {
            Some(path) => cmd_install(path),
            None => {
                ewrite(b"rpkg: install needs a file path\n");
                2
            }
        },
        b"list" => cmd_list(),
        b"remove" => match libc_lite::arg(argv, 2) {
            Some(name) => cmd_remove(name),
            None => {
                ewrite(b"rpkg: remove needs a package name\n");
                2
            }
        },
        _ => usage(),
    }
}

fn usage() -> i32 {
    ewrite(
        b"usage: rpkg install <file.rpk>\n       \
              rpkg list\n       \
              rpkg remove <name>\n",
    );
    2
}

// ─────────────────────────────────────────────────────────────────
// install
// ─────────────────────────────────────────────────────────────────

fn cmd_install(rpk_path: &[u8]) -> i32 {
    let bytes = match read_file(rpk_path) {
        Ok(b) => b,
        Err(_) => {
            ewrite(b"rpkg: cannot read .rpk\n");
            return 1;
        }
    };

    let header = match parse_header(&bytes) {
        Ok(h) => h,
        Err(_) => {
            ewrite(b"rpkg: invalid .rpk header\n");
            return 1;
        }
    };
    let manifest = match section(&bytes, &header, SectionKind::Manifest) {
        Ok(s) => s,
        Err(_) => {
            ewrite(b"rpkg: missing manifest section\n");
            return 1;
        }
    };
    let data = match section(&bytes, &header, SectionKind::Data) {
        Ok(s) => s,
        Err(_) => {
            ewrite(b"rpkg: missing data section\n");
            return 1;
        }
    };
    let summary = match manifest_summary(manifest) {
        Ok(s) => s,
        Err(_) => {
            ewrite(b"rpkg: malformed manifest\n");
            return 1;
        }
    };
    let name = match summary.name.as_deref() {
        Some(n) if !n.is_empty() => n,
        _ => {
            ewrite(b"rpkg: manifest is missing [package] name\n");
            return 1;
        }
    };

    // Best-effort mkdir on each parent — EEXIST is fine, we only care that
    // the leaf is reachable for the writes below.
    let _ = libc_lite::mkdir(b"/var\0", 0o755);
    let _ = libc_lite::mkdir(b"/var/lib\0", 0o755);
    let _ = libc_lite::mkdir(b"/var/lib/rpkg\0", 0o755);
    let _ = libc_lite::mkdir(b"/var/lib/rpkg/info\0", 0o755);

    let info_dir = join_path(INFO_ROOT, name.as_bytes());
    let _ = libc_lite::mkdir(&with_nul(&info_dir), 0o755);

    let manifest_path = join_path(&info_dir, b"manifest.toml");
    if write_file(&with_nul(&manifest_path), manifest).is_err() {
        ewrite(b"rpkg: write manifest.toml failed\n");
        return 1;
    }

    let data_path = join_path(&info_dir, b"data");
    if write_file(&with_nul(&data_path), data).is_err() {
        ewrite(b"rpkg: write data failed\n");
        return 1;
    }

    let mut files = Vec::new();
    files.push(bytes_to_string(&manifest_path));
    files.push(bytes_to_string(&data_path));
    let files_index = serialize_files_list(&files);
    let files_path = join_path(&info_dir, b"files");
    if write_file(&with_nul(&files_path), files_index.as_bytes()).is_err() {
        ewrite(b"rpkg: write files index failed\n");
        return 1;
    }

    let _ = libc_lite::write(1, b"installed ");
    let _ = libc_lite::write(1, name.as_bytes());
    let _ = libc_lite::write(1, b"\n");
    0
}

// ─────────────────────────────────────────────────────────────────
// list
// ─────────────────────────────────────────────────────────────────

fn cmd_list() -> i32 {
    let dir = with_nul(INFO_ROOT);
    let fd = match libc_lite::open(&dir, O_RDONLY, 0) {
        Ok(fd) => fd,
        // No info dir at all → no packages installed; success with no output.
        Err(_) => return 0,
    };
    let mut buf = [0u8; 4096];
    let n = libc_lite::getdents(fd, &mut buf).unwrap_or(0);
    let _ = libc_lite::close(fd);

    // Dirent layout (same parser as ps): ino8 + type1 + namelen1 + name.
    let mut off = 0usize;
    while off + 10 <= n {
        let name_len = buf[off + 9] as usize;
        let entry_size = 10 + name_len;
        if off + entry_size > n {
            break;
        }
        let name = &buf[off + 10..off + 10 + name_len];
        if name != b"." && name != b".." && !name.is_empty() {
            let _ = libc_lite::write(1, name);
            let _ = libc_lite::write(1, b"\n");
        }
        off += entry_size;
    }
    0
}

// ─────────────────────────────────────────────────────────────────
// remove
// ─────────────────────────────────────────────────────────────────

fn cmd_remove(name: &[u8]) -> i32 {
    let info_dir = join_path(INFO_ROOT, name);
    let files_path = join_path(&info_dir, b"files");

    let listed = match read_file(&with_nul(&files_path)) {
        Ok(b) => b,
        Err(_) => {
            ewrite(b"rpkg: not installed\n");
            return 1;
        }
    };
    let listed_str = core::str::from_utf8(&listed).unwrap_or("");
    let paths = parse_files_list(listed_str);

    // Unlink everything the package owns. Errors are tolerated — a manual
    // edit may have already removed some files.
    for p in &paths {
        let _ = libc_lite::unlink(&with_nul(p.as_bytes()));
    }
    let manifest_path = join_path(&info_dir, b"manifest.toml");
    let _ = libc_lite::unlink(&with_nul(&manifest_path));
    let _ = libc_lite::unlink(&with_nul(&files_path));
    let data_path = join_path(&info_dir, b"data");
    let _ = libc_lite::unlink(&with_nul(&data_path));
    let _ = libc_lite::unlink(&with_nul(&info_dir));

    let _ = libc_lite::write(1, b"removed ");
    let _ = libc_lite::write(1, name);
    let _ = libc_lite::write(1, b"\n");
    0
}

// ─────────────────────────────────────────────────────────────────
// helpers
// ─────────────────────────────────────────────────────────────────

fn ewrite(msg: &[u8]) {
    let _ = libc_lite::write(2, msg);
}

fn read_file(path: &[u8]) -> Result<Vec<u8>, i64> {
    let nul = with_nul(path);
    let fd = libc_lite::open(&nul, O_RDONLY, 0)?;
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if out.len() + n > MAX_RPK_BYTES {
                    let _ = libc_lite::close(fd);
                    return Err(-1);
                }
                out.extend_from_slice(&buf[..n]);
            }
            Err(e) => {
                let _ = libc_lite::close(fd);
                return Err(e);
            }
        }
    }
    let _ = libc_lite::close(fd);
    Ok(out)
}

fn write_file(path_nul: &[u8], content: &[u8]) -> Result<(), i64> {
    let fd = libc_lite::open(path_nul, O_WRONLY | O_CREAT | O_TRUNC, 0o644)?;
    let mut written = 0;
    while written < content.len() {
        match libc_lite::write(fd, &content[written..]) {
            Ok(0) => {
                let _ = libc_lite::close(fd);
                return Err(-1);
            }
            Ok(n) => written += n,
            Err(e) => {
                let _ = libc_lite::close(fd);
                return Err(e);
            }
        }
    }
    let _ = libc_lite::close(fd);
    Ok(())
}

/// Join two path fragments with a single '/' separator. Does not append NUL.
fn join_path(base: &[u8], tail: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(base.len() + 1 + tail.len());
    let base = if base.ends_with(b"/") {
        &base[..base.len() - 1]
    } else {
        base
    };
    out.extend_from_slice(base);
    out.push(b'/');
    let tail = tail.strip_prefix(b"/").unwrap_or(tail);
    out.extend_from_slice(tail);
    out
}

/// NUL-terminate a path slice for the syscall ABI.
fn with_nul(path: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(path.len() + 1);
    out.extend_from_slice(path);
    if !out.ends_with(&[0]) {
        out.push(0);
    }
    out
}

fn bytes_to_string(b: &[u8]) -> String {
    match core::str::from_utf8(b) {
        Ok(s) => String::from(s),
        Err(_) => String::new(),
    }
}
