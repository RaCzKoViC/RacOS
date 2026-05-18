#![no_std]
#![no_main]

use libc_lite::{clock_gettime, Timespec, CLOCK_MONOTONIC, print, println};

fn print_num(mut n: u64) {
    if n == 0 {
        print("0");
        return;
    }
    let mut digits = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let mut buf = [0u8; 20];
    for j in 0..i {
        buf[j] = digits[i - 1 - j];
    }
    print(unsafe { core::str::from_utf8_unchecked(&buf[..i]) });
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
    match clock_gettime(CLOCK_MONOTONIC, &mut ts) {
        Ok(()) => {
            let hours = ts.tv_sec / 3600;
            let mins = (ts.tv_sec % 3600) / 60;
            let secs = ts.tv_sec % 60;

            print("up ");
            if hours > 0 {
                print_num(hours);
                print("h ");
            }
            if mins > 0 || hours > 0 {
                print_num(mins);
                print("m ");
            }
            print_num(secs);
            println("s");
        }
        Err(_) => {
            println("uptime: clock_gettime failed");
        }
    }
    0
}
