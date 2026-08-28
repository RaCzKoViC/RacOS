// nc — TCP connect and listen (ROADMAP v0.2 §2.3).
//
// Usage:
//   nc HOST PORT     connect, then relay stdin -> socket and socket -> stdout
//   nc -l PORT       listen on PORT, accept one connection, then relay
//
// Both modes stop when either side closes. The relay is a poll loop rather
// than a thread pair: RacOS has no threads in userland, and poll() is already
// how racsh and racterm multiplex.
//
// TCP only. UDP would need SOCK_DGRAM plumbing through sys_send/sys_recv that
// the kernel does not offer yet, and pretending otherwise would be worse than
// saying so.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;
use libc_lite::{PollFd, SockAddrIn, AF_INET, SOCK_STREAM};

const POLLIN: i16 = 0x001;
/// How long each poll waits before looping. Short enough to notice EOF on
/// stdin promptly, long enough not to spin the CPU.
const POLL_SLICE_MS: i32 = 100;

fn err(msg: &[u8]) {
    let _ = libc_lite::write(2, b"nc: ");
    let _ = libc_lite::write(2, msg);
    let _ = libc_lite::write(2, b"\n");
}

fn parse_port(s: &[u8]) -> Option<u16> {
    if s.is_empty() {
        return None;
    }
    let mut n: u32 = 0;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (b - b'0') as u32;
        if n > 65535 {
            return None;
        }
    }
    if n == 0 {
        None
    } else {
        Some(n as u16)
    }
}

/// Resolve a host operand: dotted quad if it parses as one, otherwise DNS.
fn resolve(host: &[u8]) -> Option<[u8; 4]> {
    if let Some(ip) = parse_dotted_quad(host) {
        return Some(ip);
    }
    // gethostbyname is length-delimited, not NUL-terminated: it hands
    // `name.len()` to the kernel, so a trailing NUL becomes part of the
    // queried name.
    libc_lite::gethostbyname(host).ok()
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

/// Pump bytes both ways until either end closes. Returns the exit status.
fn relay(sock: i32) -> i32 {
    let mut buf = [0u8; 1024];
    loop {
        // Watch stdin and the socket together. Two entries, so one slow peer
        // never starves the other.
        let mut fds = [
            PollFd {
                fd: 0,
                events: POLLIN,
                revents: 0,
            },
            PollFd {
                fd: sock,
                events: POLLIN,
                revents: 0,
            },
        ];

        if libc_lite::poll(&mut fds, POLL_SLICE_MS).is_err() {
            return 1;
        }

        if fds[1].revents & POLLIN != 0 {
            match libc_lite::recv(sock, &mut buf, 0) {
                Ok(0) => return 0, // peer closed
                Ok(n) => {
                    let _ = libc_lite::write(1, &buf[..n]);
                }
                Err(_) => return 1,
            }
        }

        if fds[0].revents & POLLIN != 0 {
            match libc_lite::read(0, &mut buf) {
                Ok(0) => {
                    // stdin EOF: half-close so the peer sees it, then keep
                    // draining until the peer closes too.
                    let _ = libc_lite::shutdown(sock, libc_lite::SHUT_WR);
                    return drain(sock, &mut buf);
                }
                Ok(n) => {
                    if libc_lite::send(sock, &buf[..n], 0).is_err() {
                        return 1;
                    }
                }
                Err(_) => return 1,
            }
        }
    }
}

/// After stdin closes, keep reading the socket until the peer finishes.
fn drain(sock: i32, buf: &mut [u8]) -> i32 {
    loop {
        match libc_lite::recv(sock, buf, 0) {
            Ok(0) => return 0,
            Ok(n) => {
                let _ = libc_lite::write(1, &buf[..n]);
            }
            Err(_) => return 1,
        }
    }
}

fn do_listen(port: u16) -> i32 {
    let fd = match libc_lite::socket(AF_INET, SOCK_STREAM, 0) {
        Ok(fd) => fd,
        Err(_) => {
            err(b"cannot create socket");
            return 1;
        }
    };
    // 0.0.0.0: accept on every address the stack answers for.
    let addr = SockAddrIn::new([0, 0, 0, 0], port);
    if libc_lite::bind(fd, &addr).is_err() {
        err(b"cannot bind (port in use?)");
        let _ = libc_lite::close(fd);
        return 1;
    }
    if libc_lite::listen(fd, 1).is_err() {
        err(b"cannot listen");
        let _ = libc_lite::close(fd);
        return 1;
    }

    let conn = match libc_lite::accept(fd, None) {
        Ok(c) => c,
        Err(_) => {
            err(b"accept failed");
            let _ = libc_lite::close(fd);
            return 1;
        }
    };
    let status = relay(conn);
    let _ = libc_lite::close(conn);
    let _ = libc_lite::close(fd);
    status
}

fn do_connect(host: &[u8], port: u16) -> i32 {
    let ip = match resolve(host) {
        Some(ip) => ip,
        None => {
            err(b"cannot resolve host");
            return 1;
        }
    };
    let fd = match libc_lite::socket(AF_INET, SOCK_STREAM, 0) {
        Ok(fd) => fd,
        Err(_) => {
            err(b"cannot create socket");
            return 1;
        }
    };
    let addr = SockAddrIn::new(ip, port);
    if libc_lite::connect(fd, &addr).is_err() {
        err(b"connection refused");
        let _ = libc_lite::close(fd);
        return 1;
    }
    let status = relay(fd);
    let _ = libc_lite::close(fd);
    status
}

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let n = libc_lite::arg_count(argc);

    // nc -l PORT
    if n == 3 && libc_lite::arg(argv, 1) == Some(&b"-l"[..]) {
        return match libc_lite::arg(argv, 2).and_then(parse_port) {
            Some(p) => do_listen(p),
            None => {
                err(b"invalid port");
                2
            }
        };
    }

    // nc HOST PORT
    if n == 3 {
        let host = match libc_lite::arg(argv, 1) {
            Some(h) => h,
            None => return 2,
        };
        return match libc_lite::arg(argv, 2).and_then(parse_port) {
            Some(p) => do_connect(host, p),
            None => {
                err(b"invalid port");
                2
            }
        };
    }

    let _ = libc_lite::write(2, b"usage: nc HOST PORT | nc -l PORT\n");
    2
}
