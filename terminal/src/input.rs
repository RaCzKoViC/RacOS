// RacTerm — Input Decoder (TERMINAL_PROTOCOLS.md §6)
//
// Converts keyboard events to byte sequences sent to PTY.

extern crate alloc;

use alloc::vec::Vec;

/// A keyboard event.
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub key: Key,
    pub modifiers: Modifiers,
}

/// Key identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(u8), // ASCII character
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    F(u8), // F1-F12
}

/// Modifier flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Encode a key event into the byte sequence to send to the PTY.
pub fn encode_key(event: &KeyEvent) -> Vec<u8> {
    let mut out = Vec::new();

    if event.modifiers.ctrl {
        match event.key {
            Key::Char(c) if c >= b'a' && c <= b'z' => {
                out.push(c - b'a' + 1); // Ctrl-A=0x01 ... Ctrl-Z=0x1A
                return out;
            }
            Key::Char(c) if c >= b'A' && c <= b'Z' => {
                out.push(c - b'A' + 1);
                return out;
            }
            Key::Char(b'[') => {
                out.push(0x1B);
                return out;
            } // ESC
            Key::Char(b'\\') => {
                out.push(0x1C);
                return out;
            }
            Key::Char(b']') => {
                out.push(0x1D);
                return out;
            }
            Key::Char(b'^') => {
                out.push(0x1E);
                return out;
            }
            Key::Char(b'_') => {
                out.push(0x1F);
                return out;
            }
            _ => {}
        }
    }

    match event.key {
        Key::Char(c) => {
            if event.modifiers.alt {
                out.push(0x1B); // ESC prefix for Alt
            }
            out.push(c);
        }
        Key::Enter => out.push(b'\r'),
        Key::Backspace => out.push(0x7F),
        Key::Tab => out.push(b'\t'),
        Key::Escape => out.push(0x1B),
        Key::Up => {
            out.extend_from_slice(b"\x1B[A");
        }
        Key::Down => {
            out.extend_from_slice(b"\x1B[B");
        }
        Key::Right => {
            out.extend_from_slice(b"\x1B[C");
        }
        Key::Left => {
            out.extend_from_slice(b"\x1B[D");
        }
        Key::Home => {
            out.extend_from_slice(b"\x1B[H");
        }
        Key::End => {
            out.extend_from_slice(b"\x1B[F");
        }
        Key::PageUp => {
            out.extend_from_slice(b"\x1B[5~");
        }
        Key::PageDown => {
            out.extend_from_slice(b"\x1B[6~");
        }
        Key::Delete => {
            out.extend_from_slice(b"\x1B[3~");
        }
        Key::Insert => {
            out.extend_from_slice(b"\x1B[2~");
        }
        Key::F(n) => match n {
            1 => out.extend_from_slice(b"\x1BOP"),
            2 => out.extend_from_slice(b"\x1BOQ"),
            3 => out.extend_from_slice(b"\x1BOR"),
            4 => out.extend_from_slice(b"\x1BOS"),
            5 => out.extend_from_slice(b"\x1B[15~"),
            6 => out.extend_from_slice(b"\x1B[17~"),
            7 => out.extend_from_slice(b"\x1B[18~"),
            8 => out.extend_from_slice(b"\x1B[19~"),
            9 => out.extend_from_slice(b"\x1B[20~"),
            10 => out.extend_from_slice(b"\x1B[21~"),
            11 => out.extend_from_slice(b"\x1B[23~"),
            12 => out.extend_from_slice(b"\x1B[24~"),
            _ => {}
        },
    }

    out
}
