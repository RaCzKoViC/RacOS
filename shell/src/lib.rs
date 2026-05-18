#![no_std]
#![allow(dead_code)]
/// racsh — RacOS Shell (ADR-015, SHELL_GRAMMAR.md)
///
/// Provides lexer, parser, AST, and expansion modules.
/// The shell can operate in both interactive and script mode.

extern crate alloc;

pub mod token;
pub mod lexer;
pub mod ast;
pub mod parser;
pub mod expand;
pub mod builtin;
pub mod exec;
pub mod readline;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
