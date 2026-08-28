// ping — ICMP echo (ROADMAP v0.2 §2.3).
//
// Usage: ping [-c COUNT] [-W TIMEOUT_MS] HOST
//
// One echo per iteration, sent and awaited by the kernel (SYS_ICMP_ECHO):
// building raw ICMP in userland would need a raw-socket API the kernel does
// not expose, and the stack already had send_icmp_echo.
//
// HOST may be a dotted quad or a name; names go through gethostbyname, so
// `ping example.com` exercises DNS and ICMP together.
//
// Under QEMU's user-mode (slirp) networking, only the gateway answers ICMP --
// slirp does not forward echo requests out to the internet. So `ping 10.0.2.2`
// replies and `ping example.com` resolves but reports 100% loss. That is the
// emulated network's limit, not a fault in the stack; on a bridged or tap
// interface the same binary reaches real hosts.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;

const DEFAULT_COUNT: u32 = 4;
const DEFAULT_TIMEOUT_MS: u32 = 2000;
/// Gap between echoes, matching the conventional one-per-second cadence.
const INTERVAL_MS: u64 = 1000;

fn push_num(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut d = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        d[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        out.push(d[i] as char);
    }
}

fn push_ip(out: &mut String, ip: &[u8; 4]) {
    for (i, o) in ip.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        push_num(out, *o as u64);
    }
}

fn parse_u32(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(n)
}

fn parse_dotted_quad(s: &[u8]) -> Option<[u8; 4]> {
    let text = core::str::from_utf8(s).ok()?;
    let mut out = [0u8; 4];
    let mut parts = 0;
    for field in text.split('.') {
        if parts == 4 || field.is_empty() || field.len() > 3 {
            return None;
        }
        let mut v: u32 = 0;
        for b in field.bytes() {
            if !b.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (b - b'0') as u32;
        }
        if v > 255 {
            return None;
        }
        out[parts] = v as u8;
        parts += 1;
    }
    if parts == 4 {
        Some(out)
    } else {
        None
    }
}

fn resolve(host: &[u8]) -> Option<[u8; 4]> {
    if let Some(ip) = parse_dotted_quad(host) {
        return Some(ip);
    }
    // gethostbyname is length-delimited, not NUL-terminated: it passes
    // `name.len()` straight to the kernel. Appending a NUL would make the
    // queried name "example.com\0" -- a trailing label the resolver never
    // matches.
    libc_lite::gethostbyname(host).ok()
}

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut count = DEFAULT_COUNT;
    let mut timeout = DEFAULT_TIMEOUT_MS;
    let mut host: Option<&[u8]> = None;

    let n = libc_lite::arg_count(argc);
    let mut i = 1;
    while i < n {
        match libc_lite::arg(argv, i) {
            Some(b"-c") => {
                i += 1;
                match libc_lite::arg(argv, i).and_then(parse_u32) {
                    Some(c) if c > 0 => count = c,
                    _ => {
                        let _ = libc_lite::write(2, b"ping: -c needs a positive count\n");
                        return 2;
                    }
                }
            }
            Some(b"-W") => {
                i += 1;
                match libc_lite::arg(argv, i).and_then(parse_u32) {
                    Some(t) if t > 0 => timeout = t,
                    _ => {
                        let _ = libc_lite::write(2, b"ping: -W needs a positive timeout\n");
                        return 2;
                    }
                }
            }
            Some(arg) if arg.starts_with(b"-") && arg.len() > 1 => {
                let _ = libc_lite::write(2, b"ping: unknown option: ");
                let _ = libc_lite::write(2, arg);
                let _ = libc_lite::write(2, b"\nusage: ping [-c COUNT] [-W MS] HOST\n");
                return 2;
            }
            Some(arg) => host = Some(arg),
            None => break,
        }
        i += 1;
    }

    let host = match host {
        Some(h) => h,
        None => {
            let _ = libc_lite::write(2, b"usage: ping [-c COUNT] [-W MS] HOST\n");
            return 2;
        }
    };

    let ip = match resolve(host) {
        Some(ip) => ip,
        None => {
            let _ = libc_lite::write(2, b"ping: cannot resolve ");
            let _ = libc_lite::write(2, host);
            let _ = libc_lite::write(2, b"\n");
            return 1;
        }
    };

    let mut header = String::from("PING ");
    header.push_str(core::str::from_utf8(host).unwrap_or("?"));
    header.push_str(" (");
    push_ip(&mut header, &ip);
    header.push_str(") 32 bytes of data\n");
    let _ = libc_lite::write(1, header.as_bytes());

    let mut received = 0u32;
    let mut rtt_total = 0u64;
    let mut rtt_min = u64::MAX;
    let mut rtt_max = 0u64;

    for seq in 0..count {
        let mut line = String::new();
        match libc_lite::icmp_echo(&ip, timeout) {
            Ok(rtt) => {
                received += 1;
                rtt_total += rtt;
                if rtt < rtt_min {
                    rtt_min = rtt;
                }
                if rtt > rtt_max {
                    rtt_max = rtt;
                }
                line.push_str("32 bytes from ");
                push_ip(&mut line, &ip);
                line.push_str(": icmp_seq=");
                push_num(&mut line, seq as u64 + 1);
                line.push_str(" time=");
                push_num(&mut line, rtt);
                line.push_str(" ms\n");
            }
            Err(_) => {
                line.push_str("Request timeout for icmp_seq=");
                push_num(&mut line, seq as u64 + 1);
                line.push('\n');
            }
        }
        let _ = libc_lite::write(1, line.as_bytes());

        // No sleep after the final echo: it would just delay the summary.
        if seq + 1 < count {
            let _ = libc_lite::nanosleep(0, INTERVAL_MS * 1_000_000);
        }
    }

    let mut summary = String::from("\n--- ");
    summary.push_str(core::str::from_utf8(host).unwrap_or("?"));
    summary.push_str(" ping statistics ---\n");
    push_num(&mut summary, count as u64);
    summary.push_str(" packets transmitted, ");
    push_num(&mut summary, received as u64);
    summary.push_str(" received, ");
    push_num(
        &mut summary,
        ((count - received) as u64 * 100) / count as u64,
    );
    summary.push_str("% packet loss\n");
    if received > 0 {
        summary.push_str("rtt min/avg/max = ");
        push_num(&mut summary, rtt_min);
        summary.push('/');
        push_num(&mut summary, rtt_total / received as u64);
        summary.push('/');
        push_num(&mut summary, rtt_max);
        summary.push_str(" ms\n");
    }
    let _ = libc_lite::write(1, summary.as_bytes());

    // Conventional: success only if something answered.
    if received > 0 {
        0
    } else {
        1
    }
}
