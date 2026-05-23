// RacTerm — Cursor State Machine (TERMINAL_PROTOCOLS.md §3/§4)
//
// Cursor position, visibility, saved positions.

/// Cursor state.
#[derive(Debug, Clone)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub visible: bool,
    /// Saved cursor position (DECSC/DECRC and CSI s/u).
    saved_row: usize,
    saved_col: usize,
    /// Scroll region (top..bottom rows, inclusive top, exclusive bottom).
    pub scroll_top: usize,
    pub scroll_bottom: usize,
    /// Screen dimensions (for clamping).
    max_rows: usize,
    max_cols: usize,
    /// Wrap-pending flag: when cursor reaches right margin,
    /// the next character should wrap to next line.
    pub wrap_pending: bool,
}

impl Cursor {
    pub fn new(rows: usize, cols: usize) -> Self {
        Cursor {
            row: 0,
            col: 0,
            visible: true,
            saved_row: 0,
            saved_col: 0,
            scroll_top: 0,
            scroll_bottom: rows,
            max_rows: rows,
            max_cols: cols,
            wrap_pending: false,
        }
    }

    pub fn move_up(&mut self, n: usize) {
        self.row = self.row.saturating_sub(n);
        if self.row < self.scroll_top {
            self.row = self.scroll_top;
        }
        self.wrap_pending = false;
    }

    pub fn move_down(&mut self, n: usize) {
        self.row = (self.row + n).min(self.scroll_bottom - 1);
        self.wrap_pending = false;
    }

    pub fn move_forward(&mut self, n: usize) {
        self.col = (self.col + n).min(self.max_cols - 1);
        self.wrap_pending = false;
    }

    pub fn move_back(&mut self, n: usize) {
        self.col = self.col.saturating_sub(n);
        self.wrap_pending = false;
    }

    /// Set absolute position (1-indexed input, stored as 0-indexed).
    pub fn set_position(&mut self, row: usize, col: usize) {
        self.row = row.saturating_sub(1).min(self.max_rows - 1);
        self.col = col.saturating_sub(1).min(self.max_cols - 1);
        self.wrap_pending = false;
    }

    /// Carriage return: move to column 0.
    pub fn carriage_return(&mut self) {
        self.col = 0;
        self.wrap_pending = false;
    }

    /// Line feed: move down one row, scrolling if needed.
    /// Returns true if scroll is needed.
    pub fn line_feed(&mut self) -> bool {
        self.wrap_pending = false;
        if self.row + 1 >= self.scroll_bottom {
            // Need to scroll
            true
        } else {
            self.row += 1;
            false
        }
    }

    /// Reverse index: move up, scroll down if at top of scroll region.
    /// Returns true if reverse scroll is needed.
    pub fn reverse_index(&mut self) -> bool {
        self.wrap_pending = false;
        if self.row <= self.scroll_top {
            true
        } else {
            self.row -= 1;
            false
        }
    }

    /// Advance cursor after printing a character.
    /// Returns (needs_wrap, needs_scroll).
    pub fn advance_after_print(&mut self) -> (bool, bool) {
        if self.col + 1 >= self.max_cols {
            // At right margin — set wrap pending
            self.wrap_pending = true;
            (false, false)
        } else {
            self.col += 1;
            (false, false)
        }
    }

    /// Handle wrap: called before printing when wrap_pending is true.
    /// Returns true if scroll is needed.
    pub fn do_wrap(&mut self) -> bool {
        self.wrap_pending = false;
        self.col = 0;
        self.line_feed()
    }

    /// Backspace: move back one column.
    pub fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        }
        self.wrap_pending = false;
    }

    /// Tab: advance to next 8-column tab stop.
    pub fn tab(&mut self) {
        self.col = ((self.col / 8) + 1) * 8;
        if self.col >= self.max_cols {
            self.col = self.max_cols - 1;
        }
        self.wrap_pending = false;
    }

    /// Save cursor position.
    pub fn save(&mut self) {
        self.saved_row = self.row;
        self.saved_col = self.col;
    }

    /// Restore cursor position.
    pub fn restore(&mut self) {
        self.row = self.saved_row.min(self.max_rows - 1);
        self.col = self.saved_col.min(self.max_cols - 1);
        self.wrap_pending = false;
    }

    /// Set scroll region (1-indexed, inclusive).
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let top = top.saturating_sub(1);
        let bottom = if bottom == 0 {
            self.max_rows
        } else {
            bottom.min(self.max_rows)
        };
        if top < bottom {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            // Move cursor to home position
            self.row = top;
            self.col = 0;
            self.wrap_pending = false;
        }
    }

    /// Reset scroll region to full screen.
    pub fn reset_scroll_region(&mut self) {
        self.scroll_top = 0;
        self.scroll_bottom = self.max_rows;
    }

    /// Resize: update dimensions and clamp cursor.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.max_rows = rows;
        self.max_cols = cols;
        self.scroll_top = 0;
        self.scroll_bottom = rows;
        self.row = self.row.min(rows.saturating_sub(1));
        self.col = self.col.min(cols.saturating_sub(1));
        self.wrap_pending = false;
    }
}
