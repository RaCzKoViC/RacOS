#![no_std]
#![allow(dead_code, unused_imports)]
/// RacTerm — RacOS Terminal Emulator (ADR-016, TERMINAL_PROTOCOLS.md)
///
/// Layers: Input → Escape Parser → Screen Buffer + Cursor → Renderer.
extern crate alloc;

pub mod buffer;
pub mod cursor;
pub mod escape;
pub mod input;
pub mod terminal;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
