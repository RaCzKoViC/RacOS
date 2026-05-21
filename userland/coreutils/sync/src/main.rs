#![no_std]
#![no_main]

use libc_lite::{sync, print, println, write};

/// sync — flush all dirty filesystem caches to disk.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    match sync() {
        Ok(n) => {
            print("sync: flushed ");
            // Print n as decimal.
            let v = n as u64;
            let mut digits = [0u8; 10];
            let mut i = 0;
            let mut t = v;
            if t == 0 { digits[0] = b'0'; i = 1; }
            while t > 0 { digits[i] = b'0' + (t % 10) as u8; t /= 10; i += 1; }
            let mut buf = [0u8; 10];
            for j in 0..i { buf[j] = digits[i - 1 - j]; }
            let _ = write(1, &buf[..i]);
            println(" mount(s)");
            0
        }
        Err(_) => { println("sync: failed"); 1 }
    }
}
