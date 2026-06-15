#![no_std]
#![no_main]

use libc_lite;

/// sleep — delay for a specified number of seconds.
/// Uses clock_gettime busy-wait (no nanosleep syscall yet).
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

    // Get start time
    let mut start = libc_lite::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let _ = libc_lite::clock_gettime(libc_lite::CLOCK_MONOTONIC, &mut start);

    let target = start.tv_sec + secs;

    // Busy-wait loop
    loop {
        let mut now = libc_lite::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let _ = libc_lite::clock_gettime(libc_lite::CLOCK_MONOTONIC, &mut now);
        if now.tv_sec >= target {
            break;
        }
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
