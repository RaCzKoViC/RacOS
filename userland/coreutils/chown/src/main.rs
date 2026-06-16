// chown — change file owner and group (v0.2 §2.1).
//
// Usage:
//   chown OWNER PATH...           # set uid only, keep gid
//   chown OWNER:GROUP PATH...     # set both
//   chown :GROUP PATH...          # set gid only, keep uid
//
// OWNER and GROUP must be numeric uids/gids today; symbolic
// usernames need `/etc/passwd` + `/etc/group` lookup, which is post-
// MVP (no `getpwnam` in libc-lite yet).
//
// The kernel exposes `sys_chown(path, uid, gid)` and treats
// `uid == u32::MAX` / `gid == u32::MAX` as "leave unchanged".

#![no_std]
#![no_main]

const KEEP: u32 = u32::MAX;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 3 {
        let _ = libc_lite::write(2, b"chown: usage: chown OWNER[:GROUP] PATH...\n");
        return 1;
    }

    let spec = match libc_lite::arg(argv, 1) {
        Some(s) => s,
        None => return 1,
    };

    let (uid, gid) = match parse_owner_group(spec) {
        Some(pair) => pair,
        None => {
            let _ = libc_lite::write(2, b"chown: invalid owner spec\n");
            return 1;
        }
    };

    let mut had_error = false;
    let mut i = 2usize;
    while let Some(arg) = libc_lite::arg(argv, i) {
        i += 1;
        let mut path = [0u8; 256];
        if arg.len() + 1 > path.len() {
            let _ = libc_lite::write(2, b"chown: path too long\n");
            had_error = true;
            continue;
        }
        path[..arg.len()].copy_from_slice(arg);

        if libc_lite::chown(&path[..arg.len() + 1], uid, gid).is_err() {
            let _ = libc_lite::write(2, b"chown: cannot change owner of '");
            let _ = libc_lite::write(2, arg);
            let _ = libc_lite::write(2, b"'\n");
            had_error = true;
        }
    }

    if had_error {
        1
    } else {
        0
    }
}

/// Parse `owner`, `owner:group`, or `:group` into (uid, gid). Each
/// side that's omitted becomes [`KEEP`]. Numeric ids only.
fn parse_owner_group(s: &[u8]) -> Option<(u32, u32)> {
    let colon = s.iter().position(|&b| b == b':');
    match colon {
        None => {
            let uid = parse_u32(s)?;
            Some((uid, KEEP))
        }
        Some(idx) => {
            let owner = &s[..idx];
            let group = &s[idx + 1..];
            let uid = if owner.is_empty() {
                KEEP
            } else {
                parse_u32(owner)?
            };
            let gid = if group.is_empty() {
                KEEP
            } else {
                parse_u32(group)?
            };
            Some((uid, gid))
        }
    }
}

fn parse_u32(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in s {
        if !(b'0'..=b'9').contains(&b) {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}
