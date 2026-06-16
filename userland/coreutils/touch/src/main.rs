// touch — create files if missing (v0.2 §2.1).
//
// MVP behaviour: for each PATH operand, open it with O_CREAT and
// immediately close. If the file already exists, this is a no-op
// (O_CREAT without O_EXCL doesn't error). Mode for new files is
// 0o644.
//
// Out of scope for MVP: -a, -m, -t, -d. POSIX touch updates atime
// or mtime to the current time even if the file exists; RacOS
// doesn't have a `utime`/`utimensat` syscall yet, so leaving that
// out is the honest answer (rather than silently no-op'ing -a/-m
// flags). Once the syscall lands a follow-up can fill those in.

#![no_std]
#![no_main]

const O_RDWR: u32 = 0x0002;
const O_CREAT: u32 = 0x0040;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let _ = libc_lite::write(2, b"touch: missing file operand\n");
        return 1;
    }

    let mut had_error = false;
    let mut i = 1usize;
    while let Some(arg) = libc_lite::arg(argv, i) {
        i += 1;
        // Build a NUL-terminated copy on the stack — libc-lite::open
        // takes a &[u8] but the kernel's validate_user_string walks
        // until NUL, so we need to make sure one is present.
        let mut path = [0u8; 256];
        if arg.len() + 1 > path.len() {
            let _ = libc_lite::write(2, b"touch: path too long\n");
            had_error = true;
            continue;
        }
        path[..arg.len()].copy_from_slice(arg);
        // path[arg.len()] is already 0.

        match libc_lite::open(&path[..arg.len() + 1], O_RDWR | O_CREAT, 0o644) {
            Ok(fd) => {
                let _ = libc_lite::close(fd);
            }
            Err(_) => {
                let _ = libc_lite::write(2, b"touch: cannot touch '");
                let _ = libc_lite::write(2, arg);
                let _ = libc_lite::write(2, b"'\n");
                had_error = true;
            }
        }
    }

    if had_error {
        1
    } else {
        0
    }
}
