// whoami — print the effective user name (v0.2 §2.1).
//
// MVP: euid → name lookup via a hard-coded table. RacOS doesn't have
// `/etc/passwd` yet, so user-name resolution that goes beyond
// `0 → root` is post-MVP — for any other euid we print the decimal
// number, which is at least informative and matches what GNU
// coreutils does on a system without a passwd entry for that uid.
//
// Once /etc/passwd exists, swap the `name_for_uid` body for a real
// parse + lookup; the rest of the binary stays the same.

#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let euid = libc_lite::geteuid();
    let (s, len) = match name_for_uid(euid) {
        Some(name) => (name, name.len()),
        None => {
            // No /etc/passwd lookup yet — print the numeric uid.
            let mut buf = [0u8; 10];
            let n = write_u32_into(euid, &mut buf);
            let _ = libc_lite::write(1, &buf[..n]);
            let _ = libc_lite::write(1, b"\n");
            return 0;
        }
    };
    let _ = libc_lite::write(1, &s.as_bytes()[..len]);
    let _ = libc_lite::write(1, b"\n");
    0
}

fn name_for_uid(uid: u32) -> Option<&'static str> {
    match uid {
        0 => Some("root"),
        _ => None,
    }
}

fn write_u32_into(mut n: u32, dst: &mut [u8]) -> usize {
    if n == 0 {
        dst[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    for j in 0..i {
        dst[j] = tmp[i - 1 - j];
    }
    i
}
