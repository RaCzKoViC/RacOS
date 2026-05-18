// racsh — RacOS Interactive Shell binary
//
// REPL: display prompt → read line → lex → parse → execute → repeat
// Uses libc-lite for syscalls and racsh library for parsing + execution.

#![no_std]
#![no_main]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;
use racsh::lexer::Lexer;
use racsh::parser::Parser;
use racsh::expand::Env;
use racsh::exec;
use racsh::readline::{self, History};

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // Initialize environment
    let pid = libc_lite::getpid();
    let mut env = Env::new(pid);

    // Set default PATH
    env.set(String::from("PATH"), String::from("/bin:/sbin"));

    // Set PS1 prompt
    env.set(String::from("PS1"), String::from("racsh$ "));

    // Command history
    let mut history = History::new();

    // Print banner
    let _ = libc_lite::write(1, b"racsh ");
    let _ = libc_lite::write(1, racsh::VERSION.as_bytes());
    let _ = libc_lite::write(1, b"\n");

    // REPL loop
    loop {
        // Read a line with editing support
        let prompt = env.get("PS1").unwrap_or("$ ");
        let line = match readline::readline(prompt, &history) {
            Some(line) => line,
            None => {
                // EOF — exit
                let _ = libc_lite::write(1, b"\nexit\n");
                break;
            }
        };

        // Skip empty lines
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Add to history
        history.push(trimmed);

        // Lex
        let mut lexer = Lexer::new(trimmed);
        let tokens = match lexer.tokenize() {
            Ok(t) => t,
            Err(e) => {
                let _ = libc_lite::write(2, b"racsh: syntax error: ");
                let _ = libc_lite::write(2, e.message.as_bytes());
                let _ = libc_lite::write(2, b"\n");
                env.last_status = 2;
                continue;
            }
        };

        // Parse
        let mut parser = Parser::new(tokens);
        let ast = match parser.parse() {
            Ok(ast) => ast,
            Err(e) => {
                let _ = libc_lite::write(2, b"racsh: parse error: ");
                let _ = libc_lite::write(2, e.message.as_bytes());
                let _ = libc_lite::write(2, b"\n");
                env.last_status = 2;
                continue;
            }
        };

        // Execute
        let status = exec::execute(&ast, &mut env);
        env.last_status = status;
    }

    env.last_status
}
