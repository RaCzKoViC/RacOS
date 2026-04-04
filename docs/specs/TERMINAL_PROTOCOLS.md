# RacOS — Terminal Protocols Specification (RacTerm)

> Version: 0.1.0 | Status: Draft | Component: RacTerm

## 1. Architecture

```
┌──────────────────────────────────────────────────────────┐
│                     RacTerm                              │
│                                                          │
│  ┌──────────┐  ┌────────────┐  ┌──────────────────────┐ │
│  │  Input    │  │  Escape    │  │  Screen Buffer       │ │
│  │  Decoder  │──│  Sequence  │──│  (cells + attrs)     │ │
│  │          │  │  Parser    │  │                      │ │
│  └──────────┘  └────────────┘  └──────────┬───────────┘ │
│                                            │             │
│  ┌──────────┐  ┌────────────┐  ┌──────────▼───────────┐ │
│  │  PTY     │  │  Cursor    │  │  Renderer            │ │
│  │  Session │  │  State     │  │  (dirty region)      │ │
│  │  Binding │  │  Machine   │  │                      │ │
│  └──────────┘  └────────────┘  └──────────────────────┘ │
└──────────────────────────────────────────────────────────┘
```

**Separation rules**:
- Escape sequence parser does NOT render
- Screen buffer is a pure data structure, testable without rendering
- Renderer only reads screen buffer + dirty flags
- Partial escape sequences are buffered safely

## 2. PTY Binding

RacTerm connects to a shell (racsh) via PTY:

```
RacTerm ←→ PTY master fd ←→ PTY slave fd ←→ racsh
```

- Terminal reads from PTY master → parses → renders
- User input → writes to PTY master → reaches shell via slave's stdin
- Resize → ioctl TIOCSWINSZ on PTY master → SIGWINCH to shell

## 3. Screen Buffer

### 3.1 Cell Structure

```rust
struct Cell {
    character: char,         // Unicode codepoint (or ' ' for empty)
    fg: Color,
    bg: Color,
    attrs: CellAttrs,        // bold, italic, underline, blink, reverse, strikethrough
}

struct CellAttrs {
    bold: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    reverse: bool,
    strikethrough: bool,
}

enum Color {
    Default,
    Indexed(u8),             // 0-15 standard, 16-255 extended
    Rgb(u8, u8, u8),         // 24-bit truecolor
}
```

### 3.2 Buffer Layout

- Primary buffer: `rows × cols` grid of `Cell`
- Alternate buffer: separate `rows × cols` grid (for fullscreen apps)
- Scrollback: ring buffer of row arrays, configurable limit (default 10,000 lines)
- Dirty flags: per-row bitflag indicating which rows need re-render

## 4. Escape Sequence Support

### 4.1 CSI Sequences (Control Sequence Introducer: ESC [ ...)

| Sequence | Name | Description |
|----------|------|-------------|
| `ESC[nA` | CUU | Cursor Up n |
| `ESC[nB` | CUD | Cursor Down n |
| `ESC[nC` | CUF | Cursor Forward n |
| `ESC[nD` | CUB | Cursor Back n |
| `ESC[n;mH` | CUP | Cursor Position (row;col) |
| `ESC[nJ` | ED | Erase Display (0=below, 1=above, 2=all) |
| `ESC[nK` | EL | Erase Line (0=right, 1=left, 2=all) |
| `ESC[nL` | IL | Insert n Lines |
| `ESC[nM` | DL | Delete n Lines |
| `ESC[nS` | SU | Scroll Up n lines |
| `ESC[nT` | SD | Scroll Down n lines |
| `ESC[n;rm` | SGR | Select Graphic Rendition |
| `ESC[6n` | DSR | Device Status Report (cursor position) |
| `ESC[s` | SCP | Save Cursor Position |
| `ESC[u` | RCP | Restore Cursor Position |
| `ESC[?25h` | DECTCEM | Show Cursor |
| `ESC[?25l` | DECTCEM | Hide Cursor |
| `ESC[?1049h` | — | Enable Alternate Screen Buffer |
| `ESC[?1049l` | — | Disable Alternate Screen Buffer |
| `ESC[n;nr` | DECSTBM | Set Scroll Region (top;bottom) |

### 4.2 SGR (Select Graphic Rendition)

| Code | Effect |
|------|--------|
| 0 | Reset all attributes |
| 1 | Bold |
| 3 | Italic |
| 4 | Underline |
| 5 | Blink (slow) |
| 7 | Reverse video |
| 9 | Strikethrough |
| 22 | Normal intensity (not bold) |
| 23 | Not italic |
| 24 | Not underline |
| 25 | Not blink |
| 27 | Not reverse |
| 29 | Not strikethrough |
| 30-37 | Set foreground color (standard) |
| 38;5;n | Set foreground (256 color) |
| 38;2;r;g;b | Set foreground (truecolor) |
| 39 | Default foreground |
| 40-47 | Set background color (standard) |
| 48;5;n | Set background (256 color) |
| 48;2;r;g;b | Set background (truecolor) |
| 49 | Default background |
| 90-97 | Set foreground (bright) |
| 100-107 | Set background (bright) |

### 4.3 OSC Sequences (Operating System Command: ESC ] ...)

| Sequence | Purpose |
|----------|---------|
| `ESC]0;title\x07` | Set window title |
| `ESC]8;;url\x07text ESC]8;;\x07` | Hyperlink (post-MVP) |

### 4.4 Simple Escape Sequences

| Sequence | Description |
|----------|-------------|
| `ESC c` | Full reset (RIS) |
| `ESC 7` | Save cursor + attributes (DECSC) |
| `ESC 8` | Restore cursor + attributes (DECRC) |
| `ESC D` | Index (LF, scroll if at bottom) |
| `ESC M` | Reverse Index (scroll down if at top) |
| `ESC E` | Next Line |

## 5. Escape Sequence Parser

### 5.1 State Machine

```
Ground ──(ESC)──→ Escape
  │                  │
  │              ([ )──→ CsiEntry ──(params)──→ CsiParam ──(final)──→ CsiDispatch
  │              (] )──→ OscString ──(\x07 or ESC\)──→ OscDispatch
  │              (other)──→ EscDispatch
  │
  ├──(0x20-0x7E)──→ Print character
  ├──(0x00-0x1F)──→ Execute control (CR, LF, BS, TAB, BEL)
```

### 5.2 Partial Sequence Handling

If the input stream is split mid-sequence (e.g., `ESC[3` without final byte):
1. Buffer incomplete sequence
2. Wait for more data
3. Timeout (100ms): discard incomplete sequence, return to ground state

## 6. Input Handling

### 6.1 Key Encoding

| Key | Sequence sent to PTY |
|-----|---------------------|
| Normal char | UTF-8 bytes |
| Enter | `\r` (CR) |
| Backspace | `\x7F` (DEL) |
| Tab | `\t` |
| Escape | `\x1B` |
| Arrow Up | `ESC[A` |
| Arrow Down | `ESC[B` |
| Arrow Right | `ESC[C` |
| Arrow Left | `ESC[D` |
| Home | `ESC[H` |
| End | `ESC[F` |
| Page Up | `ESC[5~` |
| Page Down | `ESC[6~` |
| Delete | `ESC[3~` |
| F1-F4 | `ESCOP` - `ESCOS` |
| F5-F12 | `ESC[15~` - `ESC[24~` |
| Ctrl-C | `\x03` (ETX) |
| Ctrl-D | `\x04` (EOT) |
| Ctrl-Z | `\x1A` (SUB) |

## 7. Rendering

### 7.1 Dirty Region Strategy

1. Each operation marks affected rows as dirty
2. Renderer only processes dirty rows
3. After render, clear dirty flags
4. Full repaint on resize

### 7.2 Render Targets

| Phase | Target |
|-------|--------|
| Phase 1 | Serial console (text mode) |
| Phase 1 | Framebuffer (bitmap font rendering) |
| Phase 2 | Userland window (when GUI available) |

## 8. Resize Handling

1. Detect new dimensions (from UEFI GOP or window resize event)
2. Resize screen buffer (allocate new grid, copy visible content)
3. Update scrollback if needed
4. Send TIOCSWINSZ ioctl to PTY
5. Mark all rows dirty
6. Full repaint

## 9. Performance Constraints

| Metric | Target |
|--------|--------|
| Latency (keystroke to display) | < 5ms |
| Throughput | ≥ 100 MB/s without memory growth |
| Scrollback memory | Bounded by configured limit |
| Partial sequence buffer | Max 256 bytes |

## 10. Test Plan

- Escape sequence unit tests: each CSI/SGR/OSC sequence in isolation
- Partial sequence tests: split at every byte boundary
- Screen buffer tests: cursor movement, erase, scroll, wrap
- Color tests: 16, 256, truecolor
- Resize tests: shrink and grow, content preservation
- High-throughput test: 100MB of `yes` output, verify bounded memory
- UTF-8 tests: multi-byte characters, emoji (if supported)
- Regression: known escape sequences from popular terminal programs (vim, tmux patterns)
