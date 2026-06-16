// kill — send a signal to a process (v0.2 §2.1).
//
// Usage:
//   kill PID...                  # default SIGTERM
//   kill -SIG PID...             # -9, -15, -SIGTERM, -TERM
//   kill -s NAME PID...          # post-MVP (`-s` flag itself accepted,
//                                #          name resolution same as -NAME)
//
// Recognised names (canonical RacOS signal numbers from
// kernel/src/task/signal.rs:Signal::from_u8):
//   HUP=1  INT=2  QUIT=3  KILL=9  USR1=10  SEGV=11  PIPE=13
//   ALRM=14  TERM=15  CHLD=17  STOP=19  CONT=18  WINCH=28
//
// Anything else still goes through libc-lite::kill — the kernel
// will reject unknown signal numbers with EINVAL.

#![no_std]
#![no_main]

const SIGTERM: i32 = 15;

#[no_mangle]
pub extern "C" fn main(_argc: i32, argv: *const *const u8) -> i32 {
    let mut sig: i32 = SIGTERM;
    let mut i = 1usize;

    // Parse leading -SIG or `-s NAME` form.
    if let Some(arg) = libc_lite::arg(argv, i) {
        if arg == b"-s" {
            i += 1;
            let name = match libc_lite::arg(argv, i) {
                Some(n) => n,
                None => {
                    let _ = libc_lite::write(2, b"kill: -s requires a signal name\n");
                    return 1;
                }
            };
            sig = match parse_signal(name) {
                Some(s) => s,
                None => {
                    let _ = libc_lite::write(2, b"kill: unknown signal name\n");
                    return 1;
                }
            };
            i += 1;
        } else if !arg.is_empty() && arg[0] == b'-' && arg.len() > 1 {
            sig = match parse_signal(&arg[1..]) {
                Some(s) => s,
                None => {
                    let _ = libc_lite::write(2, b"kill: invalid signal\n");
                    return 1;
                }
            };
            i += 1;
        }
    }

    // Iterate PID operands.
    let mut had_error = false;
    let mut any = false;
    while let Some(arg) = libc_lite::arg(argv, i) {
        i += 1;
        any = true;
        let pid = match parse_i32(arg) {
            Some(p) => p,
            None => {
                let _ = libc_lite::write(2, b"kill: invalid pid\n");
                had_error = true;
                continue;
            }
        };
        if libc_lite::kill(pid, sig).is_err() {
            let _ = libc_lite::write(2, b"kill: could not signal pid ");
            let _ = libc_lite::write(2, arg);
            let _ = libc_lite::write(2, b"\n");
            had_error = true;
        }
    }

    if !any {
        let _ = libc_lite::write(2, b"kill: usage: kill [-SIG] PID...\n");
        return 1;
    }

    if had_error {
        1
    } else {
        0
    }
}

fn parse_signal(s: &[u8]) -> Option<i32> {
    // Numeric form first.
    if let Some(n) = parse_i32(s) {
        return Some(n);
    }
    // Strip optional SIG prefix.
    let name: &[u8] = if s.len() > 3 && &s[..3] == b"SIG" {
        &s[3..]
    } else {
        s
    };
    Some(match name {
        b"HUP" => 1,
        b"INT" => 2,
        b"QUIT" => 3,
        b"KILL" => 9,
        b"USR1" => 10,
        b"SEGV" => 11,
        b"PIPE" => 13,
        b"ALRM" => 14,
        b"TERM" => 15,
        b"CHLD" => 17,
        b"CONT" => 18,
        b"STOP" => 19,
        b"WINCH" => 28,
        _ => return None,
    })
}

fn parse_i32(s: &[u8]) -> Option<i32> {
    if s.is_empty() {
        return None;
    }
    let (sign, rest) = if s[0] == b'-' {
        (-1i32, &s[1..])
    } else {
        (1i32, s)
    };
    if rest.is_empty() {
        return None;
    }
    let mut n: i32 = 0;
    for &b in rest {
        if !(b'0'..=b'9').contains(&b) {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as i32)?;
    }
    Some(sign * n)
}
