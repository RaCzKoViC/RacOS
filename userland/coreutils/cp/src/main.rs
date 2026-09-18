#![no_std]
#![no_main]

use libc_lite;

/// cp — copy file
/// Usage: cp source dest
///
/// Refuses to copy a file onto itself (same mount, same inode - the same
/// path or a hard link) before the destination is opened, because opening
/// it with O_TRUNC would empty the source first; `cp f f` used to leave f
/// empty with exit 0. Every write goes through write_all, so a short write
/// or a write error is a failure (exit 1), never a truncated copy that
/// looks complete.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        let _ = libc_lite::write_all(2, b"cp: missing operand\n");
        return 1;
    }

    let src_ptr = unsafe { *argv.add(1) };
    let dst_ptr = unsafe { *argv.add(2) };

    if src_ptr.is_null() || dst_ptr.is_null() {
        let _ = libc_lite::write_all(2, b"cp: null path\n");
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

    if let (Some(a), Some(b)) = (stat_ident(src_path), stat_ident(dst_path)) {
        if a == b {
            let _ = libc_lite::write_all(2, b"cp: ");
            let _ = libc_lite::write_all(2, &src_path[..src_len]);
            let _ = libc_lite::write_all(2, b" and ");
            let _ = libc_lite::write_all(2, &dst_path[..dst_len]);
            let _ = libc_lite::write_all(2, b" are the same file\n");
            return 1;
        }
    }

    // Open source file
    let src_fd = match libc_lite::open(src_path, 0, 0) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc_lite::write_all(2, b"cp: ");
            let _ = libc_lite::write_all(2, &src_path[..src_len]);
            let _ = libc_lite::write_all(2, b": No such file or directory\n");
            return 1;
        }
    };

    // Create destination file (write-only, truncate, create with 0o644)
    // O_WRONLY (0x0001) | O_CREAT (0x0040) | O_TRUNC (0x0200) = 0x0241.
    let dst_fd = match libc_lite::open(dst_path, 0x0241, 0o644) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc_lite::write_all(2, b"cp: ");
            let _ = libc_lite::write_all(2, &dst_path[..dst_len]);
            let _ = libc_lite::write_all(2, b": cannot create file\n");
            let _ = libc_lite::close(src_fd);
            return 1;
        }
    };

    let mut buf = [0u8; 4096];
    let mut status = 0;
    loop {
        match libc_lite::read(src_fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if libc_lite::write_all(dst_fd, &buf[..n]).is_err() {
                    let _ = libc_lite::write_all(2, b"cp: write error\n");
                    status = 1;
                    break;
                }
            }
            Err(_) => {
                let _ = libc_lite::write_all(2, b"cp: read error\n");
                status = 1;
                break;
            }
        }
    }
    let _ = libc_lite::close(src_fd);
    let _ = libc_lite::close(dst_fd);
    status
}

/// (st_dev, st_ino) of `path`, or None if it does not exist. Offsets follow
/// StatBuf in KERNEL_ABI.md section 6.1.
fn stat_ident(path: &[u8]) -> Option<(u64, u64)> {
    let mut raw = [0u8; 80];
    libc_lite::stat(path, &mut raw).ok()?;
    let dev = u64::from_le_bytes([
        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
    ]);
    let ino = u64::from_le_bytes([
        raw[8], raw[9], raw[10], raw[11], raw[12], raw[13], raw[14], raw[15],
    ]);
    Some((dev, ino))
}
