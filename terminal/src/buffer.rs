// RacTerm — Screen Buffer (TERMINAL_PROTOCOLS.md §3)
//
// Pure data structure: cell grid + dirty flags.
// No rendering logic.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

/// Color for a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Cell text attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellAttrs {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub reverse: bool,
    pub strikethrough: bool,
}

impl Default for CellAttrs {
    fn default() -> Self {
        CellAttrs {
            bold: false,
            italic: false,
            underline: false,
            blink: false,
            reverse: false,
            strikethrough: false,
        }
    }
}

/// A single character cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub character: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: CellAttrs,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            character: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::default(),
        }
    }
}

/// The screen buffer: primary + alternate + scrollback.
pub struct ScreenBuffer {
    pub rows: usize,
    pub cols: usize,
    /// Primary screen grid.
    primary: Vec<Cell>,
    /// Alternate screen grid (for fullscreen apps).
    alternate: Vec<Cell>,
    /// Which buffer is active.
    using_alternate: bool,
    /// Scrollback buffer (ring of rows).
    scrollback: Vec<Vec<Cell>>,
    /// Maximum scrollback lines.
    scrollback_limit: usize,
    /// Per-row dirty flags for the active buffer.
    dirty: Vec<bool>,
}

impl ScreenBuffer {
    pub fn new(rows: usize, cols: usize) -> Self {
        let size = rows * cols;
        ScreenBuffer {
            rows,
            cols,
            primary: vec![Cell::default(); size],
            alternate: vec![Cell::default(); size],
            using_alternate: false,
            scrollback: Vec::new(),
            scrollback_limit: 10_000,
            dirty: vec![true; rows],
        }
    }

    /// Get the active grid.
    fn grid(&self) -> &[Cell] {
        if self.using_alternate {
            &self.alternate
        } else {
            &self.primary
        }
    }

    /// Get the active grid mutably.
    fn grid_mut(&mut self) -> &mut Vec<Cell> {
        if self.using_alternate {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    /// Get a cell at (row, col).
    pub fn get(&self, row: usize, col: usize) -> &Cell {
        &self.grid()[row * self.cols + col]
    }

    /// Set a cell at (row, col).
    pub fn set(&mut self, row: usize, col: usize, cell: Cell) {
        let idx = row * self.cols + col;
        self.grid_mut()[idx] = cell;
        self.dirty[row] = true;
    }

    /// Put a character at (row, col) with current attributes.
    pub fn put_char(
        &mut self,
        row: usize,
        col: usize,
        ch: char,
        fg: Color,
        bg: Color,
        attrs: CellAttrs,
    ) {
        if row < self.rows && col < self.cols {
            let cell = Cell {
                character: ch,
                fg,
                bg,
                attrs,
            };
            self.set(row, col, cell);
        }
    }

    /// Clear the entire active buffer.
    pub fn clear(&mut self) {
        let size = self.rows * self.cols;
        let grid = self.grid_mut();
        for i in 0..size {
            grid[i] = Cell::default();
        }
        for d in &mut self.dirty {
            *d = true;
        }
    }

    /// Clear from cursor to end of screen.
    pub fn clear_below(&mut self, row: usize, col: usize) {
        // Clear rest of current row
        for c in col..self.cols {
            self.set(row, c, Cell::default());
        }
        // Clear subsequent rows
        for r in (row + 1)..self.rows {
            for c in 0..self.cols {
                let idx = r * self.cols + c;
                self.grid_mut()[idx] = Cell::default();
            }
            self.dirty[r] = true;
        }
    }

    /// Clear from start of screen to cursor.
    pub fn clear_above(&mut self, row: usize, col: usize) {
        for r in 0..row {
            for c in 0..self.cols {
                let idx = r * self.cols + c;
                self.grid_mut()[idx] = Cell::default();
            }
            self.dirty[r] = true;
        }
        for c in 0..=col.min(self.cols - 1) {
            self.set(row, c, Cell::default());
        }
    }

    /// Clear a line.
    pub fn clear_line(&mut self, row: usize, from: usize, to: usize) {
        if row < self.rows {
            for c in from..to.min(self.cols) {
                let idx = row * self.cols + c;
                self.grid_mut()[idx] = Cell::default();
            }
            self.dirty[row] = true;
        }
    }

    /// Scroll up by n lines (within optional scroll region).
    pub fn scroll_up(&mut self, n: usize, top: usize, bottom: usize) {
        let top = top.min(self.rows - 1);
        let bottom = bottom.min(self.rows);
        if top >= bottom {
            return;
        }
        let n = n.min(bottom - top);

        // Save scrolled-out lines to scrollback (only if scrolling the whole screen)
        if !self.using_alternate && top == 0 {
            for i in 0..n {
                let start = i * self.cols;
                let end = start + self.cols;
                let row_data: Vec<Cell> = self.primary[start..end].to_vec();
                self.scrollback.push(row_data);
                if self.scrollback.len() > self.scrollback_limit {
                    self.scrollback.remove(0);
                }
            }
        }

        // Shift rows up
        let cols = self.cols;
        let grid = self.grid_mut();
        for r in top..(bottom - n) {
            let dst = r * cols;
            let src = (r + n) * cols;
            for c in 0..cols {
                grid[dst + c] = grid[src + c];
            }
        }
        // Clear the bottom n rows
        for r in (bottom - n)..bottom {
            for c in 0..cols {
                grid[r * cols + c] = Cell::default();
            }
        }
        for r in top..bottom {
            self.dirty[r] = true;
        }
    }

    /// Scroll down by n lines (within scroll region).
    pub fn scroll_down(&mut self, n: usize, top: usize, bottom: usize) {
        let top = top.min(self.rows - 1);
        let bottom = bottom.min(self.rows);
        if top >= bottom {
            return;
        }
        let n = n.min(bottom - top);

        let cols = self.cols;
        let grid = self.grid_mut();
        for r in (top + n..bottom).rev() {
            let dst = r * cols;
            let src = (r - n) * cols;
            for c in 0..cols {
                grid[dst + c] = grid[src + c];
            }
        }
        for r in top..(top + n) {
            for c in 0..cols {
                grid[r * cols + c] = Cell::default();
            }
        }
        for r in top..bottom {
            self.dirty[r] = true;
        }
    }

    /// Insert n lines at row, pushing down within scroll region.
    pub fn insert_lines(&mut self, row: usize, n: usize, bottom: usize) {
        if row < bottom {
            self.scroll_down(n, row, bottom);
        }
    }

    /// Delete n lines at row, pulling up within scroll region.
    pub fn delete_lines(&mut self, row: usize, n: usize, bottom: usize) {
        if row < bottom {
            self.scroll_up(n, row, bottom);
        }
    }

    /// Insert n blank characters at (row, col), shifting existing chars right.
    pub fn insert_chars(&mut self, row: usize, col: usize, n: usize) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let cols = self.cols;
        let buf = self.grid_mut();
        let start = row * cols;
        for c in (col + n..cols).rev() {
            buf[start + c] = buf[start + c - n];
        }
        for c in col..core::cmp::min(col + n, cols) {
            buf[start + c] = Cell::default();
        }
        self.dirty[row] = true;
    }

    /// Delete n characters at (row, col), shifting remaining chars left.
    pub fn delete_chars(&mut self, row: usize, col: usize, n: usize) {
        if row >= self.rows || col >= self.cols {
            return;
        }
        let cols = self.cols;
        let buf = self.grid_mut();
        let start = row * cols;
        for c in col..cols {
            if c + n < cols {
                buf[start + c] = buf[start + c + n];
            } else {
                buf[start + c] = Cell::default();
            }
        }
        self.dirty[row] = true;
    }

    /// Erase n characters starting at (row, col) without shifting.
    pub fn erase_chars(&mut self, row: usize, col: usize, n: usize) {
        if row >= self.rows {
            return;
        }
        let cols = self.cols;
        let buf = self.grid_mut();
        let start = row * cols;
        for c in col..core::cmp::min(col + n, cols) {
            buf[start + c] = Cell::default();
        }
        self.dirty[row] = true;
    }

    /// Switch to alternate buffer.
    pub fn enable_alternate(&mut self) {
        if !self.using_alternate {
            self.using_alternate = true;
            // Clear alternate buffer
            let size = self.rows * self.cols;
            for i in 0..size {
                self.alternate[i] = Cell::default();
            }
            for d in &mut self.dirty {
                *d = true;
            }
        }
    }

    /// Switch back to primary buffer.
    pub fn disable_alternate(&mut self) {
        if self.using_alternate {
            self.using_alternate = false;
            for d in &mut self.dirty {
                *d = true;
            }
        }
    }

    /// Resize the buffer.
    pub fn resize(&mut self, new_rows: usize, new_cols: usize) {
        let new_size = new_rows * new_cols;
        let mut new_primary = vec![Cell::default(); new_size];
        let mut new_alternate = vec![Cell::default(); new_size];

        // Copy as much as fits
        let copy_rows = self.rows.min(new_rows);
        let copy_cols = self.cols.min(new_cols);
        for r in 0..copy_rows {
            for c in 0..copy_cols {
                new_primary[r * new_cols + c] = self.primary[r * self.cols + c];
                new_alternate[r * new_cols + c] = self.alternate[r * self.cols + c];
            }
        }

        self.primary = new_primary;
        self.alternate = new_alternate;
        self.rows = new_rows;
        self.cols = new_cols;
        self.dirty = vec![true; new_rows];
    }

    /// Check if a row is dirty.
    pub fn is_dirty(&self, row: usize) -> bool {
        self.dirty.get(row).copied().unwrap_or(false)
    }

    /// Clear dirty flag for a row.
    pub fn clear_dirty(&mut self, row: usize) {
        if row < self.dirty.len() {
            self.dirty[row] = false;
        }
    }

    /// Mark all rows dirty (full repaint).
    pub fn mark_all_dirty(&mut self) {
        for d in &mut self.dirty {
            *d = true;
        }
    }

    /// Get scrollback line count.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Get a scrollback line (0 = oldest).
    pub fn scrollback_line(&self, index: usize) -> Option<&[Cell]> {
        self.scrollback.get(index).map(|v| v.as_slice())
    }
}
