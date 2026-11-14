#![no_std]
#![no_main]

use libc_lite;

/// sleep — delay for a specified number of seconds.
///
/// Through `nanosleep`, which blocks in the kernel. The old version read
/// the clock in a tight loop instead, and got two things wrong for it: it
/// burned a core for the whole delay, and it compared whole seconds, so
/// `sleep 1` started at 10.99 s returned 10 ms later. The first one also
/// wedged the guest until 2026-09 - a process that never yields could not
/// be preempted (see arch::idt::timer_handler) - which is how a busy-wait
/// in the smallest tool in the tree came to stop the machine.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        let _ = libc_lite::write(2, b"sleep: missing operand\n");
        return 1;
    }

    let arg = unsafe { arg_name(argv, 1) };
    let secs = parse_u64(arg);
    if secs == 0 && arg != b"0" {
        let _ = libc_lite::write(2, b"sleep: invalid time interval\n");
        return 1;
    }

    if libc_lite::nanosleep(secs, 0).is_err() {
        let _ = libc_lite::write(
            2,
            b"sleep: interrupted
",
        );
        return 1;
    }

    0
}

fn parse_u64(s: &[u8]) -> u64 {
    let mut val: u64 = 0;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as u64;
        } else {
            break;
        }
    }
    val
}

unsafe fn arg_name(argv: *const *const u8, i: usize) -> &'static [u8] {
    let ptr = *argv.add(i);
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}
