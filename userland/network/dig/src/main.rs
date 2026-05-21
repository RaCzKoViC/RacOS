#![no_std]
#![no_main]

use libc_lite::{gethostbyname, println, print, write};

fn print_u8(n: u8) {
    let mut digits = [0u8; 3];
    let mut i = 0;
    let mut v = n;
    if v == 0 {
        let _ = write(1, b"0");
        return;
    }
    while v > 0 {
        digits[i] = b'0' + (v % 10);
        v /= 10;
        i += 1;
    }
    let mut buf = [0u8; 3];
    for j in 0..i {
        buf[j] = digits[i - 1 - j];
    }
    let _ = write(1, &buf[..i]);
}

fn cstr_len(p: *const u8) -> usize {
    let mut n = 0usize;
    unsafe {
        while *p.add(n) != 0 { n += 1; }
    }
    n
}

/// dig — minimalny resolver A.
/// Użycie: dig <hostname>
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println("usage: dig <hostname>");
        return 1;
    }

    let name_ptr = unsafe { *argv.add(1) };
    if name_ptr.is_null() { return 1; }
    let len = cstr_len(name_ptr);
    let name = unsafe { core::slice::from_raw_parts(name_ptr, len) };

    match gethostbyname(name) {
        Ok(ip) => {
            // Print: "<name> has address a.b.c.d"
            let _ = write(1, name);
            print(" has address ");
            print_u8(ip[0]); print(".");
            print_u8(ip[1]); print(".");
            print_u8(ip[2]); print(".");
            print_u8(ip[3]);
            print("\n");
            0
        }
        Err(_) => {
            print("dig: resolution failed for ");
            let _ = write(1, name);
            print("\n");
            1
        }
    }
}
