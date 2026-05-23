// Kernel PTY subsystem (ADR-014)
//
// Pseudo-terminal master/slave pairs.
// Master is held by terminal emulator, slave by shell.

pub mod line_discipline;
pub mod pty;
pub mod tty;
pub mod vt;
