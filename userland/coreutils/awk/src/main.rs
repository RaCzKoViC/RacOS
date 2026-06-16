// awk — MVP pattern-action language (T3.3).
//
// Supported program structure:
//   BEGIN { actions }        // run once before input
//   { actions }              // run for each input line (the "main" block)
//   END { actions }          // run once after input
//
// Block ordering: BEGIN first, then the main block, then END. Any subset
// is allowed (e.g. just BEGIN, or just a main block).
//
// Supported actions (semicolon-separated within a block):
//   print                    equivalent to `print $0`
//   print $N                 print field N (1-indexed; $0 = whole line)
//   print "literal"          print a literal string
//   print $1, "x", $2        print multiple items separated by " " (OFS)
//
// Flags:
//   -F sep                   single-byte field separator (default: runs of
//                            whitespace, i.e. ' ' and '\t')
//
// Out of scope for MVP: regex patterns (e.g. `/foo/ { ... }`), variable
// assignment, arithmetic/string expressions, NR/NF as user-readable names,
// getline, multi-char -F, escape sequences inside string literals.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

const MAX_INPUT: usize = 64 * 1024;
const MAX_SCRIPT: usize = 4 * 1024;

#[no_mangle]
pub extern "C" fn main(_argc: i32, argv: *const *const u8) -> i32 {
    // ── Parse args: optional `-F sep`, then the script ────────────────
    let mut sep: Option<u8> = None;
    let mut script_idx = 1usize;
    if libc_lite::arg(argv, 1) == Some(&b"-F"[..]) {
        let s = match libc_lite::arg(argv, 2) {
            Some(s) if s.len() == 1 => s[0],
            _ => {
                let _ = libc_lite::write(2, b"awk: -F needs a single-byte separator\n");
                return 2;
            }
        };
        sep = Some(s);
        script_idx = 3;
    }
    let script = match libc_lite::arg(argv, script_idx) {
        Some(s) => s,
        None => {
            let _ = libc_lite::write(2, b"awk: missing program\n");
            return 1;
        }
    };
    if script.len() > MAX_SCRIPT {
        let _ = libc_lite::write(2, b"awk: program exceeds 4 KiB\n");
        return 1;
    }

    let program = match parse_program(script) {
        Ok(p) => p,
        Err(msg) => {
            let _ = libc_lite::write(2, b"awk: ");
            let _ = libc_lite::write(2, msg);
            let _ = libc_lite::write(2, b"\n");
            return 2;
        }
    };

    // ── BEGIN block ───────────────────────────────────────────────────
    let no_fields: [&[u8]; 0] = [];
    run_block(&program.begin, b"", &no_fields);

    // ── Main loop: read stdin and run main block per line ─────────────
    if !program.main.is_empty() {
        let mut input: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match libc_lite::read(0, &mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if input.len() + n > MAX_INPUT {
                        let _ = libc_lite::write(2, b"awk: input exceeds 64 KiB\n");
                        return 1;
                    }
                    input.extend_from_slice(&chunk[..n]);
                }
                Err(_) => {
                    let _ = libc_lite::write(2, b"awk: read error\n");
                    return 1;
                }
            }
        }

        let mut pos = 0usize;
        while pos < input.len() {
            let nl = input[pos..].iter().position(|&b| b == b'\n');
            let (line_end, had_nl) = match nl {
                Some(i) => (pos + i, true),
                None => (input.len(), false),
            };
            let line = &input[pos..line_end];

            let fields = split_fields(line, sep);
            let field_refs: Vec<&[u8]> = fields.iter().map(|s| s.as_slice()).collect();
            run_block(&program.main, line, &field_refs);

            if !had_nl {
                break;
            }
            pos = line_end + 1;
        }
    }

    // ── END block ─────────────────────────────────────────────────────
    run_block(&program.end, b"", &no_fields);

    0
}

// ─────────────────────────────────────────────────────────────────────
// Field splitting
// ─────────────────────────────────────────────────────────────────────

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn split_fields(line: &[u8], sep: Option<u8>) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    match sep {
        Some(s) => {
            // Custom single-byte separator: every separator creates a new
            // field, even adjacent ones (so "a::b" with -F: → ["a","","b"]).
            let mut start = 0usize;
            for (i, &b) in line.iter().enumerate() {
                if b == s {
                    out.push(line[start..i].to_vec());
                    start = i + 1;
                }
            }
            out.push(line[start..].to_vec());
        }
        None => {
            // Default: split on runs of whitespace, trimming leading/trailing.
            let mut i = 0usize;
            while i < line.len() && is_ws(line[i]) {
                i += 1;
            }
            while i < line.len() {
                let start = i;
                while i < line.len() && !is_ws(line[i]) {
                    i += 1;
                }
                out.push(line[start..i].to_vec());
                while i < line.len() && is_ws(line[i]) {
                    i += 1;
                }
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────
// Program model
// ─────────────────────────────────────────────────────────────────────

enum Item<'a> {
    Field(usize),      // $N (0 = whole line)
    Literal(&'a [u8]), // "..."
}

struct PrintStmt<'a> {
    items: Vec<Item<'a>>,
}

struct Program<'a> {
    begin: Vec<PrintStmt<'a>>,
    main: Vec<PrintStmt<'a>>,
    end: Vec<PrintStmt<'a>>,
}

// ─────────────────────────────────────────────────────────────────────
// Parser
// ─────────────────────────────────────────────────────────────────────

fn parse_program(script: &[u8]) -> Result<Program<'_>, &'static [u8]> {
    let mut prog = Program {
        begin: Vec::new(),
        main: Vec::new(),
        end: Vec::new(),
    };
    let mut i = 0usize;
    while i < script.len() {
        i = skip_ws(script, i);
        if i >= script.len() {
            break;
        }
        if starts_with_at(script, i, b"BEGIN") {
            i += b"BEGIN".len();
            i = skip_ws(script, i);
            let (stmts, next) = parse_block(script, i)?;
            prog.begin.extend(stmts);
            i = next;
        } else if starts_with_at(script, i, b"END") {
            i += b"END".len();
            i = skip_ws(script, i);
            let (stmts, next) = parse_block(script, i)?;
            prog.end.extend(stmts);
            i = next;
        } else if script[i] == b'{' {
            let (stmts, next) = parse_block(script, i)?;
            prog.main.extend(stmts);
            i = next;
        } else {
            return Err(b"expected BEGIN, END, or `{`");
        }
    }
    Ok(prog)
}

fn parse_block(script: &[u8], start: usize) -> Result<(Vec<PrintStmt<'_>>, usize), &'static [u8]> {
    let mut i = skip_ws(script, start);
    if i >= script.len() || script[i] != b'{' {
        return Err(b"expected `{`");
    }
    i += 1; // consume '{'
    let mut stmts = Vec::new();
    loop {
        i = skip_ws(script, i);
        if i >= script.len() {
            return Err(b"unclosed `{`");
        }
        if script[i] == b'}' {
            return Ok((stmts, i + 1));
        }
        if !starts_with_at(script, i, b"print") {
            return Err(b"expected `print`");
        }
        i += b"print".len();
        let (stmt, next) = parse_print_args(script, i)?;
        stmts.push(stmt);
        i = next;
        i = skip_ws(script, i);
        if i < script.len() && script[i] == b';' {
            i += 1;
        }
    }
}

fn parse_print_args(script: &[u8], start: usize) -> Result<(PrintStmt<'_>, usize), &'static [u8]> {
    let mut items = Vec::new();
    let mut i = skip_inline_ws(script, start);
    // `print` with no args ⇒ print $0
    if i >= script.len() || script[i] == b';' || script[i] == b'}' || script[i] == b'\n' {
        items.push(Item::Field(0));
        return Ok((PrintStmt { items }, i));
    }
    loop {
        i = skip_inline_ws(script, i);
        if i >= script.len() {
            return Err(b"unterminated print argument");
        }
        match script[i] {
            b'$' => {
                i += 1;
                let (n, next) = parse_uint(script, i)?;
                items.push(Item::Field(n));
                i = next;
            }
            b'"' => {
                i += 1;
                let lit_start = i;
                while i < script.len() && script[i] != b'"' {
                    i += 1;
                }
                if i >= script.len() {
                    return Err(b"unterminated string literal");
                }
                items.push(Item::Literal(&script[lit_start..i]));
                i += 1; // consume closing `"`
            }
            _ => return Err(b"expected `$N` or `\"...\"`"),
        }
        i = skip_inline_ws(script, i);
        if i < script.len() && script[i] == b',' {
            i += 1;
            continue;
        }
        return Ok((PrintStmt { items }, i));
    }
}

fn parse_uint(script: &[u8], start: usize) -> Result<(usize, usize), &'static [u8]> {
    let mut i = start;
    let mut n = 0usize;
    let mut any = false;
    while i < script.len() && script[i].is_ascii_digit() {
        n = n
            .checked_mul(10)
            .and_then(|v| v.checked_add((script[i] - b'0') as usize))
            .ok_or(b"field index overflow" as &[u8])?;
        i += 1;
        any = true;
    }
    if !any {
        return Err(b"expected digit after `$`");
    }
    Ok((n, i))
}

fn skip_ws(script: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < script.len() && (script[i] == b' ' || script[i] == b'\t' || script[i] == b'\n') {
        i += 1;
    }
    i
}

fn skip_inline_ws(script: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < script.len() && (script[i] == b' ' || script[i] == b'\t') {
        i += 1;
    }
    i
}

fn starts_with_at(haystack: &[u8], at: usize, needle: &[u8]) -> bool {
    at + needle.len() <= haystack.len() && &haystack[at..at + needle.len()] == needle
}

// ─────────────────────────────────────────────────────────────────────
// Runtime
// ─────────────────────────────────────────────────────────────────────

fn run_block(stmts: &[PrintStmt<'_>], line: &[u8], fields: &[&[u8]]) {
    for stmt in stmts {
        emit_print(stmt, line, fields);
    }
}

fn emit_print(stmt: &PrintStmt<'_>, line: &[u8], fields: &[&[u8]]) {
    for (idx, item) in stmt.items.iter().enumerate() {
        if idx > 0 {
            let _ = libc_lite::write(1, b" ");
        }
        match item {
            Item::Field(0) => {
                let _ = libc_lite::write(1, line);
            }
            Item::Field(n) => {
                if let Some(f) = fields.get(n - 1) {
                    let _ = libc_lite::write(1, f);
                }
                // Missing field prints as empty string.
            }
            Item::Literal(s) => {
                let _ = libc_lite::write(1, s);
            }
        }
    }
    let _ = libc_lite::write(1, b"\n");
}
