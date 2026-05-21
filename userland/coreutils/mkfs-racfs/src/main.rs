#![no_std]
#![no_main]

use libc_lite::{mkfs, print, println, write};

fn cstr_len(p: *const u8) -> usize {
    let mut n = 0usize;
    unsafe { while *p.add(n) != 0 { n += 1; } }
    n
}

/// mkfs.racfs — format a block device with the racfs filesystem.
/// The device must NOT be mounted; run `umount` first if needed.
/// Usage: mkfs.racfs <device>          (e.g. `mkfs.racfs sda` or `/dev/sda`)
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println("usage: mkfs.racfs <device>");
        println("  e.g. mkfs.racfs sda   (umount /mnt first)");
        return 1;
    }
    let arg_ptr = unsafe { *argv.add(1) };
    if arg_ptr.is_null() { return 1; }
    let len = cstr_len(arg_ptr);
    let device = unsafe { core::slice::from_raw_parts(arg_ptr, len) };

    print("mkfs.racfs: formatting ");
    let _ = write(1, device);
    print(" ... ");

    match mkfs(device, b"racfs") {
        Ok(()) => {
            println("OK");
            println("(reboot or remount to use the fresh filesystem)");
            0
        }
        Err(e) => {
            print("failed (errno ");
            // Print the negative error code.
            let v = (-e) as u64;
            let mut digits = [0u8; 8];
            let mut i = 0;
            let mut t = v;
            if t == 0 { digits[0] = b'0'; i = 1; }
            while t > 0 { digits[i] = b'0' + (t % 10) as u8; t /= 10; i += 1; }
            let mut buf = [0u8; 8];
            for j in 0..i { buf[j] = digits[i - 1 - j]; }
            let _ = write(1, &buf[..i]);
            print(")\n");
            if e == -98 {
                println("  hint: device is mounted; run `umount /mnt` first");
            }
            1
        }
    }
}
