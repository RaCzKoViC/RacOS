// sed — MVP byte-level stream editor (T3.3).
//
// Supported scripts (single command per invocation, no addresses):
//   s/X/Y/      substitute first occurrence of X with Y on each line
//   s/X/Y/g     substitute every occurrence of X with Y on each line
//   d           delete the line (no default print)
//   p           print the line explicitly (in addition to the default print)
//
// Flags:
//   -n          suppress the default print; only `p` makes a line visible
//
// Out of scope for MVP: regular expressions, addresses (line numbers / regex
// match), multiple commands separated by `;` or `-e`, hold space, branching.
// Patterns and replacements are matched byte-by-byte. The script's delimiter
// is always `/`; embed-escape (`\/`) is not yet supported, so patterns
// containing `/` are rejected with a parse error.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

const MAX_INPUT: usize = 64 * 1024;

#[no_mangle]
pub extern "C" fn main(_argc: i32, argv: *const *const u8) -> i32 {
    // ── Parse args: optional `-n` then the script ────────────────────
    let mut suppress_default = false;
    let mut script_idx = 1usize;
    if libc_lite::arg(argv, 1) == Some(&b"-n"[..]) {
        suppress_default = true;
        script_idx = 2;
    }
    let script = match libc_lite::arg(argv, script_idx) {
        Some(s) => s,
        None => {
            let _ = libc_lite::write(2, b"sed: missing script\n");
            return 1;
        }
    };

    let cmd = match parse_command(script) {
        Ok(c) => c,
        Err(msg) => {
            let _ = libc_lite::write(2, b"sed: ");
            let _ = libc_lite::write(2, msg);
            let _ = libc_lite::write(2, b"\n");
            return 2;
        }
    };

    // ── Read entire stdin into a buffer (capped) ──────────────────────
    let mut input = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match libc_lite::read(0, &mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if input.len() + n > MAX_INPUT {
                    let _ = libc_lite::write(2, b"sed: input exceeds 64 KiB\n");
                    return 1;
                }
                input.extend_from_slice(&chunk[..n]);
            }
            Err(_) => {
                let _ = libc_lite::write(2, b"sed: read error\n");
                return 1;
            }
        }
    }

    // ── Split into lines and emit ─────────────────────────────────────
    // Iterate using a manual position so we know if the final segment
    // had a trailing newline (preserve that on output) or was a
    // newline-less tail (still emit but flag accordingly so we don't
    // append a spurious '\n').
    let mut pos = 0usize;
    while pos <= input.len() {
        let nl = input[pos..].iter().position(|&b| b == b'\n');
        let (line_end, had_nl) = match nl {
            Some(i) => (pos + i, true),
            None => (input.len(), false),
        };
        let line = &input[pos..line_end];

        emit_line(line, &cmd, suppress_default, had_nl);

        if !had_nl {
            break;
        }
        pos = line_end + 1;
    }

    0
}

// ─────────────────────────────────────────────────────────────────────
// Script parsing
// ─────────────────────────────────────────────────────────────────────

enum Command<'a> {
    /// s/pattern/replacement/[g]
    Substitute {
        pattern: &'a [u8],
        replacement: &'a [u8],
        global: bool,
    },
    /// d
    Delete,
    /// p
    Print,
}

fn parse_command(script: &[u8]) -> Result<Command<'_>, &'static [u8]> {
    if script == b"d" {
        return Ok(Command::Delete);
    }
    if script == b"p" {
        return Ok(Command::Print);
    }

    // s/X/Y/[g] — accept only `/` as delimiter for MVP.
    if script.len() < 4 || !script.starts_with(b"s/") {
        return Err(b"unknown command");
    }
    // Find the next two unescaped `/`. We don't support `\/` escaping
    // yet, so a pattern containing `/` is rejected.
    let sep1 = 1usize; // index of the first `/`
    let sep2 = match script[sep1 + 1..].iter().position(|&b| b == b'/') {
        Some(i) => sep1 + 1 + i,
        None => return Err(b"missing second `/` in s///"),
    };
    let sep3 = match script[sep2 + 1..].iter().position(|&b| b == b'/') {
        Some(i) => sep2 + 1 + i,
        None => return Err(b"missing third `/` in s///"),
    };
    let pattern = &script[sep1 + 1..sep2];
    if pattern.is_empty() {
        return Err(b"empty pattern in s///");
    }
    let replacement = &script[sep2 + 1..sep3];
    let flags = &script[sep3 + 1..];
    let global = match flags {
        b"" => false,
        b"g" => true,
        _ => return Err(b"only `g` flag is supported on s///"),
    };
    Ok(Command::Substitute {
        pattern,
        replacement,
        global,
    })
}

// ─────────────────────────────────────────────────────────────────────
// Line emission
// ─────────────────────────────────────────────────────────────────────

fn emit_line(line: &[u8], cmd: &Command, suppress_default: bool, had_nl: bool) {
    match cmd {
        Command::Substitute {
            pattern,
            replacement,
            global,
        } => {
            let out = substitute(line, pattern, replacement, *global);
            if !suppress_default {
                let _ = libc_lite::write(1, &out);
                if had_nl {
                    let _ = libc_lite::write(1, b"\n");
                }
            }
        }
        Command::Delete => {
            // Drop the line — nothing to print.
            let _ = (line, suppress_default, had_nl);
        }
        Command::Print => {
            // Default print AND explicit print => the line appears twice
            // (matches GNU sed). With -n, only the explicit print fires.
            if !suppress_default {
                let _ = libc_lite::write(1, line);
                if had_nl {
                    let _ = libc_lite::write(1, b"\n");
                }
            }
            let _ = libc_lite::write(1, line);
            if had_nl {
                let _ = libc_lite::write(1, b"\n");
            }
        }
    }
}

fn substitute(line: &[u8], pattern: &[u8], replacement: &[u8], global: bool) -> Vec<u8> {
    if pattern.is_empty() {
        return line.to_vec();
    }
    let mut out = Vec::with_capacity(line.len());
    let mut i = 0usize;
    let mut replaced = false;
    while i < line.len() {
        if (!replaced || global) && line[i..].starts_with(pattern) {
            out.extend_from_slice(replacement);
            i += pattern.len();
            replaced = true;
        } else {
            out.push(line[i]);
            i += 1;
        }
    }
    out
}
