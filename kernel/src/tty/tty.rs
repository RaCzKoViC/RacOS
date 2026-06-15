// TTY device (ADR-014)
//
// Generic TTY abstraction over PTY pairs and hardware console.
// Handles association with sessions/process groups.

extern crate alloc;

use super::line_discipline::{LineDiscipline, LineMode, WinSize};
use alloc::string::String;

/// A TTY device representing one terminal session.
pub struct Tty {
    /// TTY index (e.g., pts/0 → index 0).
    pub index: u32,
    /// Name (e.g., "pts/0", "console").
    pub name: String,
    /// Line discipline for this TTY.
    pub ldisc: LineDiscipline,
    /// Terminal dimensions.
    pub winsize: WinSize,
    /// Session ID owning this TTY (-1 if none).
    pub session_id: i32,
    /// Foreground process group ID (-1 if none).
    pub foreground_pgid: i32,
}

impl Tty {
    pub fn new(index: u32, name: &str) -> Self {
        Tty {
            index,
            name: String::from(name),
            ldisc: LineDiscipline::new(),
            winsize: WinSize::default(),
            session_id: -1,
            foreground_pgid: -1,
        }
    }

    pub fn set_winsize(&mut self, rows: u16, cols: u16) {
        let changed = self.winsize.rows != rows || self.winsize.cols != cols;
        self.winsize = WinSize { rows, cols };
        if changed && self.foreground_pgid > 0 {
            unsafe {
                core::arch::asm!("cli", options(nomem, nostack));
                crate::task::scheduler::send_signal_to_group(
                    self.foreground_pgid as u32,
                    crate::task::signal::Signal::SIGWINCH,
                );
                core::arch::asm!("sti", options(nomem, nostack));
            }
        }
    }

    pub fn set_mode(&mut self, mode: LineMode) {
        self.ldisc.set_mode(mode);
    }

    pub fn set_session(&mut self, session_id: i32) {
        self.session_id = session_id;
    }

    pub fn set_foreground(&mut self, pgid: i32) {
        self.foreground_pgid = pgid;
    }
}
