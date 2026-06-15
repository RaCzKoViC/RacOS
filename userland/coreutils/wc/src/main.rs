#![no_std]
#![no_main]

use libc_lite::{print, println, read};

/// Print a u64 as decimal.
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
    let s = unsafe { core::str::from_utf8_unchecked(&buf[..i]) };
    print(s);
}

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // Read stdin and count lines, words, bytes
    let mut lines: u64 = 0;
    let mut words: u64 = 0;
    let mut bytes: u64 = 0;
    let mut in_word = false;
    let mut buf = [0u8; 1024];

    loop {
        match read(0, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                bytes += n as u64;
                for i in 0..n {
                    let c = buf[i];
                    if c == b'\n' {
                        lines += 1;
                    }
                    if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                        in_word = false;
                    } else if !in_word {
                        in_word = true;
                        words += 1;
                    }
                }
            }
            Err(_) => break,
        }
    }

    print("  ");
    print_num(lines);
    print("  ");
    print_num(words);
    print("  ");
    print_num(bytes);
    println("");
    0
}
