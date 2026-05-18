#![no_std]
#![no_main]

use libc_lite;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    libc_lite::println("[netloop] start");

    let listen_fd = match libc_lite::socket(libc_lite::AF_INET, libc_lite::SOCK_STREAM, 0) {
        Ok(fd) => fd,
        Err(e) => return fail("socket listen", e),
    };

    let bind_addr = libc_lite::SockAddrIn::new_loopback(18080);
    if let Err(e) = libc_lite::bind(listen_fd, &bind_addr) {
        let _ = libc_lite::close(listen_fd);
        return fail("bind", e);
    }

    if let Err(e) = libc_lite::listen(listen_fd, 4) {
        let _ = libc_lite::close(listen_fd);
        return fail("listen", e);
    }

    let client_fd = match libc_lite::socket(libc_lite::AF_INET, libc_lite::SOCK_STREAM, 0) {
        Ok(fd) => fd,
        Err(e) => {
            let _ = libc_lite::close(listen_fd);
            return fail("socket client", e);
        }
    };

    if let Err(e) = libc_lite::connect(client_fd, &bind_addr) {
        let _ = libc_lite::close(client_fd);
        let _ = libc_lite::close(listen_fd);
        return fail("connect", e);
    }

    let accepted_fd = match libc_lite::accept(listen_fd, None) {
        Ok(fd) => fd,
        Err(e) => {
            let _ = libc_lite::close(client_fd);
            let _ = libc_lite::close(listen_fd);
            return fail("accept", e);
        }
    };

    let payload = b"ping-loopback";
    if let Err(e) = libc_lite::send(client_fd, payload, 0) {
        cleanup(listen_fd, client_fd, accepted_fd);
        return fail("send", e);
    }

    let mut rx = [0u8; 64];
    let n = match libc_lite::recv(accepted_fd, &mut rx, 0) {
        Ok(n) => n,
        Err(e) => {
            cleanup(listen_fd, client_fd, accepted_fd);
            return fail("recv", e);
        }
    };

    if &rx[..n] != payload {
        libc_lite::println("[netloop] FAIL payload mismatch");
        cleanup(listen_fd, client_fd, accepted_fd);
        return 2;
    }

    libc_lite::println("[netloop] PASS");
    cleanup(listen_fd, client_fd, accepted_fd);
    0
}

fn fail(step: &str, err: i64) -> i32 {
    libc_lite::print("[netloop] FAIL ");
    libc_lite::print(step);
    libc_lite::print(" err=");
    let mut b = [0u8; 20];
    libc_lite::print(fmt_i64(err, &mut b));
    libc_lite::print("\n");
    1
}

fn cleanup(listen_fd: i32, client_fd: i32, accepted_fd: i32) {
    let _ = libc_lite::shutdown(client_fd, libc_lite::SHUT_RDWR);
    let _ = libc_lite::shutdown(accepted_fd, libc_lite::SHUT_RDWR);
    let _ = libc_lite::close(client_fd);
    let _ = libc_lite::close(accepted_fd);
    let _ = libc_lite::close(listen_fd);
}

fn fmt_i64(n: i64, buf: &mut [u8; 20]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut v = if n < 0 { -n } else { n } as u64;
    let mut i = buf.len();
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    if n < 0 {
        i -= 1;
        buf[i] = b'-';
    }
    unsafe { core::str::from_utf8_unchecked(&buf[i..]) }
}
