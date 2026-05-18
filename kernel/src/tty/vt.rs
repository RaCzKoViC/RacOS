// RaCore — Virtual Terminal Manager
//
// Manages multiple virtual terminals (VT1-VT6).
// Handles switching between terminals and screen buffering.

#![allow(static_mut_refs)]

use alloc::vec::Vec;
use alloc::vec;
use crate::fb_console::{Color, FramebufferConsole, get_console};

/// Number of virtual terminals (standard 6).
pub const MAX_VT: usize = 6;

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
            b'\x08' => { // Backspace
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

        VtManager {
            vts,
            current_vt: 0,
        }
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
pub fn vt_print(s: &str) {
    unsafe {
        if let Some(mgr) = VT_MANAGER.as_mut() {
            for &b in s.as_bytes() {
                mgr.current().put_char(b);
            }
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