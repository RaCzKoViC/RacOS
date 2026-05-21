#![no_std]
#![no_main]

use libc_lite::{open, read, close, write, println};

/// mount — list active mountpoints (reads /proc/mounts).
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let fd = match open(b"/proc/mounts\0", 0, 0) {
        Ok(fd) => fd,
        Err(_) => {
            println("mount: cannot open /proc/mounts");
            return 1;
        }
    };
    let mut buf = [0u8; 1024];
    loop {
        match read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => { let _ = write(1, &buf[..n]); }
            Err(_) => break,
        }
    }
    let _ = close(fd);
    0
}
