// racsh — RacOS Shell binary
//
// Modes:
//   sh                  → interactive REPL (read line → lex → parse → execute)
//   sh -c COMMAND [n a*] → run COMMAND string; n becomes $0, a* become $1..
//   sh FILE [args...]   → run script FILE; FILE becomes $0, args become $1..
//
// Uses libc-lite for syscalls and the racsh library for parsing + execution.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;
use alloc::vec::Vec;
use racsh::exec;
use racsh::expand::Env;
use racsh::lexer::Lexer;
use racsh::parser::Parser;
use racsh::readline::{self, History};

/// Decode argv bytes into an owned String (lossy: invalid UTF-8 → empty).
fn to_string(bytes: &[u8]) -> String {
    match core::str::from_utf8(bytes) {
        Ok(s) => String::from(s),
        Err(_) => String::new(),
    }
}

/// Lex, parse and execute a whole source string (one or many statements).
/// Returns the resulting exit status and stores it in `env.last_status`.
fn run_source(src: &str, env: &mut Env) -> i32 {
    let mut lexer = Lexer::new(src);
    let tokens = match lexer.tokenize() {
        Ok(t) => t,
        Err(e) => {
            let _ = libc_lite::write(2, b"racsh: syntax error: ");
            let _ = libc_lite::write(2, e.message.as_bytes());
            let _ = libc_lite::write(2, b"\n");
            env.last_status = 2;
            return 2;
        }
    };
    let mut parser = Parser::new(tokens);
    let ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            let _ = libc_lite::write(2, b"racsh: parse error: ");
            let _ = libc_lite::write(2, e.message.as_bytes());
            let _ = libc_lite::write(2, b"\n");
            env.last_status = 2;
            return 2;
        }
    };
    let status = exec::execute(&ast, env);
    env.last_status = status;
    status
}

/// Read a whole file into memory. Returns None if it cannot be opened/read.
fn read_file(path: &[u8]) -> Option<Vec<u8>> {
    let mut nul_path = Vec::with_capacity(path.len() + 1);
    nul_path.extend_from_slice(path);
    nul_path.push(0);

    let fd = libc_lite::open(&nul_path, 0, 0).ok()?; // O_RDONLY
    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&buf[..n]),
            Err(_) => {
                let _ = libc_lite::close(fd);
                return None;
            }
        }
    }
    let _ = libc_lite::close(fd);
    Some(data)
}

/// Collect positional parameters from argv[start..].
fn collect_positional(argv: *const *const u8, start: usize) -> Vec<String> {
    let mut params = Vec::new();
    let mut i = start;
    while let Some(a) = libc_lite::arg(argv, i) {
        params.push(to_string(a));
        i += 1;
    }
    params
}

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let pid = libc_lite::getpid();
    let mut env = Env::new(pid);
    env.set(String::from("PATH"), String::from("/bin:/sbin"));
    env.set(String::from("PS1"), String::from("racsh$ "));

    let argn = libc_lite::arg_count(argc);

    // sh -c COMMAND [command_name [args...]]
    if argn >= 2 && libc_lite::arg(argv, 1) == Some(&b"-c"[..]) {
        let cmd = match libc_lite::arg(argv, 2) {
            Some(c) => c,
            None => {
                let _ = libc_lite::write(2, b"sh: -c: option requires an argument\n");
                return 2;
            }
        };
        // POSIX: the operand after COMMAND is $0, the rest are $1, $2, ...
        if let Some(name) = libc_lite::arg(argv, 3) {
            env.arg0 = to_string(name);
            env.positional = collect_positional(argv, 4);
        }
        let src = to_string(cmd);
        return run_source(&src, &mut env);
    }

    // sh FILE [args...] — run a script file.
    if argn >= 2 {
        let file = match libc_lite::arg(argv, 1) {
            Some(f) => f,
            None => return 0,
        };
        env.arg0 = to_string(file);
        env.positional = collect_positional(argv, 2);
        match read_file(file) {
            Some(data) => match core::str::from_utf8(&data) {
                Ok(src) => return run_source(src, &mut env),
                Err(_) => {
                    let _ = libc_lite::write(2, b"sh: script is not valid UTF-8\n");
                    return 1;
                }
            },
            None => {
                let _ = libc_lite::write(2, b"sh: cannot open script: ");
                let _ = libc_lite::write(2, file);
                let _ = libc_lite::write(2, b"\n");
                return 127;
            }
        }
    }

    // No operands → interactive REPL.
    let mut history = History::new();

    let _ = libc_lite::write(1, b"racsh ");
    let _ = libc_lite::write(1, racsh::VERSION.as_bytes());
    let _ = libc_lite::write(1, b"\n");

    loop {
        let prompt = env.get("PS1").unwrap_or("$ ");
        let line = match readline::readline(prompt, &history) {
            Some(line) => line,
            None => {
                let _ = libc_lite::write(1, b"\nexit\n");
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        history.push(trimmed);
        run_source(trimmed, &mut env);
    }

    env.last_status
}
