// RacTerm — Escape Sequence Parser (TERMINAL_PROTOCOLS.md §5)
//
// State machine: Ground → Escape → CSI/OSC → Dispatch.
// Handles partial sequences with buffering.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Parser state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    CsiEntry,
    CsiParam,
    OscString,
}

/// A parsed action from the escape sequence parser.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Print a character to screen.
    Print(char),
    /// Execute a C0 control (CR, LF, BS, TAB, BEL, etc).
    Execute(u8),
    /// CSI sequence dispatched.
    CsiDispatch {
        params: Vec<u16>,
        intermediates: Vec<u8>,
        final_byte: u8,
        private: bool,
    },
    /// OSC string dispatched.
    OscDispatch(String),
    /// Simple escape (ESC + final byte, no CSI/OSC).
    EscDispatch(u8),
}

/// The VT escape sequence parser.
pub struct EscParser {
    state: State,
    /// CSI parameter bytes being collected.
    params_buf: Vec<u8>,
    /// CSI intermediate bytes.
    intermediates: Vec<u8>,
    /// Whether this CSI is a private sequence (started with ?).
    private: bool,
    /// OSC string being collected.
    osc_buf: Vec<u8>,
}

impl EscParser {
    pub fn new() -> Self {
        EscParser {
            state: State::Ground,
            params_buf: Vec::new(),
            intermediates: Vec::new(),
            private: false,
            osc_buf: Vec::new(),
        }
    }

    /// Feed one byte, return any generated action.
    pub fn feed(&mut self, byte: u8) -> Option<Action> {
        match self.state {
            State::Ground => self.ground(byte),
            State::Escape => self.escape(byte),
            State::CsiEntry => self.csi_entry(byte),
            State::CsiParam => self.csi_param(byte),
            State::OscString => self.osc_string(byte),
        }
    }

    /// Feed a slice of bytes, collecting all actions.
    pub fn feed_bytes(&mut self, data: &[u8]) -> Vec<Action> {
        let mut actions = Vec::new();
        for &b in data {
            if let Some(a) = self.feed(b) {
                actions.push(a);
            }
        }
        actions
    }

    /// Reset parser to ground state.
    pub fn reset(&mut self) {
        self.state = State::Ground;
        self.params_buf.clear();
        self.intermediates.clear();
        self.private = false;
        self.osc_buf.clear();
    }

    // ── State handlers ──

    fn ground(&mut self, byte: u8) -> Option<Action> {
        match byte {
            0x1B => {
                self.state = State::Escape;
                None
            }
            // C0 controls
            0x00..=0x1A | 0x1C..=0x1F => Some(Action::Execute(byte)),
            // DEL
            0x7F => None,
            // Printable
            _ => {
                // Decode as ASCII for now; UTF-8 handling would need a
                // multi-byte decoder wrapping this parser.
                Some(Action::Print(byte as char))
            }
        }
    }

    fn escape(&mut self, byte: u8) -> Option<Action> {
        match byte {
            b'[' => {
                self.state = State::CsiEntry;
                self.params_buf.clear();
                self.intermediates.clear();
                self.private = false;
                None
            }
            b']' => {
                self.state = State::OscString;
                self.osc_buf.clear();
                None
            }
            // Simple escape sequences
            b'c' | b'7' | b'8' | b'D' | b'E' | b'M' => {
                self.state = State::Ground;
                Some(Action::EscDispatch(byte))
            }
            0x1B => {
                // Double ESC — stay in escape state
                None
            }
            _ => {
                // Unknown escape — dispatch and return to ground
                self.state = State::Ground;
                Some(Action::EscDispatch(byte))
            }
        }
    }

    fn csi_entry(&mut self, byte: u8) -> Option<Action> {
        match byte {
            b'?' => {
                self.private = true;
                self.state = State::CsiParam;
                None
            }
            b'0'..=b'9' | b';' => {
                self.params_buf.push(byte);
                self.state = State::CsiParam;
                None
            }
            // Final byte (0x40-0x7E)
            0x40..=0x7E => {
                self.state = State::Ground;
                Some(self.dispatch_csi(byte))
            }
            // Intermediate bytes
            0x20..=0x2F => {
                self.intermediates.push(byte);
                self.state = State::CsiParam;
                None
            }
            0x1B => {
                // ESC within CSI — abort current, start new escape
                self.state = State::Escape;
                None
            }
            _ => {
                self.state = State::Ground;
                None
            }
        }
    }

    fn csi_param(&mut self, byte: u8) -> Option<Action> {
        match byte {
            b'0'..=b'9' | b';' => {
                if self.params_buf.len() < 64 {
                    self.params_buf.push(byte);
                }
                None
            }
            0x20..=0x2F => {
                self.intermediates.push(byte);
                None
            }
            // Final byte
            0x40..=0x7E => {
                self.state = State::Ground;
                Some(self.dispatch_csi(byte))
            }
            0x1B => {
                self.state = State::Escape;
                None
            }
            _ => {
                self.state = State::Ground;
                None
            }
        }
    }

    fn osc_string(&mut self, byte: u8) -> Option<Action> {
        match byte {
            // BEL terminates OSC
            0x07 => {
                self.state = State::Ground;
                let s = core::str::from_utf8(&self.osc_buf).unwrap_or("").into();
                Some(Action::OscDispatch(s))
            }
            // ESC might start ST (ESC \)
            0x1B => {
                // We'll optimistically treat any ESC as OSC terminator
                // A proper implementation would check for backslash next
                self.state = State::Ground;
                let s = core::str::from_utf8(&self.osc_buf).unwrap_or("").into();
                Some(Action::OscDispatch(s))
            }
            _ => {
                if self.osc_buf.len() < 256 {
                    self.osc_buf.push(byte);
                }
                None
            }
        }
    }

    /// Parse CSI parameter string "1;2;3" → [1, 2, 3].
    fn parse_params(&self) -> Vec<u16> {
        if self.params_buf.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut current: u16 = 0;
        let mut has_current = false;

        for &b in &self.params_buf {
            if b == b';' {
                result.push(if has_current { current } else { 0 });
                current = 0;
                has_current = false;
            } else if b >= b'0' && b <= b'9' {
                current = current.saturating_mul(10).saturating_add((b - b'0') as u16);
                has_current = true;
            }
        }
        result.push(if has_current { current } else { 0 });
        result
    }

    fn dispatch_csi(&self, final_byte: u8) -> Action {
        Action::CsiDispatch {
            params: self.parse_params(),
            intermediates: self.intermediates.clone(),
            final_byte,
            private: self.private,
        }
    }
}
