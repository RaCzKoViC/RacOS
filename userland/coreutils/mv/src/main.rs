#![no_std]
#![no_main]

use libc_lite;

/// mv — move or rename file
/// Usage: mv source dest
///
/// Implemented as copy-then-unlink (the kernel's rename is not an atomic
/// rename yet). Two rules make that safe:
///
/// - source and destination that are the same file - the same path, or two
///   hard links to one inode - are left alone with exit 0, as POSIX rename
///   does. Copying a file onto itself with O_TRUNC empties it before a byte
///   is read; before this check `mv f f` deleted f.
/// - the source is unlinked only after every byte was written (write_all,
///   not `let _ = write`) and the destination's size matches the source's.
///   A write error leaves the source untouched, and removes the partial
///   destination only if mv created it.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        let _ = libc_lite::write_all(2, b"mv: missing operand\n");
        return 1;
    }

    let src_ptr = unsafe { *argv.add(1) };
    let dst_ptr = unsafe { *argv.add(2) };

    if src_ptr.is_null() || dst_ptr.is_null() {
        let _ = libc_lite::write_all(2, b"mv: null path\n");
        return 1;
    }

    // Get lengths
    let mut src_len = 0usize;
    unsafe {
        while *src_ptr.add(src_len) != 0 {
            src_len += 1;
        }
    }
    let mut dst_len = 0usize;
    unsafe {
        while *dst_ptr.add(dst_len) != 0 {
            dst_len += 1;
        }
    }

    let src_path = unsafe { core::slice::from_raw_parts(src_ptr, src_len + 1) };
    let dst_path = unsafe { core::slice::from_raw_parts(dst_ptr, dst_len + 1) };

    let src_st = match stat_of(src_path) {
        Some(st) => st,
        None => {
            let _ = libc_lite::write_all(2, b"mv: ");
            let _ = libc_lite::write_all(2, &src_path[..src_len]);
            let _ = libc_lite::write_all(2, b": No such file or directory\n");
            return 1;
        }
    };

    // Same file (same mount, same inode): nothing to do, and touching it
    // would be the bug this guards against.
    let dst_existed = match stat_of(dst_path) {
        Some(dst_st) => {
            if dst_st.dev == src_st.dev && dst_st.ino == src_st.ino {
                return 0;
            }
            true
        }
        None => false,
    };

    let src_fd = match libc_lite::open(src_path, 0, 0) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc_lite::write_all(2, b"mv: ");
            let _ = libc_lite::write_all(2, &src_path[..src_len]);
            let _ = libc_lite::write_all(2, b": cannot open\n");
            return 1;
        }
    };

    // O_WRONLY|O_CREAT|O_TRUNC = 0x0241.
    let dst_fd = match libc_lite::open(dst_path, 0x0241, 0o644) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc_lite::write_all(2, b"mv: ");
            let _ = libc_lite::write_all(2, &dst_path[..dst_len]);
            let _ = libc_lite::write_all(2, b": cannot create file\n");
            let _ = libc_lite::close(src_fd);
            return 1;
        }
    };

    let mut buf = [0u8; 4096];
    let mut copied: u64 = 0;
    let mut failed: Option<&[u8]> = None;
    loop {
        match libc_lite::read(src_fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => match libc_lite::write_all(dst_fd, &buf[..n]) {
                Ok(()) => copied += n as u64,
                Err(_) => {
                    failed = Some(b"mv: write error\n");
                    break;
                }
            },
            Err(_) => {
                failed = Some(b"mv: read error\n");
                break;
            }
        }
    }

    // What the destination holds now, as the filesystem reports it, before
    // the source is given up. A device node reports no size; the byte count
    // written is the only account there.
    let dst_size = fstat_size(dst_fd);
    let _ = libc_lite::close(src_fd);
    let _ = libc_lite::close(dst_fd);

    if failed.is_none() && copied != src_st.size {
        failed = Some(b"mv: destination is short of the source\n");
    }
    if failed.is_none() && src_st.is_regular() {
        if let Some(sz) = dst_size {
            if sz != src_st.size {
                failed = Some(b"mv: destination size does not match the source\n");
            }
        }
    }

    if let Some(msg) = failed {
        let _ = libc_lite::write_all(2, msg);
        let _ = libc_lite::write_all(2, b"mv: ");
        let _ = libc_lite::write_all(2, &src_path[..src_len]);
        let _ = libc_lite::write_all(2, b" left in place\n");
        if !dst_existed {
            let _ = libc_lite::unlink(dst_path);
        }
        return 1;
    }

    match libc_lite::unlink(src_path) {
        Ok(()) => 0,
        Err(_) => {
            let _ = libc_lite::write_all(2, b"mv: cannot unlink source\n");
            1
        }
    }
}

struct Ident {
    dev: u64,
    ino: u64,
    size: u64,
    mode: u32,
}

impl Ident {
    fn is_regular(&self) -> bool {
        (self.mode & 0o170000) == 0o100000
    }
}

/// (st_dev, st_ino, st_size, st_mode) of `path`, or None if it does not
/// exist. Field offsets follow StatBuf in KERNEL_ABI.md section 6.1.
fn stat_of(path: &[u8]) -> Option<Ident> {
    let mut raw = [0u8; 80];
    libc_lite::stat(path, &mut raw).ok()?;
    Some(Ident {
        dev: u64::from_le_bytes([
            raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
        ]),
        ino: u64::from_le_bytes([
            raw[8], raw[9], raw[10], raw[11], raw[12], raw[13], raw[14], raw[15],
        ]),
        mode: u32::from_le_bytes([raw[16], raw[17], raw[18], raw[19]]),
        size: u64::from_le_bytes([
            raw[32], raw[33], raw[34], raw[35], raw[36], raw[37], raw[38], raw[39],
        ]),
    })
}

/// st_size of an open regular file; None for anything fstat cannot size
/// (a device node) or on error.
fn fstat_size(fd: i32) -> Option<u64> {
    let mut raw = [0u8; 80];
    libc_lite::fstat(fd, &mut raw).ok()?;
    let mode = u32::from_le_bytes([raw[16], raw[17], raw[18], raw[19]]);
    if (mode & 0o170000) != 0o100000 {
        return None;
    }
    Some(u64::from_le_bytes([
        raw[32], raw[33], raw[34], raw[35], raw[36], raw[37], raw[38], raw[39],
    ]))
}
