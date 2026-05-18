#![no_std]
#![no_main]

use libc_lite;

/// od — octal dump
/// Shows hexadecimal/octal representation of file
/// Usage: od [file...]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc <= 1 {
        od_fd(0);
    } else {
        for i in 1..argc {
            let file_ptr = unsafe { *argv.add(i as usize) };
            if file_ptr.is_null() { continue; }
            let mut len = 0;
            unsafe { while *file_ptr.add(len) != 0 { len += 1; } }
            let path = unsafe { core::slice::from_raw_parts(file_ptr, len + 1) };
            match libc_lite::open(path, 0, 0) {
                Ok(fd) => {
                    od_fd(fd);
                    let _ = libc_lite::close(fd);
                }
                Err(_) => {
                    let _ = libc_lite::write(2, b"od: ");
                    let _ = libc_lite::write(2, &path[..len]);
                    let _ = libc_lite::write(2, b": No such file or directory\n");
                }
            }
        }
    }
    0
}

fn od_fd(fd: i32) {
    let mut buf = [0u8; 512];
    let mut offset = 0u32;

    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                for i in (0..n).step_by(16) {
                    print_hex_line(offset, &buf[i..core::cmp::min(i + 16, n)]);
                    offset += 16 as u32;
                }
            }
            Err(_) => break,
        }
    }
}

fn print_hex_line(offset: u32, data: &[u8]) {
    // Print offset
    let mut buf = [0u8; 48];
    let mut pos = 0;

    // Offset in octal
    write_u32_octal(offset, &mut buf, &mut pos);
    buf[pos] = b' ';
    pos += 1;

    // Hex bytes
    for (i, &byte) in data.iter().enumerate() {
        if i > 0 {
            buf[pos] = b' ';
            pos += 1;
        }
        write_u8_hex(byte, &mut buf, &mut pos);
    }

    buf[pos] = b'\n';
    pos += 1;

    let _ = libc_lite::write(1, &buf[..pos]);
}

fn write_u32_octal(mut val: u32, buf: &mut [u8], pos: &mut usize) {
    if val == 0 {
        if *pos < buf.len() {
            buf[*pos] = b'0';
            *pos += 1;
        }
        return;
    }

    let mut temp = [0u8; 16];
    let mut len = 0;
    while val > 0 {
        temp[len] = b'0' + (val % 8) as u8;
        val /= 8;
        len += 1;
    }

    for i in (0..len).rev() {
        if *pos < buf.len() {
            buf[*pos] = temp[i];
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
