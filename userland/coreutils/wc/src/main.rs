#![no_std]
#![no_main]

use libc_lite::{close, open, print, println, read, write};

/// Print a u64 as decimal.
fn print_num(mut n: u64) {
    if n == 0 {
        print("0");
        return;
    }
    let mut digits = [0u8; 20];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let mut buf = [0u8; 20];
    for j in 0..i {
        buf[j] = digits[i - 1 - j];
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf[..i]) };
    print(s);
}

#[derive(Clone, Copy, Default)]
struct Counts {
    lines: u64,
    words: u64,
    bytes: u64,
}

impl Counts {
    fn add(&mut self, other: &Counts) {
        self.lines += other.lines;
        self.words += other.words;
        self.bytes += other.bytes;
    }
}

/// Which counts to print. All three when no flag selects any, which is what
/// plain `wc` has always printed.
#[derive(Clone, Copy)]
struct Want {
    lines: bool,
    words: bool,
    bytes: bool,
}

impl Want {
    fn any(&self) -> bool {
        self.lines || self.words || self.bytes
    }
}

fn count_fd(fd: i32) -> Counts {
    let mut c = Counts::default();
    let mut in_word = false;
    let mut buf = [0u8; 1024];
    loop {
        match read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                c.bytes += n as u64;
                for &ch in buf.iter().take(n) {
                    if ch == b'\n' {
                        c.lines += 1;
                    }
                    if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' {
                        in_word = false;
                    } else if !in_word {
                        in_word = true;
                        c.words += 1;
                    }
                }
            }
            Err(_) => break,
        }
    }
    c
}

/// One result line: the selected counts, space-separated, then the name.
///
/// Counts are printed bare rather than column-padded, because the thing that
/// reads them here is a shell: `n=$(wc -c < f)` has to yield a number `test`
/// can compare. The old output was three padded numbers whatever flags were
/// passed, so `wc -c` returned "  258  258  16402" and every arithmetic
/// comparison built on it silently failed.
fn report(c: &Counts, want: Want, name: Option<&[u8]>) {
    let mut first = true;
    let mut field = |v: u64| {
        if !first {
            print(" ");
        }
        first = false;
        print_num(v);
    };
    if want.lines {
        field(c.lines);
    }
    if want.words {
        field(c.words);
    }
    if want.bytes {
        field(c.bytes);
    }
    if let Some(n) = name {
        print(" ");
        let _ = write(1, n);
    }
    println("");
}

fn cstr_len(p: *const u8) -> usize {
    let mut n = 0usize;
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// wc — count lines, words and bytes.
///
/// Usage: wc [-c] [-l] [-w] [FILE...]
/// Flags may be bundled (`-lc`). With no FILE, or with `-`, reads stdin.
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut want = Want {
        lines: false,
        words: false,
        bytes: false,
    };
    let mut first_operand = argc; // no operands unless one is found
    let mut status = 0;
    let mut end_of_flags = false;

    let mut i = 1;
    while i < argc {
        let p = unsafe { *argv.add(i as usize) };
        if p.is_null() {
            i += 1;
            continue;
        }
        let len = cstr_len(p);
        let arg = unsafe { core::slice::from_raw_parts(p, len) };

        // "-" is stdin, not a flag; "--" ends the flags.
        if end_of_flags || len < 2 || arg[0] != b'-' {
            first_operand = i;
            break;
        }
        if arg == b"--" {
            end_of_flags = true;
            i += 1;
            continue;
        }
        for &ch in &arg[1..] {
            match ch {
                b'l' => want.lines = true,
                b'w' => want.words = true,
                b'c' => want.bytes = true,
                // No `-m`: these tools have no multibyte handling, so a
                // character count would just be the byte count wearing a
                // different name, and would be wrong the day one arrives.
                _ => {
                    let _ = write(2, b"wc: unknown option -");
                    let _ = write(2, &[ch]);
                    let _ = write(2, b"\n");
                    return 1;
                }
            }
        }
        i += 1;
    }

    // No selection means all three, which is what plain `wc` prints.
    if !want.any() {
        want = Want {
            lines: true,
            words: true,
            bytes: true,
        };
    }

    if first_operand >= argc {
        let c = count_fd(0);
        report(&c, want, None);
        return 0;
    }

    let mut total = Counts::default();
    let mut files = 0;
    for i in first_operand..argc {
        let p = unsafe { *argv.add(i as usize) };
        if p.is_null() {
            continue;
        }
        let len = cstr_len(p);
        let arg = unsafe { core::slice::from_raw_parts(p, len) };
        files += 1;

        if arg == b"-" {
            let c = count_fd(0);
            total.add(&c);
            report(&c, want, Some(b"-"));
            continue;
        }

        // open() wants the terminating NUL, which argv already has.
        let path = unsafe { core::slice::from_raw_parts(p, len + 1) };
        match open(path, 0, 0) {
            Ok(fd) => {
                let c = count_fd(fd);
                let _ = close(fd);
                total.add(&c);
                report(&c, want, Some(arg));
            }
            Err(_) => {
                let _ = write(2, b"wc: ");
                let _ = write(2, arg);
                let _ = write(2, b": No such file or directory\n");
                status = 1;
            }
        }
    }

    // Only worth a total when there was more than one thing to total.
    if files > 1 {
        report(&total, want, Some(b"total"));
    }
    status
}
