#![no_std]
#![no_main]

use libc_lite::{
    close, connect, gethostbyname, print, println, recv, send, socket, write, SockAddrIn, AF_INET,
    SOCK_STREAM,
};

fn cstr_len(p: *const u8) -> usize {
    let mut n = 0usize;
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// Strip "http://" prefix if present. Returns the remaining slice.
fn strip_scheme(s: &[u8]) -> &[u8] {
    const HTTP: &[u8] = b"http://";
    if s.len() >= HTTP.len() && &s[..HTTP.len()] == HTTP {
        &s[HTTP.len()..]
    } else {
        s
    }
}

/// Split host[:port][/path] → (host, port, path). Defaults: port 80, path "/".
fn split_url(s: &[u8]) -> (&[u8], u16, &[u8]) {
    let mut host_end = s.len();
    let mut path_start = s.len();
    let mut colon = None;
    for (i, &b) in s.iter().enumerate() {
        if b == b'/' {
            host_end = i;
            path_start = i;
            break;
        }
        if b == b':' && colon.is_none() {
            colon = Some(i);
            host_end = i;
        }
    }
    let port = if let Some(c) = colon {
        let end = if path_start < s.len() {
            path_start
        } else {
            s.len()
        };
        parse_u16(&s[c + 1..end]).unwrap_or(80)
    } else {
        80
    };
    (&s[..host_end], port, &s[path_start..])
}

fn parse_u16(s: &[u8]) -> Option<u16> {
    let mut acc: u32 = 0;
    if s.is_empty() {
        return None;
    }
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        acc = acc * 10 + (b - b'0') as u32;
        if acc > 65535 {
            return None;
        }
    }
    Some(acc as u16)
}

fn print_u16(n: u16) {
    let mut digits = [0u8; 5];
    let mut i = 0;
    let mut v = n;
    if v == 0 {
        let _ = write(1, b"0");
        return;
    }
    while v > 0 {
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    let mut buf = [0u8; 5];
    for j in 0..i {
        buf[j] = digits[i - 1 - j];
    }
    let _ = write(1, &buf[..i]);
}

fn print_ip(ip: [u8; 4]) {
    print_u16(ip[0] as u16);
    print(".");
    print_u16(ip[1] as u16);
    print(".");
    print_u16(ip[2] as u16);
    print(".");
    print_u16(ip[3] as u16);
}

/// wget — pobierz URL przez HTTP/1.0 i wypisz odpowiedź na stdout.
/// Użycie: wget <host[:port]>[/path]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println("usage: wget <host[:port]>[/path]");
        return 1;
    }

    let arg_ptr = unsafe { *argv.add(1) };
    if arg_ptr.is_null() {
        return 1;
    }
    let raw = unsafe { core::slice::from_raw_parts(arg_ptr, cstr_len(arg_ptr)) };
    let stripped = strip_scheme(raw);
    let (host, port, path_raw) = split_url(stripped);
    let path: &[u8] = if path_raw.is_empty() { b"/" } else { path_raw };

    print("wget: resolving ");
    let _ = write(1, host);
    print("... ");
    let ip = match gethostbyname(host) {
        Ok(ip) => {
            print_ip(ip);
            print("\n");
            ip
        }
        Err(e) => {
            print("failed (err ");
            let _ = write(1, b"--");
            print(")\n");
            let _ = e;
            return 1;
        }
    };

    print("wget: connecting to ");
    print_ip(ip);
    print(":");
    print_u16(port);
    print("\n");

    let fd = match socket(AF_INET, SOCK_STREAM, 0) {
        Ok(f) => f,
        Err(_) => {
            println("wget: socket failed");
            return 1;
        }
    };

    let addr = SockAddrIn::new(ip, port);
    if connect(fd, &addr).is_err() {
        println("wget: connect failed");
        let _ = close(fd);
        return 1;
    }

    // Build the request: GET <path> HTTP/1.0\r\nHost: <host>\r\nConnection: close\r\n\r\n
    let mut req = [0u8; 512];
    let mut n = 0usize;
    fn put(buf: &mut [u8], n: &mut usize, s: &[u8]) {
        let take = s.len().min(buf.len() - *n);
        buf[*n..*n + take].copy_from_slice(&s[..take]);
        *n += take;
    }
    put(&mut req, &mut n, b"GET ");
    put(&mut req, &mut n, path);
    put(&mut req, &mut n, b" HTTP/1.0\r\nHost: ");
    put(&mut req, &mut n, host);
    put(
        &mut req,
        &mut n,
        b"\r\nConnection: close\r\nUser-Agent: racos-wget/0.1\r\n\r\n",
    );

    if send(fd, &req[..n], 0).is_err() {
        println("wget: send failed");
        let _ = close(fd);
        return 1;
    }

    // Read loop until EOF.
    let mut buf = [0u8; 1024];
    let mut total: u64 = 0;
    loop {
        match recv(fd, &mut buf, 0) {
            Ok(0) => break,
            Ok(k) => {
                let _ = write(1, &buf[..k]);
                total += k as u64;
            }
            Err(_) => {
                println("\nwget: recv error");
                break;
            }
        }
    }

    let _ = close(fd);
    print("\n--- wget done, ");
    // Print total as decimal.
    let mut tmp = [0u8; 20];
    let mut i = 0;
    if total == 0 {
        tmp[0] = b'0';
        i = 1;
    }
    let mut t = total;
    while t > 0 {
        tmp[i] = b'0' + (t % 10) as u8;
        t /= 10;
        i += 1;
    }
    let mut rev = [0u8; 20];
    for j in 0..i {
        rev[j] = tmp[i - 1 - j];
    }
    let _ = write(1, &rev[..i]);
    print(" bytes received ---\n");
    0
}
