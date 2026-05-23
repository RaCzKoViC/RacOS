#![no_std]
#![no_main]
#![deny(unsafe_code)]

use libc_lite;

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let uid = libc_lite::getuid();
    let gid = libc_lite::getgid();
    let euid = libc_lite::geteuid();
    let egid = libc_lite::getegid();

    libc_lite::print("uid=");
    print_u32(uid);
    libc_lite::print(" gid=");
    print_u32(gid);
    libc_lite::print(" euid=");
    print_u32(euid);
    libc_lite::print(" egid=");
    print_u32(egid);
    libc_lite::print("\n");

    0
}

fn print_u32(mut n: u32) {
    if n == 0 {
        let _ = libc_lite::write(1, b"0");
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let _ = libc_lite::write(1, &buf[i..]);
}
