// RaCore — Virtual Terminal Manager
//
// Manages multiple virtual terminals (VT1-VT6).
// Handles switching between terminals and screen buffering.

#![allow(static_mut_refs)]

use crate::fb_console::{get_console, Color, FramebufferConsole};
use alloc::vec;
use alloc::vec::Vec;

/// Number of virtual terminals (standard 6).
pub const MAX_VT: usize = 6;

/// ESC sequence parsing state for the off-screen VT buffer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EscState {
    Idle,
    EscSeen,
    Csi,
}

/// State of a single virtual terminal.
pub struct VirtualTerminal {
    /// Zero-based ID (0-5).
    pub id: usize,
    /// Screen buffer for this VT (cols * rows characters).
    buffer: Vec<u8>,
    /// Terminal dimensions.
    cols: u32,
    rows: u32,
    /// Cursor X position.
    cursor_x: u32,
    /// Cursor Y position.
    cursor_y: u32,
    /// Foreground color.
    fg: Color,
    /// Background color.
    bg: Color,
    /// Whether this VT is currently active (visible).
    active: bool,
    /// Stateful parser for stripping ANSI CSI sequences across multiple
    /// vt_print() calls (each sys_write may carry a partial sequence).
    esc_state: EscState,
}

impl VirtualTerminal {
    pub fn new(id: usize, cols: u32, rows: u32) -> Self {
        let size = (cols * rows) as usize;
        VirtualTerminal {
            id,
            buffer: vec![b' '; size],
            cols,
            rows,
            cursor_x: 0,
            cursor_y: 0,
            fg: Color::White,
            bg: Color::Black,
            active: false,
            esc_state: EscState::Idle,
        }
    }

    /// Feed a byte stream into the VT buffer with ANSI CSI sequences stripped.
    /// The framebuffer itself is rendered elsewhere (vt_print routes through
    /// fb_console::write_str); this only keeps the off-screen "remembered
    /// screen" buffer free of escape bytes so VT switching shows clean text.
    pub fn consume_stripped(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.esc_state {
                EscState::Idle => {
                    if b == 0x1B {
                        self.esc_state = EscState::EscSeen;
                    } else {
                        self.put_char_local(b);
                    }
                }
                EscState::EscSeen => {
                    if b == b'[' {
                        self.esc_state = EscState::Csi;
                    } else {
                        // Unknown ESC <x> — drop both bytes.
                        self.esc_state = EscState::Idle;
                    }
                }
                EscState::Csi => {
                    // CSI body: keep consuming until the final byte (0x40..=0x7E).
                    if b >= 0x40 && b <= 0x7E {
                        self.esc_state = EscState::Idle;
                    }
                }
            }
        }
    }

    /// Update only the off-screen buffer (no framebuffer side effect).
    fn put_char_local(&mut self, c: u8) {
        match c {
            b'\n' => {
                self.cursor_x = 0;
                self.cursor_y += 1;
            }
            b'\r' => {
                self.cursor_x = 0;
            }
            b'\x08' => {
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            _ => {
                let idx = (self.cursor_y * self.cols + self.cursor_x) as usize;
                if idx < self.buffer.len() {
                    self.buffer[idx] = c;
                    self.cursor_x += 1;
                    if self.cursor_x >= self.cols {
                        self.cursor_x = 0;
                        self.cursor_y += 1;
                    }
                }
            }
        }
        if self.cursor_y >= self.rows {
            self.scroll();
            self.cursor_y = self.rows - 1;
        }
    }

    /// Write character to VT buffer.
    pub fn put_char(&mut self, c: u8) {
        match c {
            b'\n' => {
                self.cursor_x = 0;
                self.cursor_y += 1;
            }
            b'\r' => {
                self.cursor_x = 0;
            }
            b'\x08' => {
                // Backspace
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            _ => {
                let idx = (self.cursor_y * self.cols + self.cursor_x) as usize;
                if idx < self.buffer.len() {
                    self.buffer[idx] = c;
                    self.cursor_x += 1;
                    if self.cursor_x >= self.cols {
                        self.cursor_x = 0;
                        self.cursor_y += 1;
                    }
                }
            }
        }

        if self.cursor_y >= self.rows {
            self.scroll();
            self.cursor_y = self.rows - 1;
        }

        // If this VT is active, we also write to the actual framebuffer
        if self.active {
            if let Some(console) = unsafe { get_console() } {
                console.put_char(c);
            }
        }
    }

    fn scroll(&mut self) {
        let cols = self.cols;
        let rows = self.rows;
        for r in 0..rows - 1 {
            let src = ((r + 1) * cols) as usize;
            let dst = (r * cols) as usize;
            for c in 0..cols {
                self.buffer[dst + c as usize] = self.buffer[src + c as usize];
            }
        }
        let last_row = ((rows - 1) * cols) as usize;
        for c in 0..cols {
            self.buffer[last_row + c as usize] = b' ';
        }
    }

    pub fn clear(&mut self) {
        for ch in self.buffer.iter_mut() {
            *ch = b' ';
        }
        self.cursor_x = 0;
        self.cursor_y = 0;

        if self.active {
            if let Some(console) = unsafe { get_console() } {
                console.clear();
            }
        }
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
        if active {
            self.redraw();
        }
    }

    /// Redraw this VT from its buffer.
    pub fn redraw(&self) {
        if let Some(console) = unsafe { get_console() } {
            console.clear();

            // Temporary: print VT header
            let msg = alloc::format!("--- Virtual Terminal {} ---\n", self.id + 1);
            console.write_str(&msg);

            // Restore from buffer
            // For now, we just dump the buffer content
            // In a better implementation, we'd handle cursor tracking better
            for r in 0..self.rows {
                for c in 0..self.cols {
                    let ch = self.buffer[(r * self.cols + c) as usize];
                    if ch != b' ' {
                        // This is inefficient but demonstrates the buffer persistence
                        console.put_char(ch);
                    }
                }
            }
        }
    }
}

/// Global VT Manager.
pub struct VtManager {
    vts: Vec<VirtualTerminal>,
    current_vt: usize,
}

impl VtManager {
    pub fn new() -> Self {
        let mut vts = Vec::new();
        // Get actual dimensions from the framebuffer console, fall back to 80x25.
        let (cols, rows) = unsafe {
            if let Some(console) = get_console() {
                (console.cols(), console.rows())
            } else {
                (80, 25)
            }
        };
        for i in 0..MAX_VT {
            vts.push(VirtualTerminal::new(i, cols, rows));
        }

        // Set VT1 as active
        vts[0].set_active(true);

        VtManager { vts, current_vt: 0 }
    }

    /// Switch to a specific VT.
    pub fn switch_to(&mut self, id: usize) {
        if id >= MAX_VT || id == self.current_vt {
            return;
        }

        self.vts[self.current_vt].set_active(false);
        self.current_vt = id;
        self.vts[id].set_active(true);

        crate::serial::serial_println!("[  VT  ] Switched to VT{}", id + 1);
    }

    /// Get current VT.
    pub fn current(&mut self) -> &mut VirtualTerminal {
        &mut self.vts[self.current_vt]
    }
}

static mut VT_MANAGER: Option<VtManager> = None;

pub fn init() {
    unsafe {
        VT_MANAGER = Some(VtManager::new());
    }
}

pub unsafe fn get_manager() -> &'static mut VtManager {
    VT_MANAGER.as_mut().expect("VT Manager not initialized")
}

/// Helper to write to current VT.
///
/// Routes the entire string through fb_console::write_str so that ANSI CSI
/// sequences (\x1B[...) are parsed as a unit. Going through put_char one
/// byte at a time, as a previous version did, exposed the ESC bytes to the
/// framebuffer as literal characters whenever a CSI was split across
/// multiple sys_write calls — producing garbage like `[K[14C` after every
/// backspace in racsh.
pub fn vt_print(s: &str) {
    unsafe {
        if let Some(mgr) = VT_MANAGER.as_mut() {
            // Update the off-screen VT buffer with stripped text (skip CSI).
            mgr.current().consume_stripped(s.as_bytes());
        }
        // Render to the actual framebuffer with full CSI handling.
        if let Some(console) = crate::fb_console::get_console() {
            console.write_str(s);
        }
    }
}

pub fn vt_clear_current() {
    unsafe {
        if let Some(mgr) = VT_MANAGER.as_mut() {
            mgr.current().clear();
        }
    }
}
