#![no_std]
#![no_main]

use libc_lite;

/// cat — kopiuj pliki (lub stdin) na stdout.
/// Bez argumentów czyta stdin. Z argumentami czyta podane pliki.
///
/// Zwraca 1, jeśli którykolwiek operand zawiódł. Wcześniej `cat` zawsze
/// kończył się zerem — wypisywał "No such file or directory" na stderr, a
/// mimo to `cat missing && echo ok` drukowało `ok`, przez co każdy skrypt
/// rozgałęziający się na powodzeniu `cat` cicho brał złą gałąź.
///
/// Zapis na stdout idzie przez `write_all`: pipe mieści 4 KiB, a `write`
/// bierze tyle, ile się mieści (albo odmawia EAGAIN) — `cat big | wc -c`
/// odpowiadało 4096 dla pliku 8320 B, bo reszta szła w `let _ =`. Błąd
/// zapisu (np. `> /dev/full`) kończy z komunikatem i statusem 1.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut status = 0;

    if argc <= 1 {
        // No arguments — copy stdin to stdout
        if !cat_fd(0) {
            status = 1;
        }
    } else {
        for i in 1..argc {
            let arg_ptr = unsafe { *argv.add(i as usize) };
            if arg_ptr.is_null() {
                continue;
            }
            // Find length
            let mut len = 0usize;
            unsafe {
                while *arg_ptr.add(len) != 0 {
                    len += 1;
                }
            }
            if len == 1 && unsafe { *arg_ptr } == b'-' {
                if !cat_fd(0) {
                    status = 1; // "-" means stdin
                }
            } else {
                // Build null-terminated path
                let path = unsafe { core::slice::from_raw_parts(arg_ptr, len + 1) }; // includes null
                match libc_lite::open(path, 0, 0) {
                    Ok(fd) => {
                        if !cat_fd(fd) {
                            status = 1;
                        }
                        let _ = libc_lite::close(fd);
                    }
                    Err(_) => {
                        let _ = libc_lite::write(2, b"cat: ");
                        let _ = libc_lite::write(2, &path[..len]);
                        let _ = libc_lite::write(2, b": No such file or directory\n");
                        status = 1;
                    }
                }
            }
        }
    }
    status
}

/// Copy `fd` to stdout. Returns false if a write failed - the error is
/// reported once and the rest of this operand is skipped, since every
/// further write would fail the same way.
fn cat_fd(fd: i32) -> bool {
    let mut buf = [0u8; 4096];
    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => return true,
            Ok(n) => {
                if libc_lite::write_all(1, &buf[..n]).is_err() {
                    let _ = libc_lite::write_all(2, b"cat: write error\n");
                    return false;
                }
            }
            Err(_) => return true,
        }
    }
}
