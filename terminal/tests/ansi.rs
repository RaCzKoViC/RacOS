// RacTerm — host-side coverage for the ANSI/VT escape parser + Terminal
// emulator. None of these tests need a real PTY or display, so they run
// under `cargo test -p racterm` on the host toolchain.

use racterm::buffer::Color;
use racterm::escape::{Action, EscParser};
use racterm::terminal::Terminal;

fn term_with(input: &[u8]) -> Terminal {
    let mut t = Terminal::new(10, 20);
    t.feed(input);
    t
}

fn cell_char(t: &Terminal, row: usize, col: usize) -> char {
    t.buffer.get(row, col).character
}

// ─────────────────────────────────────────────────
// Parser ground state
// ─────────────────────────────────────────────────

#[test]
fn parser_emits_print_for_ascii_byte() {
    let mut p = EscParser::new();
    let actions = p.feed_bytes(b"hi");
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0], Action::Print('h'));
    assert_eq!(actions[1], Action::Print('i'));
}

#[test]
fn parser_emits_execute_for_c0_controls() {
    let mut p = EscParser::new();
    let actions = p.feed_bytes(b"a\nb");
    assert!(matches!(actions[0], Action::Print('a')));
    assert!(matches!(actions[1], Action::Execute(0x0A)));
    assert!(matches!(actions[2], Action::Print('b')));
}

#[test]
fn parser_buffers_partial_csi_across_feeds() {
    // The escape sequence is delivered byte-by-byte; parser must hold
    // state and only emit one CsiDispatch at the final byte.
    let mut p = EscParser::new();
    let mut all = Vec::new();
    for b in b"\x1b[3;7H" {
        if let Some(a) = p.feed(*b) {
            all.push(a);
        }
    }
    assert_eq!(all.len(), 1, "single CsiDispatch expected");
    match &all[0] {
        Action::CsiDispatch {
            params,
            final_byte,
            private,
            ..
        } => {
            assert_eq!(params, &[3u16, 7u16]);
            assert_eq!(*final_byte, b'H');
            assert!(!*private);
        }
        other => panic!("expected CsiDispatch, got {:?}", other),
    }
}

#[test]
fn parser_recognises_private_csi() {
    let mut p = EscParser::new();
    let actions = p.feed_bytes(b"\x1b[?25l");
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::CsiDispatch {
            params,
            final_byte,
            private,
            ..
        } => {
            assert!(*private);
            assert_eq!(params, &[25u16]);
            assert_eq!(*final_byte, b'l');
        }
        other => panic!("expected private CSI, got {:?}", other),
    }
}

// ─────────────────────────────────────────────────
// Print + cursor advance
// ─────────────────────────────────────────────────

#[test]
fn print_places_chars_at_cursor_and_advances() {
    let t = term_with(b"abc");
    assert_eq!(cell_char(&t, 0, 0), 'a');
    assert_eq!(cell_char(&t, 0, 1), 'b');
    assert_eq!(cell_char(&t, 0, 2), 'c');
    assert_eq!(t.cursor.col, 3);
    assert_eq!(t.cursor.row, 0);
}

#[test]
fn newline_advances_to_next_row_without_carriage_return() {
    // racterm follows strict VT100: LF advances row only, the column stays
    // put. So after `a\n` the cursor sits at (1, 1) (col 1 because printing
    // 'a' advanced from col 0 to col 1), and 'b' goes to (1, 1).
    let t = term_with(b"a\nb");
    assert_eq!(cell_char(&t, 0, 0), 'a');
    assert_eq!(
        cell_char(&t, 1, 1),
        'b',
        "no implicit CR: 'b' lands at (1, 1)"
    );
    assert_eq!(cell_char(&t, 1, 0), ' ', "(1, 0) was never touched");
    assert_eq!(t.cursor.row, 1);
    assert_eq!(t.cursor.col, 2);
}

#[test]
fn crlf_resets_column() {
    // The well-formed sequence is \r\n. Verify it puts 'b' at column 0.
    let t = term_with(b"a\r\nb");
    assert_eq!(cell_char(&t, 1, 0), 'b');
}

#[test]
fn carriage_return_resets_column() {
    let t = term_with(b"abc\rZ");
    assert_eq!(cell_char(&t, 0, 0), 'Z');
    assert_eq!(cell_char(&t, 0, 1), 'b');
    assert_eq!(t.cursor.col, 1);
}

// ─────────────────────────────────────────────────
// CSI cursor movement
// ─────────────────────────────────────────────────

#[test]
fn cup_sets_cursor_absolute_1_indexed() {
    let t = term_with(b"\x1b[5;9H");
    assert_eq!(t.cursor.row, 4, "ESC[5;9H → row 5 (1-indexed) = 4");
    assert_eq!(t.cursor.col, 8, "ESC[5;9H → col 9 (1-indexed) = 8");
}

#[test]
fn cup_with_no_params_moves_to_origin() {
    let mut t = Terminal::new(10, 20);
    t.feed(b"\x1b[3;3H"); // first jump somewhere
    assert_eq!(t.cursor.row, 2);
    t.feed(b"\x1b[H"); // bare ESC[H
    assert_eq!(t.cursor.row, 0);
    assert_eq!(t.cursor.col, 0);
}

#[test]
fn cuu_cud_cuf_cub_move_relative() {
    let mut t = Terminal::new(10, 20);
    t.feed(b"\x1b[5;10H");
    t.feed(b"\x1b[2A"); // up 2
    assert_eq!(t.cursor.row, 2);
    t.feed(b"\x1b[3B"); // down 3
    assert_eq!(t.cursor.row, 5);
    t.feed(b"\x1b[4D"); // back 4
    assert_eq!(t.cursor.col, 5);
    t.feed(b"\x1b[2C"); // forward 2
    assert_eq!(t.cursor.col, 7);
}

#[test]
fn relative_moves_clamp_at_screen_edges() {
    let mut t = Terminal::new(10, 20);
    t.feed(b"\x1b[1;1H");
    t.feed(b"\x1b[10A"); // try to go way up
    assert_eq!(t.cursor.row, 0);
    t.feed(b"\x1b[100B"); // way down
    assert_eq!(t.cursor.row, 9);
    t.feed(b"\x1b[100C"); // way right
    assert_eq!(t.cursor.col, 19);
    t.feed(b"\x1b[100D"); // way left
    assert_eq!(t.cursor.col, 0);
}

// ─────────────────────────────────────────────────
// Erase
// ─────────────────────────────────────────────────

#[test]
fn ed_2_clears_full_screen() {
    let mut t = term_with(b"hello\nworld");
    t.feed(b"\x1b[2J");
    assert_eq!(cell_char(&t, 0, 0), ' ');
    assert_eq!(cell_char(&t, 1, 0), ' ');
}

#[test]
fn ed_0_clears_below_cursor_inclusive() {
    let mut t = Terminal::new(5, 10);
    t.feed(b"AAAAAAAAAA");
    t.feed(b"\x1b[2;1H");
    t.feed(b"BBBBBBBBBB");
    t.feed(b"\x1b[1;5H"); // back to row 1 col 5
    t.feed(b"\x1b[0J"); // clear from here down
    assert_eq!(cell_char(&t, 0, 4), ' ', "clear from col 5 to end");
    assert_eq!(cell_char(&t, 0, 3), 'A', "col 4 untouched");
    assert_eq!(cell_char(&t, 1, 0), ' ', "row 2 fully cleared");
}

#[test]
fn el_2_clears_whole_line() {
    let mut t = term_with(b"hello world");
    t.feed(b"\x1b[1;1H");
    t.feed(b"\x1b[2K");
    assert_eq!(cell_char(&t, 0, 0), ' ');
    assert_eq!(cell_char(&t, 0, 10), ' ');
}

#[test]
fn el_0_clears_from_cursor_to_end_of_line() {
    let mut t = term_with(b"abcdefgh");
    t.feed(b"\x1b[1;4H"); // col 4 = index 3 = 'd'
    t.feed(b"\x1b[0K");
    assert_eq!(cell_char(&t, 0, 2), 'c', "left side preserved");
    assert_eq!(cell_char(&t, 0, 3), ' ', "cursor cell cleared");
    assert_eq!(cell_char(&t, 0, 7), ' ', "rest of line cleared");
}

// ─────────────────────────────────────────────────
// SGR — colors and attributes
// ─────────────────────────────────────────────────

#[test]
fn sgr_basic_foreground_colors() {
    let mut t = Terminal::new(5, 20);
    t.feed(b"\x1b[31mR"); // red foreground
    t.feed(b"\x1b[32mG"); // green
    t.feed(b"\x1b[0mZ"); // reset, default
    assert_eq!(t.buffer.get(0, 0).fg, Color::Indexed(1));
    assert_eq!(t.buffer.get(0, 1).fg, Color::Indexed(2));
    assert_eq!(t.buffer.get(0, 2).fg, Color::Default);
}

#[test]
fn sgr_bright_foreground_maps_to_indices_8_to_15() {
    let mut t = Terminal::new(5, 20);
    t.feed(b"\x1b[90mB"); // bright black = index 8
    t.feed(b"\x1b[97mW"); // bright white = index 15
    assert_eq!(t.buffer.get(0, 0).fg, Color::Indexed(8));
    assert_eq!(t.buffer.get(0, 1).fg, Color::Indexed(15));
}

#[test]
fn sgr_256_indexed_color() {
    let mut t = Terminal::new(5, 20);
    t.feed(b"\x1b[38;5;208mO"); // orange-ish (208)
    assert_eq!(t.buffer.get(0, 0).fg, Color::Indexed(208));
}

#[test]
fn sgr_truecolor_rgb() {
    let mut t = Terminal::new(5, 20);
    t.feed(b"\x1b[38;2;255;128;0mX");
    assert_eq!(t.buffer.get(0, 0).fg, Color::Rgb(255, 128, 0));
}

#[test]
fn sgr_attrs_bold_and_underline() {
    let mut t = Terminal::new(5, 20);
    t.feed(b"\x1b[1;4mA"); // bold + underline
    assert!(t.buffer.get(0, 0).attrs.bold);
    assert!(t.buffer.get(0, 0).attrs.underline);
    t.feed(b"\x1b[22mB"); // bold off
    assert!(!t.buffer.get(0, 1).attrs.bold);
    assert!(t.buffer.get(0, 1).attrs.underline, "underline still on");
}

#[test]
fn sgr_reset_clears_all_attrs() {
    let mut t = Terminal::new(5, 20);
    t.feed(b"\x1b[1;4;31;43mA");
    t.feed(b"\x1b[0mB"); // reset
    assert_eq!(t.buffer.get(0, 1).fg, Color::Default);
    assert_eq!(t.buffer.get(0, 1).bg, Color::Default);
    assert!(!t.buffer.get(0, 1).attrs.bold);
    assert!(!t.buffer.get(0, 1).attrs.underline);
}

// ─────────────────────────────────────────────────
// Alternate buffer + cursor save/restore
// ─────────────────────────────────────────────────

#[test]
fn dec_1049_enters_alternate_buffer_and_returns() {
    let mut t = Terminal::new(5, 20);
    t.feed(b"primary");
    assert_eq!(cell_char(&t, 0, 0), 'p');

    // Enter alternate (also saves cursor)
    t.feed(b"\x1b[?1049h");
    assert_eq!(cell_char(&t, 0, 0), ' ', "alternate starts blank");

    t.feed(b"\x1b[1;1Halt"); // write into alternate
    assert_eq!(cell_char(&t, 0, 0), 'a');

    // Leave alternate (restores cursor)
    t.feed(b"\x1b[?1049l");
    assert_eq!(cell_char(&t, 0, 0), 'p', "primary buffer restored");
}

#[test]
fn dec_25_toggles_cursor_visibility() {
    let mut t = Terminal::new(5, 20);
    assert!(t.cursor.visible);
    t.feed(b"\x1b[?25l");
    assert!(!t.cursor.visible);
    t.feed(b"\x1b[?25h");
    assert!(t.cursor.visible);
}

// ─────────────────────────────────────────────────
// Scroll region (DECSTBM)
// ─────────────────────────────────────────────────

#[test]
fn decstbm_sets_scroll_region_and_homes_cursor() {
    let mut t = Terminal::new(10, 20);
    t.feed(b"\x1b[3;7r"); // scroll region rows 3..7 (1-indexed)
    assert_eq!(t.cursor.scroll_top, 2);
    assert_eq!(t.cursor.scroll_bottom, 7);
    // Spec: cursor goes to top of region after DECSTBM.
    assert_eq!(t.cursor.row, 2);
    assert_eq!(t.cursor.col, 0);
}

// ─────────────────────────────────────────────────
// Scrollback
// ─────────────────────────────────────────────────

#[test]
fn scroll_up_pushes_lines_into_scrollback() {
    let mut t = Terminal::new(3, 10);
    // Print 3 lines (fills screen), then 2 more (forces scroll).
    t.feed(b"line1\r\nline2\r\nline3\r\nline4\r\nline5");
    assert_eq!(t.buffer.scrollback_len(), 2, "two lines pushed back");
    let oldest = t.buffer.scrollback_line(0).unwrap();
    assert_eq!(oldest[0].character, 'l');
    assert_eq!(oldest[1].character, 'i');
    assert_eq!(oldest[2].character, 'n');
    assert_eq!(oldest[3].character, 'e');
    assert_eq!(oldest[4].character, '1');
}

// ─────────────────────────────────────────────────
// DSR — Device Status Report
// ─────────────────────────────────────────────────

#[test]
fn dsr_cpr_responds_with_cursor_position() {
    let mut t = Terminal::new(10, 20);
    t.feed(b"\x1b[4;7H"); // move to row 4 col 7
    t.feed(b"\x1b[6n"); // CPR query
    let resp = t.drain_response();
    // Reply format: ESC [ row ; col R (1-indexed)
    assert_eq!(resp, b"\x1b[4;7R");
}

#[test]
fn dsr_status_returns_ok() {
    let mut t = Terminal::new(10, 20);
    t.feed(b"\x1b[5n");
    assert_eq!(t.drain_response(), b"\x1b[0n");
}

#[test]
fn da_primary_responds_with_vt100_attrs() {
    let mut t = Terminal::new(10, 20);
    t.feed(b"\x1b[c");
    assert_eq!(t.drain_response(), b"\x1b[?1;2c");
}

#[test]
fn drain_response_clears_buffer() {
    let mut t = Terminal::new(10, 20);
    t.feed(b"\x1b[6n");
    let _first = t.drain_response();
    let second = t.drain_response();
    assert!(second.is_empty(), "second drain must be empty");
}

// ─────────────────────────────────────────────────
// OSC — window title
// ─────────────────────────────────────────────────

#[test]
fn osc_0_sets_window_title() {
    let mut t = Terminal::new(10, 20);
    // OSC 0;TITLE ST(BEL=0x07)
    t.feed(b"\x1b]0;RacOS Shell\x07");
    assert_eq!(t.title, "RacOS Shell");
}
