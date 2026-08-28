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
/// Maximum history entries held in memory and written back to the file.
const MAX_HISTORY: usize = 1000;
/// Cap on how much of a history file is read at startup. A truncated read
/// costs the oldest entries, which is preferable to refusing to start.
const HISTORY_READ_LIMIT: usize = 64 * 1024;

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

    /// Iterate entries oldest-first, for the `history` builtin.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|s| s.as_str())
    }

    /// Rebuild history from the contents of a history file.
    ///
    /// Splits on newlines, drops blanks, and keeps only the most recent
    /// MAX_HISTORY lines. Parsing is separate from I/O so it can be tested on
    /// the host, where there are no syscalls.
    pub fn load_from_str(&mut self, contents: &str) {
        self.entries.clear();
        for line in contents.split('\n') {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                self.entries.push(String::from(trimmed));
            }
        }
        if self.entries.len() > MAX_HISTORY {
            let excess = self.entries.len() - MAX_HISTORY;
            self.entries.drain(..excess);
        }
    }

    /// Serialise history back to file form: one entry per line, trailing
    /// newline so appending stays well-formed.
    pub fn to_file_string(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(e);
            out.push('\n');
        }
        out
    }

    /// Read history from `path`. A missing or unreadable file is not an error:
    /// the first session on a fresh system simply starts with none.
    pub fn load_file(&mut self, path: &str) {
        let mut cpath = String::from(path);
        cpath.push('\0');
        let fd = match libc_lite::open(cpath.as_bytes(), 0, 0) {
            Ok(fd) => fd,
            Err(_) => return,
        };

        let mut contents = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match libc_lite::read(fd, &mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    contents.extend_from_slice(&chunk[..n]);
                    if contents.len() >= HISTORY_READ_LIMIT {
                        break;
                    }
                }
            }
        }
        let _ = libc_lite::close(fd);

        if let Ok(text) = core::str::from_utf8(&contents) {
            self.load_from_str(text);
        }
    }

    /// Write history to `path`, replacing whatever was there.
    ///
    /// Rewriting the whole file (rather than appending each line) keeps the
    /// MAX_HISTORY cap honest and needs no seek. Failure is silent: a shell
    /// that cannot save history should still exit cleanly.
    pub fn save_file(&self, path: &str) {
        let mut cpath = String::from(path);
        cpath.push('\0');
        // O_WRONLY (0x0001) | O_CREAT (0x0040) | O_TRUNC (0x0200) = 0x0241.
        let fd = match libc_lite::open(cpath.as_bytes(), 0x0241, 0o644) {
            Ok(fd) => fd,
            Err(_) => return,
        };
        let data = self.to_file_string();
        let mut written = 0usize;
        while written < data.len() {
            match libc_lite::write(fd, &data.as_bytes()[written..]) {
                Ok(0) | Err(_) => break,
                Ok(n) => written += n,
            }
        }
        let _ = libc_lite::close(fd);
    }
}

/// Where to keep the history file.
///
/// `$HOME/.racsh_history` when HOME is set, otherwise `/var/.racsh_history` —
/// `/` is the read-only initramfs, so a fallback there would silently never
/// save. /var survives the session; surviving a *reboot* waits on v0.3, which
/// moves /home onto persistent storage.
pub fn history_path(home: Option<&str>) -> String {
    match home {
        Some(h) if !h.is_empty() && h != "/" => {
            let mut p = String::from(h.trim_end_matches('/'));
            p.push_str("/.racsh_history");
            p
        }
        _ => String::from("/var/.racsh_history"),
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
            // Tab — complete the word under the cursor
            b'\t' => {
                complete_at_cursor(&mut state, prompt);
            }
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
                state.saved_line = String::from(core::str::from_utf8(&state.buf).unwrap_or(""));
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
/// Builtin names offered in command position. Kept here rather than imported
/// from `builtin` so completion stays independent of the dispatch table's
/// shape; the two lists are small and both are exercised by tests.
const COMPLETABLE_BUILTINS: &[&str] = &[
    "alias", "bg", "cd", "exit", "export", "false", "fg", "history", "jobs", "kill", "pwd", "read",
    "set", "source", "test", "true", "type", "unalias", "unset", "wait",
];

/// Tab handler: complete the word under the cursor.
///
/// One match is inserted outright. Several extend the word by their common
/// prefix, and if that adds nothing, the list is printed and the line redrawn
/// beneath it. No matches leaves the line untouched.
fn complete_at_cursor(state: &mut LineState, prompt: &str) {
    let line = match core::str::from_utf8(&state.buf) {
        Ok(s) => String::from(s),
        Err(_) => return,
    };
    let span = crate::complete::word_span(&line, state.cursor);
    let word = &line[span.start..span.end];

    // A word with a slash is a path even in command position, so `./x` and
    // `/bin/l` complete the way they look.
    let matches = if span.command_position && !word.contains('/') {
        let path = "/bin:/sbin";
        crate::complete::command_candidates(word, path, COMPLETABLE_BUILTINS)
    } else {
        crate::complete::path_candidates(word)
    };

    if matches.is_empty() {
        return;
    }

    if matches.len() == 1 {
        let mut replacement = matches[0].clone();
        replacement.push(' ');
        replace_word(state, span.start, span.end, &replacement, prompt);
        return;
    }

    let prefix = crate::complete::common_prefix(&matches);
    if prefix.len() > word.len() {
        replace_word(state, span.start, span.end, &prefix, prompt);
        return;
    }

    // Ambiguous and no further prefix to add: show the options.
    let _ = libc_lite::write(1, b"\n");
    for m in &matches {
        let _ = libc_lite::write(1, m.as_bytes());
        let _ = libc_lite::write(1, b"  ");
    }
    let _ = libc_lite::write(1, b"\n");
    refresh_line(state, prompt);
}

/// Replace bytes [start, end) of the line with `replacement`, leaving the
/// cursor just past it.
fn replace_word(state: &mut LineState, start: usize, end: usize, replacement: &str, prompt: &str) {
    let mut next: Vec<u8> = Vec::with_capacity(state.buf.len() + replacement.len());
    next.extend_from_slice(&state.buf[..start]);
    next.extend_from_slice(replacement.as_bytes());
    next.extend_from_slice(&state.buf[end..]);

    if next.len() >= MAX_LINE {
        return;
    }
    state.buf = next;
    state.cursor = start + replacement.len();
    refresh_line(state, prompt);
}

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
    // Emit the whole sequence in ONE write so the terminal's CSI parser
    // sees a complete ESC [ <n> C without intervening flushes.
    //
    // Layout: "\r\x1B[<col>C"  (with col omitted entirely if 0)
    let mut buf = [0u8; 32];
    buf[0] = b'\r';
    let mut len = 1;
    let col = prompt.len() + state.cursor;
    if col > 0 {
        buf[len] = 0x1B;
        len += 1;
        buf[len] = b'[';
        len += 1;
        let mut num = [0u8; 16];
        let nlen = format_usize(col, &mut num);
        buf[len..len + nlen].copy_from_slice(&num[..nlen]);
        len += nlen;
        buf[len] = b'C';
        len += 1;
    }
    let _ = libc_lite::write(1, &buf[..len]);
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
