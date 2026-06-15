// PTY subsystem (ADR-014)
//
// Pseudo-terminal master/slave pairs.
// ptmx allocates pairs; /dev/pts/N are the slave endpoints.
//
// Data flow:
//   Terminal (master write) → line discipline → slave read (shell)
//   Shell (slave write) → master read (terminal renders)

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::line_discipline::{LdiscOutput, LineDiscipline, TtySignal, WinSize};

static NEXT_PTY_INDEX: AtomicU32 = AtomicU32::new(0);

/// Maximum bytes buffered in a PTY direction.
const PTY_BUF_SIZE: usize = 4096;

/// One end of a PTY pair.
pub struct PtyMaster {
    pub index: u32,
    /// Data written by the terminal → goes through ldisc → slave reads.
    /// This buffer holds data *after* line discipline for the slave to read.
    slave_buf: VecDeque<u8>,
    /// Data written by shell (slave) → terminal reads from here.
    master_buf: VecDeque<u8>,
    /// Line discipline.
    ldisc: LineDiscipline,
    /// Terminal size.
    winsize: WinSize,
    /// Foreground process group ID (signals are sent to this group).
    pub foreground_pgid: i32,
}

pub struct PtySlave {
    pub index: u32,
}

/// Create a new PTY master/slave pair.
pub fn alloc_pty() -> (PtyMaster, PtySlave) {
    let idx = NEXT_PTY_INDEX.fetch_add(1, Ordering::Relaxed);
    let master = PtyMaster {
        index: idx,
        slave_buf: VecDeque::with_capacity(PTY_BUF_SIZE),
        master_buf: VecDeque::with_capacity(PTY_BUF_SIZE),
        ldisc: LineDiscipline::new(),
        winsize: WinSize::default(),
        foreground_pgid: -1,
    };
    let slave = PtySlave { index: idx };
    (master, slave)
}

impl PtyMaster {
    /// Terminal writes input to the master side.
    /// Bytes go through the line discipline.
    /// Returns echo bytes that should be sent back to the terminal.
    pub fn write_input(&mut self, data: &[u8]) -> Vec<u8> {
        let mut echo = Vec::new();
        for &byte in data {
            match self.ldisc.process_input(byte) {
                LdiscOutput::Echo(b) => {
                    echo.push(b);
                }
                LdiscOutput::EchoSeq(seq, len) => {
                    for i in 0..len as usize {
                        echo.push(seq[i]);
                    }
                }
                LdiscOutput::LineReady => {
                    // Transfer cooked data to slave buffer
                    self.flush_ldisc_to_slave();
                }
                LdiscOutput::Passthrough(b) => {
                    if self.slave_buf.len() < PTY_BUF_SIZE {
                        self.slave_buf.push_back(b);
                    }
                }
                LdiscOutput::Signal(sig) => {
                    // Deliver signal to the foreground process group.
                    self.deliver_tty_signal(sig);
                    self.flush_ldisc_to_slave();
                }
                LdiscOutput::None => {}
            }
        }
        // After processing, check if ldisc has queued data (canonical mode EOF case)
        self.flush_ldisc_to_slave();
        echo
    }

    /// Read output from the shell (slave wrote it).
    pub fn read_output(&mut self, buf: &mut [u8]) -> usize {
        let mut count = 0;
        while count < buf.len() {
            if let Some(b) = self.master_buf.pop_front() {
                buf[count] = b;
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    /// Check if the terminal has output to read.
    pub fn has_output(&self) -> bool {
        !self.master_buf.is_empty()
    }

    /// Set terminal size and (eventually) deliver SIGWINCH.
    pub fn set_winsize(&mut self, rows: u16, cols: u16) {
        self.winsize = WinSize { rows, cols };
        // Deliver SIGWINCH to slave's foreground process group
        if self.foreground_pgid > 0 {
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

    pub fn winsize(&self) -> WinSize {
        self.winsize
    }

    pub fn ldisc_mut(&mut self) -> &mut LineDiscipline {
        &mut self.ldisc
    }

    /// Set the foreground process group for signal delivery.
    pub fn set_foreground(&mut self, pgid: i32) {
        self.foreground_pgid = pgid;
    }

    /// Deliver a TTY signal to the foreground process group.
    fn deliver_tty_signal(&self, sig: TtySignal) {
        use crate::task::signal::Signal;
        let signal = match sig {
            TtySignal::Interrupt => Signal::SIGINT,
            TtySignal::Quit => Signal::SIGQUIT,
            TtySignal::Suspend => Signal::SIGTSTP,
            TtySignal::Eof => return, // EOF is not a signal
        };
        if self.foreground_pgid > 0 {
            unsafe {
                core::arch::asm!("cli", options(nomem, nostack));
                crate::task::scheduler::send_signal_to_group(self.foreground_pgid as u32, signal);
                core::arch::asm!("sti", options(nomem, nostack));
            }
        }
    }

    fn flush_ldisc_to_slave(&mut self) {
        let mut tmp = [0u8; 256];
        loop {
            let n = self.ldisc.read(&mut tmp);
            if n == 0 {
                break;
            }
            for &b in &tmp[..n] {
                if self.slave_buf.len() < PTY_BUF_SIZE {
                    self.slave_buf.push_back(b);
                }
            }
        }
    }

    // --- Methods used by the slave side (internal) ---

    /// Slave reads from the PTY (gets data that went through ldisc).
    pub fn slave_read(&mut self, buf: &mut [u8]) -> usize {
        let mut count = 0;
        while count < buf.len() {
            if let Some(b) = self.slave_buf.pop_front() {
                buf[count] = b;
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    /// Slave writes to the PTY (output goes to master for rendering).
    pub fn slave_write(&mut self, data: &[u8]) -> usize {
        let mut count = 0;
        for &b in data {
            if self.master_buf.len() < PTY_BUF_SIZE {
                self.master_buf.push_back(b);
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    /// Check if the slave has data to read.
    pub fn slave_has_data(&self) -> bool {
        !self.slave_buf.is_empty()
    }
}
