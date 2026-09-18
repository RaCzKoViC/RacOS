#![no_std]
#![no_main]

use libc_lite;

/// tee — read from stdin and write to stdout + file(s)
/// Usage: tee [file...]
///
/// Every output goes through write_all: stdout is usually a pipe, a pipe
/// holds 4 KiB, and `write` takes what fits - `cat big | tee copy | wc -c`
/// used to deliver 4096 bytes to both sides and exit 0. An output that
/// fails (a full disk, `/dev/full`) is reported once, dropped, and makes
/// the exit status 1; the remaining outputs keep receiving data, as GNU
/// tee does.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut fds = [0i32; 8];
    let mut fd_count = 0;
    let mut status = 0;

    // Open all output files
    for i in 1..argc {
        if fd_count >= 8 {
            break;
        }
        let file_ptr = unsafe { *argv.add(i as usize) };
        if file_ptr.is_null() {
            continue;
        }
        let mut len = 0;
        unsafe {
            while *file_ptr.add(len) != 0 {
                len += 1;
            }
        }
        let path = unsafe { core::slice::from_raw_parts(file_ptr, len + 1) };
        // O_WRONLY (0x0001) | O_CREAT (0x0040) | O_TRUNC (0x0200) = 0x0241.
        match libc_lite::open(path, 0x0241, 0o644) {
            Ok(fd) => {
                fds[fd_count] = fd;
                fd_count += 1;
            }
            Err(_) => {
                let _ = libc_lite::write_all(2, b"tee: ");
                let _ = libc_lite::write_all(2, &path[..len]);
                let _ = libc_lite::write_all(2, b": cannot open for writing\n");
                status = 1;
            }
        }
    }

    // -1 marks an output that failed and is no longer written to.
    let mut stdout_ok = true;
    let mut buf = [0u8; 4096];
    loop {
        match libc_lite::read(0, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if stdout_ok && libc_lite::write_all(1, &buf[..n]).is_err() {
                    let _ = libc_lite::write_all(2, b"tee: stdout: write error\n");
                    stdout_ok = false;
                    status = 1;
                }
                for i in 0..fd_count {
                    if fds[i] < 0 {
                        continue;
                    }
                    if libc_lite::write_all(fds[i], &buf[..n]).is_err() {
                        let _ = libc_lite::write_all(2, b"tee: write error on an output file\n");
                        let _ = libc_lite::close(fds[i]);
                        fds[i] = -1;
                        status = 1;
                    }
                }
            }
            Err(_) => break,
        }
    }

    // Close files
    for i in 0..fd_count {
        if fds[i] >= 0 {
            let _ = libc_lite::close(fds[i]);
        }
    }

    status
}
