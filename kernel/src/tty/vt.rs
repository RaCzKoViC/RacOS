// RaCore — Virtual Terminals, rendered from RacTerm's buffer (v0.4 §4.2)
//
// Until v0.4 this file kept six byte-arrays of "remembered screen" with the
// escape sequences stripped out, while the actual pixels came from
// fb_console's ANSI-naive parser. Two parsers, two states, and the remembered
// screen forgot every attribute: switching VTs brought text back with the
// colors gone.
//
// Now each VT owns a real `racterm::Terminal` — the same emulator the
// userland racterm binary uses, 34 host tests and all — and the screen is
// rendered FROM its cell buffer: bytes go in through `Terminal::feed`, dirty
// rows come out through the gfx owner as presented surface strips. One
// parser, one state, and a VT switch redraws exactly what that VT's grid
// holds, attributes included. fb_console remains the pre-heap boot console;
// once `init()` runs, this path owns the console region.
//
// Rendering is a gfx client per ROADMAP §6b: this module never touches the
// framebuffer, it draws rows into a Surface and asks the owner to present
// them.

#![allow(static_mut_refs)]

extern crate alloc;

use alloc::vec::Vec;
use racterm::buffer::{Cell, Color};
use racterm::terminal::Terminal;

use crate::fb_console::FONT;
use crate::gfx;

pub const MAX_VT: usize = 6;

/// One virtual terminal: a RacTerm instance plus its identity.
pub struct VirtualTerminal {
    #[allow(dead_code)]
    id: usize,
    term: Terminal,
}

impl VirtualTerminal {
    fn new(id: usize, cols: usize, rows: usize) -> Self {
        let mut term = Terminal::new(rows, cols);
        // Scrollback costs one row-clone allocation per scrolled line, there
        // is no scroll-up UI in the kernel yet, and the console path runs in
        // whatever context printed. Off.
        term.set_scrollback_limit(0);
        VirtualTerminal { id, term }
    }
}

/// The VT manager: six terminals, one active, and the render state shared
/// between them (row surface, last drawn cursor).
pub struct VtManager {
    vts: Vec<VirtualTerminal>,
    current_vt: usize,
    /// Reusable one-text-row surface; allocating ~80 KiB per keystroke echo
    /// would be silly, so it is built once.
    row_surface: gfx::Surface,
    /// Where the cursor cell was last drawn, so the next render can restore
    /// that cell before drawing the cursor at its new position.
    last_cursor: (usize, usize),
    cell_w: u32,
    cell_h: u32,
}

impl VtManager {
    fn new() -> Option<Self> {
        let (w, h) = gfx::console_region()?;
        let cell_w = 8u32;
        let cell_h = 16u32;
        let cols = (w / cell_w) as usize;
        let rows = (h / cell_h) as usize;

        let mut vts = Vec::new();
        for i in 0..MAX_VT {
            vts.push(VirtualTerminal::new(i, cols, rows));
        }
        Some(VtManager {
            vts,
            current_vt: 0,
            row_surface: gfx::Surface::new(cols as u32 * cell_w, cell_h),
            last_cursor: (0, 0),
            cell_w,
            cell_h,
        })
    }

    /// Feed output bytes to the current VT and repaint what changed.
    pub fn write(&mut self, bytes: &[u8]) {
        // The old cursor cell must repaint even if its row is otherwise
        // clean, or the screen accumulates ghost cursors.
        let (cr, _) = self.last_cursor;
        self.vts[self.current_vt].term.feed(bytes);
        self.render_row_if_valid(cr);
        self.render_dirty();
    }

    /// Switch VTs: full redraw from the target's grid. This is where the
    /// racterm rewrite pays off — the grid still has every attribute, so the
    /// restored screen is the screen, not a monochrome memory of it.
    pub fn switch_to(&mut self, id: usize) {
        if id >= MAX_VT || id == self.current_vt {
            return;
        }
        self.current_vt = id;
        self.vts[id].term.buffer.mark_all_dirty();
        self.render_dirty();
        crate::serial::serial_println!("[  VT  ] Switched to VT{}", id + 1);
    }

    /// Clear the current VT (used by the shell's clear path).
    pub fn clear_current(&mut self) {
        let term = &mut self.vts[self.current_vt].term;
        term.feed(b"\x1b[2J\x1b[H");
        self.render_dirty();
    }

    fn render_row_if_valid(&mut self, row: usize) {
        if row < self.vts[self.current_vt].term.buffer.rows {
            self.render_row(row);
        }
    }

    /// Repaint every dirty row of the active VT, then the cursor.
    fn render_dirty(&mut self) {
        let rows = self.vts[self.current_vt].term.buffer.rows;
        for row in 0..rows {
            if self.vts[self.current_vt].term.buffer.is_dirty(row) {
                self.render_row(row);
                self.vts[self.current_vt].term.buffer.clear_dirty(row);
            }
        }
        self.draw_cursor();
    }

    /// Draw one text row from the grid into the row surface and present it.
    fn render_row(&mut self, row: usize) {
        let term = &self.vts[self.current_vt].term;
        let cols = term.buffer.cols;
        for col in 0..cols {
            let cell = term.buffer.get(row, col);
            let (fg, bg) = cell_colors(cell);
            draw_cell(
                &mut self.row_surface,
                col as u32 * self.cell_w,
                cell,
                fg,
                bg,
            );
        }
        gfx::present(&self.row_surface, 0, row as u32 * self.cell_h);
    }

    /// Paint the cursor as a reversed cell at the terminal's cursor position.
    fn draw_cursor(&mut self) {
        let term = &self.vts[self.current_vt].term;
        if !term.cursor.visible {
            self.last_cursor = (term.cursor.row, term.cursor.col);
            return;
        }
        let (row, col) = (term.cursor.row, term.cursor.col);
        if row >= term.buffer.rows || col >= term.buffer.cols {
            return;
        }
        let cell = *term.buffer.get(row, col);
        let (fg, bg) = cell_colors(&cell);

        let mut cursor_surface = gfx::Surface::new(self.cell_w, self.cell_h);
        // Reversed: the block cursor is the cell with fg/bg swapped.
        draw_cell(&mut cursor_surface, 0, &cell, bg, fg);
        gfx::present(
            &cursor_surface,
            col as u32 * self.cell_w,
            row as u32 * self.cell_h,
        );
        self.last_cursor = (row, col);
    }
}

/// Resolve a cell's colors to native pixels, attributes applied.
fn cell_colors(cell: &Cell) -> (u32, u32) {
    let mut fg = color_to_rgb(cell.fg, true, cell.attrs.bold);
    let mut bg = color_to_rgb(cell.bg, false, false);
    if cell.attrs.reverse {
        core::mem::swap(&mut fg, &mut bg);
    }
    (gfx::encode(fg.0, fg.1, fg.2), gfx::encode(bg.0, bg.1, bg.2))
}

/// Draw one cell's glyph into `surface` at horizontal pixel offset `x`.
fn draw_cell(surface: &mut gfx::Surface, x: u32, cell: &Cell, fg: u32, bg: u32) {
    let ch = cell.character;
    let glyph = if (ch as u32) < 0x80 {
        &FONT[ch as usize]
    } else {
        // The 128-glyph font cannot show it; the hollow box says so
        // honestly, exactly like the pre-racterm console path did.
        &FONT[0x7F]
    };
    for (dy, bits) in glyph.iter().enumerate() {
        for dx in 0..8u32 {
            let on = bits & (0x80 >> dx) != 0;
            surface.put(x + dx, dy as u32, if on { fg } else { bg });
        }
    }
    if cell.attrs.underline {
        for dx in 0..8u32 {
            surface.put(x + dx, 14, fg);
        }
    }
}

/// RacTerm color -> (r, g, b). VGA palette for the 16 base colors, the
/// standard 6x6x6 cube for 16..231, the grayscale ramp for 232..255.
fn color_to_rgb(color: Color, is_fg: bool, bold: bool) -> (u8, u8, u8) {
    const VGA: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0xAA, 0x00, 0x00),
        (0x00, 0xAA, 0x00),
        (0xAA, 0x55, 0x00),
        (0x00, 0x00, 0xAA),
        (0xAA, 0x00, 0xAA),
        (0x00, 0xAA, 0xAA),
        (0xAA, 0xAA, 0xAA),
        (0x55, 0x55, 0x55),
        (0xFF, 0x55, 0x55),
        (0x55, 0xFF, 0x55),
        (0xFF, 0xFF, 0x55),
        (0x55, 0x55, 0xFF),
        (0xFF, 0x55, 0xFF),
        (0x55, 0xFF, 0xFF),
        (0xFF, 0xFF, 0xFF),
    ];
    match color {
        Color::Default => {
            if is_fg {
                // Bold default brightens to white, matching every terminal
                // the shell's prompt was written against.
                if bold {
                    VGA[15]
                } else {
                    VGA[7]
                }
            } else {
                VGA[0]
            }
        }
        Color::Indexed(i) => {
            let i = i as usize;
            if i < 8 && bold {
                VGA[i + 8]
            } else if i < 16 {
                VGA[i]
            } else if i < 232 {
                // 6x6x6 color cube; component levels 0,95,135,175,215,255.
                let i = i - 16;
                let level = |v: usize| -> u8 {
                    if v == 0 {
                        0
                    } else {
                        (55 + v * 40) as u8
                    }
                };
                (level(i / 36), level((i / 6) % 6), level(i % 6))
            } else {
                let v = (8 + (i - 232) * 10) as u8;
                (v, v, v)
            }
        }
        Color::Rgb(r, g, b) => (r, g, b),
    }
}

// ─── Global instance + public API (unchanged surface) ───────────────────────

static mut VT_MANAGER: Option<VtManager> = None;

/// Initialize the VT layer. Requires the heap (Terminals and surfaces are
/// allocations) and the gfx owner (for the console region); with no
/// framebuffer this quietly stays uninitialised and fb_print's fallback
/// keeps routing to the serial-mirroring fb_console.
pub fn init() {
    // SAFETY: VT_MANAGER is set once at boot, single-threaded.
    unsafe {
        VT_MANAGER = VtManager::new();
        if let Some(mgr) = VT_MANAGER.as_mut() {
            // Hand over from the boot console: start VT1 from a clean grid
            // rather than racterm-parsing on top of pixels fb_console drew.
            mgr.clear_current();
            crate::serial::serial_println!(
                "[  VT  ] {} terminals, rendered from RacTerm buffers",
                MAX_VT
            );
        }
    }
}

pub fn is_active() -> bool {
    // SAFETY: read-only after init.
    unsafe { VT_MANAGER.is_some() }
}

pub unsafe fn get_manager() -> &'static mut VtManager {
    VT_MANAGER.as_mut().expect("VT Manager not initialized")
}

/// Write console output to the current VT.
pub fn vt_print(s: &str) {
    // SAFETY: VT_MANAGER is a boot-once singleton; single-CPU MVP.
    unsafe {
        if let Some(mgr) = VT_MANAGER.as_mut() {
            mgr.write(s.as_bytes());
        } else if let Some(console) = crate::fb_console::get_console() {
            // Pre-init fallback: the boot console.
            console.write_str(s);
        }
    }
}

pub fn vt_clear_current() {
    // SAFETY: VT_MANAGER is a boot-once singleton; single-CPU MVP.
    unsafe {
        if let Some(mgr) = VT_MANAGER.as_mut() {
            mgr.clear_current();
        }
    }
}
