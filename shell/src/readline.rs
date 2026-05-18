// racsh — Readline-lite: interactive line editor
//
// Features:
// - Left/Right arrow cursor movement
// - Home/End jump to start/end of line
// - Backspace / Delete
// - Command history with Up/Down arrows (last 64 entries)
// - Ctrl-A (home), Ctrl-E (end), Ctrl-U (kill line), Ctrl-K (kill to end)
// - Ctrl-L (clear screen)
// - Ctrl-W (delete word backward)
// - Ctrl-D (EOF if line is empty)

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Maximum line length.
const MAX_LINE: usize = 1024;
/// Maximum history entries.
const MAX_HISTORY: usize = 64;

/// Command history ring buffer.
pub struct History {
    entries: Vec<String>,
}

impl History {
    pub fn new() -> Self {
        History {
            entries: Vec::new(),
        }
    }

    /// Push a line to history (skip empty/duplicate of last).
    pub fn push(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        // Don't duplicate last entry
        if let Some(last) = self.entries.last() {
            if last.as_str() == trimmed {
                return;
            }
        }
        if self.entries.len() >= MAX_HISTORY {
            self.entries.remove(0);
        }
        self.entries.push(String::from(trimmed));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, idx: usize) -> Option<&str> {
        self.entries.get(idx).map(|s| s.as_str())
    }
}

/// State for the line editor during one readline call.
struct LineState {
    /// The current line buffer (UTF-8 bytes).
    buf: Vec<u8>,
    /// Cursor position (byte offset).
    cursor: usize,
    /// History browsing index (counts from end: 0 = current, 1 = last entry, etc.)
    hist_idx: usize,
    /// Saved current line when browsing history.
    saved_line: String,
    /// Prompt length (for redraws).
    prompt_len: usize,
}

impl LineState {
    fn new(prompt_len: usize) -> Self {
        LineState {
            buf: Vec::new(),
            cursor: 0,
            hist_idx: 0,
            saved_line: String::new(),
            prompt_len,
        }
    }

    fn len(&self) -> usize {
        self.buf.len()
    }
}

/// Read a line with editing support.
/// Returns None on EOF (Ctrl-D on empty line).
pub fn readline(prompt: &str, history: &History) -> Option<String> {
    // Print prompt
    let _ = libc_lite::write(1, prompt.as_bytes());

    let mut state = LineState::new(prompt.len());

    loop {
        let b = match read_byte() {
            Some(b) => b,
            None => {
                if state.len() == 0 {
                    return None; // EOF
                }
                break;
            }
        };

        match b {
            // Enter
            b'\r' | b'\n' => {
                let _ = libc_lite::write(1, b"\n");
                break;
            }
            // Ctrl-D — EOF on empty line, delete char otherwise
            0x04 => {
                if state.len() == 0 {
                    return None;
                }
                // Delete char at cursor
                if state.cursor < state.len() {
                    state.buf.remove(state.cursor);
                    refresh_line(&state, prompt);
                }
            }
            // Ctrl-A — home
            0x01 => {
                state.cursor = 0;
                refresh_cursor(&state, prompt);
            }
            // Ctrl-E — end
            0x05 => {
                state.cursor = state.len();
                refresh_cursor(&state, prompt);
            }
            // Ctrl-U — kill whole line
            0x15 => {
                state.buf.clear();
                state.cursor = 0;
                refresh_line(&state, prompt);
            }
            // Ctrl-K — kill from cursor to end
            0x0B => {
                state.buf.truncate(state.cursor);
                refresh_line(&state, prompt);
            }
            // Ctrl-W — delete word backward
            0x17 => {
                if state.cursor > 0 {
                    let mut i = state.cursor;
                    // Skip trailing spaces
                    while i > 0 && state.buf[i - 1] == b' ' {
                        i -= 1;
                    }
                    // Skip word chars
                    while i > 0 && state.buf[i - 1] != b' ' {
                        i -= 1;
                    }
                    state.buf.drain(i..state.cursor);
                    state.cursor = i;
                    refresh_line(&state, prompt);
                }
            }
            // Ctrl-L — clear screen
            0x0C => {
                // ANSI: clear screen + move cursor home
                let _ = libc_lite::write(1, b"\x1B[2J\x1B[H");
                let _ = libc_lite::write(1, prompt.as_bytes());
                refresh_line(&state, prompt);
            }
            // Backspace (0x08 or 0x7F)
            0x08 | 0x7F => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                    state.buf.remove(state.cursor);
                    refresh_line(&state, prompt);
                }
            }
            // Escape sequence
            0x1B => {
                handle_escape(&mut state, history, prompt);
            }
            // Tab — ignore for now (future: completion)
            b'\t' => {}
            // Printable characters
            0x20..=0x7E => {
                if state.len() < MAX_LINE - 1 {
                    state.buf.insert(state.cursor, b);
                    state.cursor += 1;
                    if state.cursor == state.len() {
                        // Append at end — just echo the char
                        let _ = libc_lite::write(1, &[b]);
                    } else {
                        // Inserted in middle — redraw
                        refresh_line(&state, prompt);
                    }
                }
            }
            _ => {
                // Ignore other control characters
            }
        }
    }

    let s = core::str::from_utf8(&state.buf).unwrap_or("");
    Some(String::from(s))
}

/// Handle ESC [ <code> sequences (arrow keys, home, end, delete).
fn handle_escape(state: &mut LineState, history: &History, prompt: &str) {
    let b2 = match read_byte() {
        Some(b) => b,
        None => return,
    };
    if b2 != b'[' {
        return; // Not a CSI sequence
    }
    let b3 = match read_byte() {
        Some(b) => b,
        None => return,
    };
    match b3 {
        // Up arrow — history previous
        b'A' => {
            if history.len() == 0 {
                return;
            }
            if state.hist_idx == 0 {
                // Save current line before browsing
                state.saved_line = String::from(
                    core::str::from_utf8(&state.buf).unwrap_or("")
                );
            }
            if state.hist_idx < history.len() {
                state.hist_idx += 1;
                let idx = history.len() - state.hist_idx;
                if let Some(entry) = history.get(idx) {
                    replace_line(state, entry, prompt);
                }
            }
        }
        // Down arrow — history next
        b'B' => {
            if state.hist_idx > 0 {
                state.hist_idx -= 1;
                if state.hist_idx == 0 {
                    // Restore saved line
                    let saved = state.saved_line.clone();
                    replace_line(state, &saved, prompt);
                } else {
                    let idx = history.len() - state.hist_idx;
                    if let Some(entry) = history.get(idx) {
                        replace_line(state, entry, prompt);
                    }
                }
            }
        }
        // Right arrow
        b'C' => {
            if state.cursor < state.len() {
                state.cursor += 1;
                let _ = libc_lite::write(1, b"\x1B[C");
            }
        }
        // Left arrow
        b'D' => {
            if state.cursor > 0 {
                state.cursor -= 1;
                let _ = libc_lite::write(1, b"\x1B[D");
            }
        }
        // Home
        b'H' => {
            state.cursor = 0;
            refresh_cursor(state, prompt);
        }
        // End
        b'F' => {
            state.cursor = state.len();
            refresh_cursor(state, prompt);
        }
        // Delete key — ESC [ 3 ~
        b'3' => {
            if let Some(b'~') = read_byte() {
                if state.cursor < state.len() {
                    state.buf.remove(state.cursor);
                    refresh_line(state, prompt);
                }
            }
        }
        _ => {}
    }
}

/// Replace the current line buffer with a new string and redraw.
fn replace_line(state: &mut LineState, new: &str, prompt: &str) {
    state.buf.clear();
    state.buf.extend_from_slice(new.as_bytes());
    state.cursor = state.buf.len();
    refresh_line(state, prompt);
}

/// Redraw the line from the prompt onward.
fn refresh_line(state: &LineState, prompt: &str) {
    // Move cursor to start of line (after prompt)
    // \r → beginning of line, then print prompt + buffer + clear to EOL
    let _ = libc_lite::write(1, b"\r");
    let _ = libc_lite::write(1, prompt.as_bytes());
    let _ = libc_lite::write(1, &state.buf);
    // Clear from cursor to end of line
    let _ = libc_lite::write(1, b"\x1B[K");
    // Reposition cursor
    refresh_cursor(state, prompt);
}

/// Move terminal cursor to the correct position.
fn refresh_cursor(state: &LineState, prompt: &str) {
    // Move to absolute column: \r then move right (prompt_len + cursor) columns
    let _ = libc_lite::write(1, b"\r");
    let col = prompt.len() + state.cursor;
    if col > 0 {
        // ESC [ <n> C — move cursor right n columns
        let mut num_buf = [0u8; 16];
        let n = format_usize(col, &mut num_buf);
        let _ = libc_lite::write(1, b"\x1B[");
        let _ = libc_lite::write(1, &num_buf[..n]);
        let _ = libc_lite::write(1, b"C");
    }
}

/// Read one byte from stdin. Returns None on EOF/error.
fn read_byte() -> Option<u8> {
    let mut b = [0u8; 1];
    match libc_lite::read(0, &mut b) {
        Ok(1) => Some(b[0]),
        _ => None,
    }
}

/// Format usize into a decimal string in a fixed buffer. Returns length.
fn format_usize(mut val: usize, buf: &mut [u8; 16]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut pos = 16;
    while val > 0 && pos > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    let len = 16 - pos;
    // Shift to beginning
    buf.copy_within(pos..16, 0);
    len
}
