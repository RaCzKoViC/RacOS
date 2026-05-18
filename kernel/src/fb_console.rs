// RaCore — Framebuffer Console
//
// Text-mode console on top of linear framebuffer.
// Supports basic text output with scrolling, colors, etc.

#![allow(static_mut_refs)]

use core::ptr;
use crate::boot::BootInfo;

/// Console colors (VGA-compatible).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Black = 0,
    Blue = 1,
    Green = 2,
    Cyan = 3,
    Red = 4,
    Magenta = 5,
    Brown = 6,
    LightGray = 7,
    DarkGray = 8,
    LightBlue = 9,
    LightGreen = 10,
    LightCyan = 11,
    LightRed = 12,
    LightMagenta = 13,
    Yellow = 14,
    White = 15,
}

/// Character with color attributes.
#[derive(Debug, Clone, Copy)]
struct Char {
    char: u8,
    fg: Color,
    bg: Color,
}

/// Complete 8x16 font for all 128 ASCII characters.
/// Each glyph is 8 pixels wide, 16 pixels tall.
/// Row 0 is the topmost row; bit 7 (MSB) is the leftmost pixel.
const FONT: [[u8; 16]; 128] = {
    let mut f = [[0u8; 16]; 128];

    // ── Control characters 0-31 and 127 left blank ──

    // 0x20 SPACE
    f[0x20] = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
    // 0x21 !
    f[0x21] = [0,0,0x18,0x18,0x18,0x18,0x18,0x18,0x00,0x18,0x18,0,0,0,0,0];
    // 0x22 "
    f[0x22] = [0,0x66,0x66,0x66,0x24,0,0,0,0,0,0,0,0,0,0,0];
    // 0x23 #
    f[0x23] = [0,0,0x24,0x24,0x7E,0x24,0x24,0x7E,0x24,0x24,0,0,0,0,0,0];
    // 0x24 $
    f[0x24] = [0,0x10,0x3C,0x50,0x50,0x3C,0x12,0x12,0x7C,0x10,0,0,0,0,0,0];
    // 0x25 %
    f[0x25] = [0,0,0x62,0x64,0x08,0x10,0x20,0x4C,0x8C,0,0,0,0,0,0,0];
    // 0x26 &
    f[0x26] = [0,0,0x30,0x48,0x48,0x30,0x4A,0x44,0x44,0x3A,0,0,0,0,0,0];
    // 0x27 '
    f[0x27] = [0,0x18,0x18,0x18,0x10,0,0,0,0,0,0,0,0,0,0,0];
    // 0x28 (
    f[0x28] = [0,0x08,0x10,0x20,0x20,0x20,0x20,0x20,0x10,0x08,0,0,0,0,0,0];
    // 0x29 )
    f[0x29] = [0,0x20,0x10,0x08,0x08,0x08,0x08,0x08,0x10,0x20,0,0,0,0,0,0];
    // 0x2A *
    f[0x2A] = [0,0,0,0x24,0x18,0x7E,0x18,0x24,0,0,0,0,0,0,0,0];
    // 0x2B +
    f[0x2B] = [0,0,0,0x10,0x10,0x7C,0x10,0x10,0,0,0,0,0,0,0,0];
    // 0x2C ,
    f[0x2C] = [0,0,0,0,0,0,0,0,0x18,0x18,0x08,0x10,0,0,0,0];
    // 0x2D -
    f[0x2D] = [0,0,0,0,0,0x7E,0,0,0,0,0,0,0,0,0,0];
    // 0x2E .
    f[0x2E] = [0,0,0,0,0,0,0,0,0x18,0x18,0,0,0,0,0,0];
    // 0x2F /
    f[0x2F] = [0,0x02,0x02,0x04,0x08,0x10,0x20,0x40,0x80,0x80,0,0,0,0,0,0];

    // ── Digits ──
    // 0x30 0
    f[0x30] = [0,0x3C,0x42,0x46,0x4A,0x52,0x62,0x42,0x42,0x3C,0,0,0,0,0,0];
    // 0x31 1
    f[0x31] = [0,0x10,0x30,0x50,0x10,0x10,0x10,0x10,0x10,0x7C,0,0,0,0,0,0];
    // 0x32 2
    f[0x32] = [0,0x3C,0x42,0x02,0x04,0x08,0x10,0x20,0x40,0x7E,0,0,0,0,0,0];
    // 0x33 3
    f[0x33] = [0,0x3C,0x42,0x02,0x02,0x1C,0x02,0x02,0x42,0x3C,0,0,0,0,0,0];
    // 0x34 4
    f[0x34] = [0,0x04,0x0C,0x14,0x24,0x44,0x7E,0x04,0x04,0x04,0,0,0,0,0,0];
    // 0x35 5
    f[0x35] = [0,0x7E,0x40,0x40,0x7C,0x02,0x02,0x02,0x42,0x3C,0,0,0,0,0,0];
    // 0x36 6
    f[0x36] = [0,0x1C,0x20,0x40,0x7C,0x42,0x42,0x42,0x42,0x3C,0,0,0,0,0,0];
    // 0x37 7
    f[0x37] = [0,0x7E,0x02,0x04,0x08,0x10,0x10,0x10,0x10,0x10,0,0,0,0,0,0];
    // 0x38 8
    f[0x38] = [0,0x3C,0x42,0x42,0x42,0x3C,0x42,0x42,0x42,0x3C,0,0,0,0,0,0];
    // 0x39 9
    f[0x39] = [0,0x3C,0x42,0x42,0x42,0x3E,0x02,0x02,0x04,0x38,0,0,0,0,0,0];

    // 0x3A :
    f[0x3A] = [0,0,0,0x18,0x18,0,0,0x18,0x18,0,0,0,0,0,0,0];
    // 0x3B ;
    f[0x3B] = [0,0,0,0x18,0x18,0,0,0x18,0x18,0x08,0x10,0,0,0,0,0];
    // 0x3C <
    f[0x3C] = [0,0,0x04,0x08,0x10,0x20,0x10,0x08,0x04,0,0,0,0,0,0,0];
    // 0x3D =
    f[0x3D] = [0,0,0,0,0x7E,0,0x7E,0,0,0,0,0,0,0,0,0];
    // 0x3E >
    f[0x3E] = [0,0,0x20,0x10,0x08,0x04,0x08,0x10,0x20,0,0,0,0,0,0,0];
    // 0x3F ?
    f[0x3F] = [0,0x3C,0x42,0x02,0x04,0x08,0x08,0,0x08,0x08,0,0,0,0,0,0];
    // 0x40 @
    f[0x40] = [0,0x3C,0x42,0x42,0x4E,0x52,0x4E,0x40,0x40,0x3C,0,0,0,0,0,0];

    // ── Uppercase letters ──
    // 0x41 A
    f[0x41] = [0,0x18,0x24,0x42,0x42,0x7E,0x42,0x42,0x42,0x42,0,0,0,0,0,0];
    // 0x42 B
    f[0x42] = [0,0x7C,0x42,0x42,0x42,0x7C,0x42,0x42,0x42,0x7C,0,0,0,0,0,0];
    // 0x43 C
    f[0x43] = [0,0x3C,0x42,0x40,0x40,0x40,0x40,0x40,0x42,0x3C,0,0,0,0,0,0];
    // 0x44 D
    f[0x44] = [0,0x78,0x44,0x42,0x42,0x42,0x42,0x42,0x44,0x78,0,0,0,0,0,0];
    // 0x45 E
    f[0x45] = [0,0x7E,0x40,0x40,0x40,0x7C,0x40,0x40,0x40,0x7E,0,0,0,0,0,0];
    // 0x46 F
    f[0x46] = [0,0x7E,0x40,0x40,0x40,0x7C,0x40,0x40,0x40,0x40,0,0,0,0,0,0];
    // 0x47 G
    f[0x47] = [0,0x3C,0x42,0x40,0x40,0x4E,0x42,0x42,0x42,0x3C,0,0,0,0,0,0];
    // 0x48 H
    f[0x48] = [0,0x42,0x42,0x42,0x42,0x7E,0x42,0x42,0x42,0x42,0,0,0,0,0,0];
    // 0x49 I
    f[0x49] = [0,0x3C,0x10,0x10,0x10,0x10,0x10,0x10,0x10,0x3C,0,0,0,0,0,0];
    // 0x4A J
    f[0x4A] = [0,0x1E,0x04,0x04,0x04,0x04,0x04,0x44,0x44,0x38,0,0,0,0,0,0];
    // 0x4B K
    f[0x4B] = [0,0x42,0x44,0x48,0x50,0x60,0x50,0x48,0x44,0x42,0,0,0,0,0,0];
    // 0x4C L
    f[0x4C] = [0,0x40,0x40,0x40,0x40,0x40,0x40,0x40,0x40,0x7E,0,0,0,0,0,0];
    // 0x4D M
    f[0x4D] = [0,0x42,0x66,0x5A,0x5A,0x42,0x42,0x42,0x42,0x42,0,0,0,0,0,0];
    // 0x4E N
    f[0x4E] = [0,0x42,0x62,0x52,0x4A,0x46,0x42,0x42,0x42,0x42,0,0,0,0,0,0];
    // 0x4F O
    f[0x4F] = [0,0x3C,0x42,0x42,0x42,0x42,0x42,0x42,0x42,0x3C,0,0,0,0,0,0];
    // 0x50 P
    f[0x50] = [0,0x7C,0x42,0x42,0x42,0x7C,0x40,0x40,0x40,0x40,0,0,0,0,0,0];
    // 0x51 Q
    f[0x51] = [0,0x3C,0x42,0x42,0x42,0x42,0x42,0x4A,0x44,0x3A,0,0,0,0,0,0];
    // 0x52 R
    f[0x52] = [0,0x7C,0x42,0x42,0x42,0x7C,0x48,0x44,0x42,0x42,0,0,0,0,0,0];
    // 0x53 S
    f[0x53] = [0,0x3C,0x42,0x40,0x40,0x3C,0x02,0x02,0x42,0x3C,0,0,0,0,0,0];
    // 0x54 T
    f[0x54] = [0,0x7E,0x10,0x10,0x10,0x10,0x10,0x10,0x10,0x10,0,0,0,0,0,0];
    // 0x55 U
    f[0x55] = [0,0x42,0x42,0x42,0x42,0x42,0x42,0x42,0x42,0x3C,0,0,0,0,0,0];
    // 0x56 V
    f[0x56] = [0,0x42,0x42,0x42,0x42,0x42,0x24,0x24,0x18,0x18,0,0,0,0,0,0];
    // 0x57 W
    f[0x57] = [0,0x42,0x42,0x42,0x42,0x42,0x5A,0x5A,0x66,0x42,0,0,0,0,0,0];
    // 0x58 X
    f[0x58] = [0,0x42,0x42,0x24,0x18,0x18,0x24,0x42,0x42,0x42,0,0,0,0,0,0];
    // 0x59 Y
    f[0x59] = [0,0x44,0x44,0x28,0x28,0x10,0x10,0x10,0x10,0x10,0,0,0,0,0,0];
    // 0x5A Z
    f[0x5A] = [0,0x7E,0x02,0x04,0x08,0x10,0x20,0x40,0x40,0x7E,0,0,0,0,0,0];

    // 0x5B [
    f[0x5B] = [0,0x3C,0x20,0x20,0x20,0x20,0x20,0x20,0x20,0x3C,0,0,0,0,0,0];
    // 0x5C backslash
    f[0x5C] = [0,0x80,0x80,0x40,0x20,0x10,0x08,0x04,0x02,0x02,0,0,0,0,0,0];
    // 0x5D ]
    f[0x5D] = [0,0x3C,0x04,0x04,0x04,0x04,0x04,0x04,0x04,0x3C,0,0,0,0,0,0];
    // 0x5E ^
    f[0x5E] = [0,0x10,0x28,0x44,0,0,0,0,0,0,0,0,0,0,0,0];
    // 0x5F _
    f[0x5F] = [0,0,0,0,0,0,0,0,0,0,0x7E,0,0,0,0,0];
    // 0x60 `
    f[0x60] = [0,0x20,0x10,0x08,0,0,0,0,0,0,0,0,0,0,0,0];

    // ── Lowercase letters ──
    // 0x61 a
    f[0x61] = [0,0,0,0,0x3C,0x02,0x3E,0x42,0x42,0x3E,0,0,0,0,0,0];
    // 0x62 b
    f[0x62] = [0,0x40,0x40,0x40,0x5C,0x62,0x42,0x42,0x62,0x5C,0,0,0,0,0,0];
    // 0x63 c
    f[0x63] = [0,0,0,0,0x3C,0x42,0x40,0x40,0x42,0x3C,0,0,0,0,0,0];
    // 0x64 d
    f[0x64] = [0,0x02,0x02,0x02,0x3A,0x46,0x42,0x42,0x46,0x3A,0,0,0,0,0,0];
    // 0x65 e
    f[0x65] = [0,0,0,0,0x3C,0x42,0x7E,0x40,0x42,0x3C,0,0,0,0,0,0];
    // 0x66 f
    f[0x66] = [0,0x0C,0x12,0x10,0x10,0x7C,0x10,0x10,0x10,0x10,0,0,0,0,0,0];
    // 0x67 g
    f[0x67] = [0,0,0,0,0x3A,0x46,0x42,0x42,0x46,0x3A,0x02,0x3C,0,0,0,0];
    // 0x68 h
    f[0x68] = [0,0x40,0x40,0x40,0x5C,0x62,0x42,0x42,0x42,0x42,0,0,0,0,0,0];
    // 0x69 i
    f[0x69] = [0,0x10,0,0,0x30,0x10,0x10,0x10,0x10,0x38,0,0,0,0,0,0];
    // 0x6A j
    f[0x6A] = [0,0x04,0,0,0x0C,0x04,0x04,0x04,0x04,0x04,0x44,0x38,0,0,0,0];
    // 0x6B k
    f[0x6B] = [0,0x40,0x40,0x40,0x44,0x48,0x70,0x48,0x44,0x42,0,0,0,0,0,0];
    // 0x6C l
    f[0x6C] = [0,0x30,0x10,0x10,0x10,0x10,0x10,0x10,0x10,0x38,0,0,0,0,0,0];
    // 0x6D m
    f[0x6D] = [0,0,0,0,0x76,0x49,0x49,0x49,0x49,0x49,0,0,0,0,0,0];
    // 0x6E n
    f[0x6E] = [0,0,0,0,0x5C,0x62,0x42,0x42,0x42,0x42,0,0,0,0,0,0];
    // 0x6F o
    f[0x6F] = [0,0,0,0,0x3C,0x42,0x42,0x42,0x42,0x3C,0,0,0,0,0,0];
    // 0x70 p
    f[0x70] = [0,0,0,0,0x5C,0x62,0x42,0x42,0x62,0x5C,0x40,0x40,0,0,0,0];
    // 0x71 q
    f[0x71] = [0,0,0,0,0x3A,0x46,0x42,0x42,0x46,0x3A,0x02,0x02,0,0,0,0];
    // 0x72 r
    f[0x72] = [0,0,0,0,0x5C,0x62,0x40,0x40,0x40,0x40,0,0,0,0,0,0];
    // 0x73 s
    f[0x73] = [0,0,0,0,0x3E,0x40,0x3C,0x02,0x02,0x7C,0,0,0,0,0,0];
    // 0x74 t
    f[0x74] = [0,0x10,0x10,0x10,0x7C,0x10,0x10,0x10,0x12,0x0C,0,0,0,0,0,0];
    // 0x75 u
    f[0x75] = [0,0,0,0,0x42,0x42,0x42,0x42,0x46,0x3A,0,0,0,0,0,0];
    // 0x76 v
    f[0x76] = [0,0,0,0,0x42,0x42,0x42,0x24,0x24,0x18,0,0,0,0,0,0];
    // 0x77 w
    f[0x77] = [0,0,0,0,0x42,0x42,0x42,0x5A,0x66,0x42,0,0,0,0,0,0];
    // 0x78 x
    f[0x78] = [0,0,0,0,0x42,0x24,0x18,0x18,0x24,0x42,0,0,0,0,0,0];
    // 0x79 y
    f[0x79] = [0,0,0,0,0x42,0x42,0x42,0x46,0x3A,0x02,0x42,0x3C,0,0,0,0];
    // 0x7A z
    f[0x7A] = [0,0,0,0,0x7E,0x04,0x08,0x10,0x20,0x7E,0,0,0,0,0,0];
    // 0x7B {
    f[0x7B] = [0,0x0C,0x10,0x10,0x10,0x60,0x10,0x10,0x10,0x0C,0,0,0,0,0,0];
    // 0x7C |
    f[0x7C] = [0,0x10,0x10,0x10,0x10,0x10,0x10,0x10,0x10,0x10,0,0,0,0,0,0];
    // 0x7D }
    f[0x7D] = [0,0x30,0x08,0x08,0x08,0x06,0x08,0x08,0x08,0x30,0,0,0,0,0,0];
    // 0x7E ~
    f[0x7E] = [0,0,0,0x32,0x4C,0,0,0,0,0,0,0,0,0,0,0];

    f
};

/// Framebuffer console state.
pub struct FramebufferConsole {
    fb_addr: *mut u32,  // Pointer to framebuffer
    width: u32,         // Pixels
    height: u32,        // Pixels
    pitch: u32,         // Bytes per row
    bpp: u8,            // Bits per pixel
    char_width: u32,    // Character width in pixels (8)
    char_height: u32,   // Character height in pixels (16)
    cols: u32,          // Text columns
    rows: u32,          // Text rows
    cursor_x: u32,      // Current cursor column
    cursor_y: u32,      // Current cursor row
    current_fg: Color,  // Current foreground color
    current_bg: Color,  // Current background color
}

impl FramebufferConsole {
    /// Text columns.
    pub fn cols(&self) -> u32 { self.cols }
    /// Text rows.
    pub fn rows(&self) -> u32 { self.rows }

    /// Initialize framebuffer console from BootInfo.
    ///
    /// # Safety
    /// BootInfo framebuffer must be valid and accessible.
    pub unsafe fn new(boot_info: &BootInfo) -> Option<Self> {
        if boot_info.framebuffer.address == 0 {
            return None;
        }

        let fb = &boot_info.framebuffer;
        let char_width = 8;
        let char_height = 16;
        let cols = fb.width / char_width;
        let rows = fb.height / char_height;

        Some(FramebufferConsole {
            fb_addr: fb.address as *mut u32,
            width: fb.width,
            height: fb.height,
            pitch: fb.pitch,
            bpp: fb.bpp,
            char_width,
            char_height,
            cols,
            rows,
            cursor_x: 0,
            cursor_y: 0,
            current_fg: Color::White,
            current_bg: Color::Black,
        })
    }

    /// Clear the entire screen.
    pub fn clear(&mut self) {
        let bg_color = self.color_to_pixel(self.current_bg);
        for y in 0..self.height {
            for x in 0..self.width {
                self.put_pixel(x, y, bg_color);
            }
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
    }

    /// Write a single character at current cursor position.
    pub fn put_char(&mut self, c: u8) {
        match c {
            b'\n' => {
                self.cursor_x = 0;
                self.cursor_y += 1;
                if self.cursor_y >= self.rows {
                    self.scroll();
                    self.cursor_y = self.rows - 1;
                }
            }
            b'\r' => {
                self.cursor_x = 0;
            }
            b'\t' => {
                self.cursor_x = (self.cursor_x + 8) & !7; // Tab to next 8-column boundary
                if self.cursor_x >= self.cols {
                    self.put_char(b'\n');
                }
            }
            0x20..=0x7E => { // Printable ASCII
                self.draw_char(c);
                self.cursor_x += 1;
                if self.cursor_x >= self.cols {
                    self.put_char(b'\n');
                }
            }
            _ => {} // Ignore other characters for now
        }
    }

    /// Write a string with ANSI escape sequence support.
    pub fn write_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1B && i + 1 < bytes.len() && bytes[i+1] == b'[' {
                // CSI sequence: collect parameter bytes and final byte
                let mut j = i + 2;
                while j < bytes.len() && (bytes[j] < 0x40 || bytes[j] > 0x7E) {
                    j += 1;
                }
                if j < bytes.len() {
                    let final_byte = bytes[j];
                    let params = &bytes[i+2..j];
                    self.handle_csi(params, final_byte);
                    i = j + 1;
                    continue;
                }
            }
            self.put_char(bytes[i]);
            i += 1;
        }
    }

    /// Parse CSI parameter string into a list of numeric values.
    fn parse_csi_params(params: &[u8]) -> [u32; 8] {
        let mut result = [0u32; 8];
        let mut idx = 0;
        let mut val = 0u32;
        let mut has_val = false;
        for &b in params {
            if b == b';' {
                if idx < 8 {
                    result[idx] = val;
                    idx += 1;
                }
                val = 0;
                has_val = false;
            } else if b >= b'0' && b <= b'9' {
                val = val * 10 + (b - b'0') as u32;
                has_val = true;
            }
        }
        if (has_val || idx == 0) && idx < 8 {
            result[idx] = val;
        }
        result
    }

    fn csi_param_count(params: &[u8]) -> usize {
        if params.is_empty() { return 0; }
        let mut count = 1;
        for &b in params { if b == b';' { count += 1; } }
        count
    }

    /// Handle a CSI sequence with given parameter bytes and final character.
    fn handle_csi(&mut self, params: &[u8], final_byte: u8) {
        let p = Self::parse_csi_params(params);
        match final_byte {
            b'm' => self.handle_sgr(params),
            b'A' => { // CUU – Cursor Up
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.cursor_y = self.cursor_y.saturating_sub(n);
            }
            b'B' => { // CUD – Cursor Down
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.cursor_y = core::cmp::min(self.cursor_y + n, self.rows - 1);
            }
            b'C' => { // CUF – Cursor Forward
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.cursor_x = core::cmp::min(self.cursor_x + n, self.cols - 1);
            }
            b'D' => { // CUB – Cursor Back
                let n = if p[0] == 0 { 1 } else { p[0] };
                self.cursor_x = self.cursor_x.saturating_sub(n);
            }
            b'H' | b'f' => { // CUP – Cursor Position (1-based)
                let row = if p[0] == 0 { 1 } else { p[0] };
                let col = if p[1] == 0 { 1 } else { p[1] };
                self.cursor_y = core::cmp::min(row - 1, self.rows - 1);
                self.cursor_x = core::cmp::min(col - 1, self.cols - 1);
            }
            b'J' => { // ED – Erase in Display
                match p[0] {
                    0 => self.erase_below(),
                    1 => self.erase_above(),
                    2 | 3 => { self.clear(); }
                    _ => {}
                }
            }
            b'K' => { // EL – Erase in Line
                match p[0] {
                    0 => self.erase_line_right(),
                    1 => self.erase_line_left(),
                    2 => self.erase_line(),
                    _ => {}
                }
            }
            _ => {} // Unknown CSI — ignore
        }
    }

    fn handle_sgr(&mut self, params: &[u8]) {
        if params.is_empty() {
            self.current_fg = Color::White;
            self.current_bg = Color::Black;
            return;
        }

        let p = Self::parse_csi_params(params);
        let count = Self::csi_param_count(params);
        for i in 0..count {
            match p[i] {
                0 => { self.current_fg = Color::White; self.current_bg = Color::Black; }
                1 => {} // Bold – ignored for now
                30 => self.current_fg = Color::Black,
                31 => self.current_fg = Color::Red,
                32 => self.current_fg = Color::Green,
                33 => self.current_fg = Color::Brown,
                34 => self.current_fg = Color::Blue,
                35 => self.current_fg = Color::Magenta,
                36 => self.current_fg = Color::Cyan,
                37 => self.current_fg = Color::LightGray,
                39 => self.current_fg = Color::White,
                40 => self.current_bg = Color::Black,
                41 => self.current_bg = Color::Red,
                42 => self.current_bg = Color::Green,
                43 => self.current_bg = Color::Brown,
                44 => self.current_bg = Color::Blue,
                45 => self.current_bg = Color::Magenta,
                46 => self.current_bg = Color::Cyan,
                47 => self.current_bg = Color::LightGray,
                49 => self.current_bg = Color::Black,
                // Bright foreground
                90 => self.current_fg = Color::DarkGray,
                91 => self.current_fg = Color::LightRed,
                92 => self.current_fg = Color::LightGreen,
                93 => self.current_fg = Color::Yellow,
                94 => self.current_fg = Color::LightBlue,
                95 => self.current_fg = Color::LightMagenta,
                96 => self.current_fg = Color::LightCyan,
                97 => self.current_fg = Color::White,
                // Bright background
                100 => self.current_bg = Color::DarkGray,
                101 => self.current_bg = Color::LightRed,
                102 => self.current_bg = Color::LightGreen,
                103 => self.current_bg = Color::Yellow,
                104 => self.current_bg = Color::LightBlue,
                105 => self.current_bg = Color::LightMagenta,
                106 => self.current_bg = Color::LightCyan,
                107 => self.current_bg = Color::White,
                _ => {}
            }
        }
    }

    /// Erase from cursor to end of screen.
    fn erase_below(&mut self) {
        let bg = self.color_to_pixel(self.current_bg);
        // Erase rest of current line
        for x in self.cursor_x..self.cols {
            self.clear_cell(x, self.cursor_y, bg);
        }
        // Erase all lines below
        for y in (self.cursor_y + 1)..self.rows {
            for x in 0..self.cols {
                self.clear_cell(x, y, bg);
            }
        }
    }

    /// Erase from start of screen to cursor.
    fn erase_above(&mut self) {
        let bg = self.color_to_pixel(self.current_bg);
        for y in 0..self.cursor_y {
            for x in 0..self.cols {
                self.clear_cell(x, y, bg);
            }
        }
        for x in 0..=self.cursor_x {
            self.clear_cell(x, self.cursor_y, bg);
        }
    }

    /// Erase from cursor to end of line.
    fn erase_line_right(&mut self) {
        let bg = self.color_to_pixel(self.current_bg);
        for x in self.cursor_x..self.cols {
            self.clear_cell(x, self.cursor_y, bg);
        }
    }

    /// Erase from start of line to cursor.
    fn erase_line_left(&mut self) {
        let bg = self.color_to_pixel(self.current_bg);
        for x in 0..=self.cursor_x {
            self.clear_cell(x, self.cursor_y, bg);
        }
    }

    /// Erase entire current line.
    fn erase_line(&mut self) {
        let bg = self.color_to_pixel(self.current_bg);
        for x in 0..self.cols {
            self.clear_cell(x, self.cursor_y, bg);
        }
    }

    /// Clear a single character cell.
    fn clear_cell(&mut self, col: u32, row: u32, bg: u32) {
        let sx = col * self.char_width;
        let sy = row * self.char_height;
        for dy in 0..self.char_height {
            for dx in 0..self.char_width {
                self.put_pixel(sx + dx, sy + dy, bg);
            }
        }
    }

    /// Set foreground color.
    pub fn set_fg(&mut self, color: Color) {
        self.current_fg = color;
    }

    /// Set background color.
    pub fn set_bg(&mut self, color: Color) {
        self.current_bg = color;
    }

    /// Scroll the screen up by one line.
    fn scroll(&mut self) {
        let bg_color = self.color_to_pixel(self.current_bg);
        let stride = self.pitch / 4; // pixels per scanline (accounts for padding)

        // Move all lines up by one character row
        unsafe {
            ptr::copy(
                self.fb_addr.add((self.char_height * stride) as usize),
                self.fb_addr,
                ((self.rows - 1) * self.char_height * stride) as usize,
            );
        }

        // Clear the bottom line
        let bottom_start = ((self.rows - 1) * self.char_height * stride) as usize;
        for i in 0..(self.char_height * stride) as usize {
            unsafe {
                *self.fb_addr.add(bottom_start + i) = bg_color;
            }
        }
    }

    /// Draw a character at current cursor position.
    fn draw_char(&mut self, c: u8) {
        let fg_color = self.color_to_pixel(self.current_fg);
        let bg_color = self.color_to_pixel(self.current_bg);

        let start_x = self.cursor_x * self.char_width;
        let start_y = self.cursor_y * self.char_height;

        let glyph = &FONT[c as usize];
        for dy in 0..self.char_height {
            let row = glyph[dy as usize];
            for dx in 0..self.char_width {
                // MSB is left pixel
                let color = if (row & (1 << (7 - dx))) != 0 { fg_color } else { bg_color };
                self.put_pixel(start_x + dx, start_y + dy, color);
            }
        }
    }

    /// Put a pixel at (x, y) with given color.
    fn put_pixel(&mut self, x: u32, y: u32, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }

        let offset = (y * (self.pitch / 4) + x) as usize;
        unsafe {
            *self.fb_addr.add(offset) = color;
        }
    }

    /// Convert Color to pixel value.
    fn color_to_pixel(&self, color: Color) -> u32 {
        // Simple RGB mapping for now
        match color {
            Color::Black => 0x000000,
            Color::Blue => 0x0000AA,
            Color::Green => 0x00AA00,
            Color::Cyan => 0x00AAAA,
            Color::Red => 0xAA0000,
            Color::Magenta => 0xAA00AA,
            Color::Brown => 0xAA5500,
            Color::LightGray => 0xAAAAAA,
            Color::DarkGray => 0x555555,
            Color::LightBlue => 0x5555FF,
            Color::LightGreen => 0x55FF55,
            Color::LightCyan => 0x55FFFF,
            Color::LightRed => 0xFF5555,
            Color::LightMagenta => 0xFF55FF,
            Color::Yellow => 0xFFFF55,
            Color::White => 0xFFFFFF,
        }
    }
}

/// Global framebuffer console instance.
static mut FB_CONSOLE: Option<FramebufferConsole> = None;

/// Initialize framebuffer console.
///
/// # Safety
/// Must be called once during kernel init.
pub unsafe fn init(boot_info: &BootInfo) {
    FB_CONSOLE = FramebufferConsole::new(boot_info);
    if let Some(ref mut console) = FB_CONSOLE {
        console.clear();
        console.write_str("RacOS Framebuffer Console Initialized\n");
    }
}

/// Write to framebuffer console (if available).
pub fn fb_print(s: &str) {
    unsafe {
        if let Some(console) = get_console() {
            console.write_str(s);
        }
    }
}

/// Check if framebuffer console is available.
pub fn is_available() -> bool {
    unsafe { FB_CONSOLE.is_some() }
}

/// Get mutable reference to console (unsafe, single-threaded kernel).
///
/// # Safety
/// Must be called only from kernel code, not from interrupts or multiple threads.
pub unsafe fn get_console() -> Option<&'static mut FramebufferConsole> {
    FB_CONSOLE.as_mut()
}