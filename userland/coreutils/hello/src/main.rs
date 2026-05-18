#![no_std]
#![no_main]

use libc_lite;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    libc_lite::println("Hello from RacOS userland!");
    let pid = libc_lite::getpid();
    libc_lite::print("PID: ");
    // Prosty print liczby
    let mut buf = [0u8; 16];
    let s = format_u32(pid as u32, &mut buf);
    libc_lite::print(s);
    libc_lite::print("\n");
    0
}

fn format_u32(mut n: u32, buf: &mut [u8; 16]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut i = 15;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i -= 1;
    }
    unsafe { core::str::from_utf8_unchecked(&buf[i + 1..]) }
}
