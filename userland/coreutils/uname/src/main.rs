// uname — print system information (v0.2 §2.1).
//
// Flags (compatible with GNU coreutils where it makes sense):
//   -s    kernel name (default if no flag)
//   -n    nodename (hostname)
//   -r    kernel release
//   -v    kernel version
//   -m    machine hardware name
//   -a    all of the above, space-separated, in -s -n -r -v -m order
//
// The kernel sys_uname syscall fills a 325-byte UTS struct
// (kernel/src/syscall/handlers.rs:sys_uname). libc-lite has the
// wrapper.
//
// Flags can be combined (`-srm`); they're processed in canonical
// order regardless of input order, matching GNU's behaviour.

#![no_std]
#![no_main]

use libc_lite::UtsName;

const F_S: u8 = 1 << 0;
const F_N: u8 = 1 << 1;
const F_R: u8 = 1 << 2;
const F_V: u8 = 1 << 3;
const F_M: u8 = 1 << 4;
const F_ALL: u8 = F_S | F_N | F_R | F_V | F_M;

#[no_mangle]
pub extern "C" fn main(_argc: i32, argv: *const *const u8) -> i32 {
    let mut flags: u8 = 0;
    let mut i = 1usize;
    while let Some(arg) = libc_lite::arg(argv, i) {
        i += 1;
        if arg.is_empty() || arg[0] != b'-' || arg.len() < 2 {
            let _ = libc_lite::write(2, b"uname: unknown operand\n");
            return 1;
        }
        for &b in &arg[1..] {
            match b {
                b's' => flags |= F_S,
                b'n' => flags |= F_N,
                b'r' => flags |= F_R,
                b'v' => flags |= F_V,
                b'm' => flags |= F_M,
                b'a' => flags |= F_ALL,
                _ => {
                    let _ = libc_lite::write(2, b"uname: unknown flag\n");
                    return 1;
                }
            }
        }
    }
    if flags == 0 {
        flags = F_S;
    }

    let mut uts = UtsName::zeroed();
    if libc_lite::uname(&mut uts).is_err() {
        let _ = libc_lite::write(2, b"uname: sys_uname failed\n");
        return 1;
    }

    let mut first = true;
    if flags & F_S != 0 {
        emit_field(&uts.sysname, &mut first);
    }
    if flags & F_N != 0 {
        emit_field(&uts.nodename, &mut first);
    }
    if flags & F_R != 0 {
        emit_field(&uts.release, &mut first);
    }
    if flags & F_V != 0 {
        emit_field(&uts.version, &mut first);
    }
    if flags & F_M != 0 {
        emit_field(&uts.machine, &mut first);
    }
    let _ = libc_lite::write(1, b"\n");
    0
}

fn emit_field(field: &[u8; 65], first: &mut bool) {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    if end == 0 {
        return;
    }
    if !*first {
        let _ = libc_lite::write(1, b" ");
    }
    *first = false;
    let _ = libc_lite::write(1, &field[..end]);
}
