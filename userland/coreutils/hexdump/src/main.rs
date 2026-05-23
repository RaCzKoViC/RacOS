#![no_std]
#![no_main]

use libc_lite;

/// hexdump — display file contents in hex
/// Usage: hexdump [file...]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        hexdump_fd(0);
    } else {
        for i in 1..argc {
            let file_ptr = unsafe { *argv.add(i as usize) };
            if file_ptr.is_null() {
                continue;
            }
            let mut len = 0;
            unsafe {
                while *file_ptr.add(len) != 0 {
                    len += 1;
                }
            }
            let path = unsafe { core::slice::from_raw_parts(file_ptr, len + 1) };
            match libc_lite::open(path, 0, 0) {
                Ok(fd) => {
                    hexdump_fd(fd);
                    let _ = libc_lite::close(fd);
                }
                Err(_) => {
                    let _ = libc_lite::write(2, b"hexdump: ");
                    let _ = libc_lite::write(2, &path[..len]);
                    let _ = libc_lite::write(2, b": No such file or directory\n");
                }
            }
        }
    }
    0
}

fn hexdump_fd(fd: i32) {
    let mut buf = [0u8; 512];
    let mut offset = 0u32;

    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for i in (0..n).step_by(16) {
                    print_hexdump_line(offset, &buf[i..core::cmp::min(i + 16, n)]);
                    offset += 16 as u32;
                }
            }
            Err(_) => break,
        }
    }
}

fn print_hexdump_line(offset: u32, data: &[u8]) {
    let mut buf = [0u8; 100];
    let mut pos = 0;

    // Print offset in hex
    write_u32_hex(offset, &mut buf, &mut pos);
    buf[pos] = b' ';
    pos += 1;
    buf[pos] = b' ';
    pos += 1;

    // Print 16 bytes in hex
    for (i, &byte) in data.iter().enumerate() {
        write_u8_hex(byte, &mut buf, &mut pos);
        buf[pos] = b' ';
        pos += 1;
        if i == 7 {
            buf[pos] = b' ';
            pos += 1;
        }
    }

    // Pad to align ASCII section
    while pos < 56 {
        buf[pos] = b' ';
        pos += 1;
    }

    buf[pos] = b'|';
    pos += 1;

    // Print ASCII representation
    for &byte in data {
        if byte >= 32 && byte < 127 {
            buf[pos] = byte;
        } else {
            buf[pos] = b'.';
        }
        pos += 1;
    }

    buf[pos] = b'|';
    pos += 1;
    buf[pos] = b'\n';
    pos += 1;

    let _ = libc_lite::write(1, &buf[..pos]);
}

fn write_u32_hex(val: u32, buf: &mut [u8], pos: &mut usize) {
    let mut temp = [0u8; 8];

    for i in 0..8 {
        temp[7 - i] = to_hex_digit(((val >> (i * 4)) & 0xf) as u8);
    }

    for &b in &temp {
        if *pos < buf.len() {
            buf[*pos] = b;
            *pos += 1;
        }
    }
}

fn write_u8_hex(byte: u8, buf: &mut [u8], pos: &mut usize) {
    let nib_h = (byte >> 4) & 0xf;
    let nib_l = byte & 0xf;

    if *pos < buf.len() {
        buf[*pos] = to_hex_digit(nib_h);
        *pos += 1;
    }
    if *pos < buf.len() {
        buf[*pos] = to_hex_digit(nib_l);
        *pos += 1;
    }
}

fn to_hex_digit(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + (nibble - 10)
    }
}
