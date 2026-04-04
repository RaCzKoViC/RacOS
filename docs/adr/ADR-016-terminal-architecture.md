# ADR-016: Terminal Architecture (RacTerm)

**Status**: Accepted
**Date**: 2026-04-04

## Context

A terminal emulator is needed to provide the visual interface for racsh. The architecture must separate parsing, state management, and rendering to ensure testability and maintainability.

## Decision

RacTerm has strictly separated layers:
1. **Input decoder** — keyboard/mouse events to byte sequences
2. **Escape sequence parser** — state machine processing CSI/SGR/OSC sequences
3. **Screen buffer** — cell grid (character + attributes) with dirty flags
4. **Cursor state machine** — position, visibility, saved positions
5. **Renderer** — reads screen buffer, outputs to display (dirty regions only)
6. **PTY session binding** — connects to shell via PTY master fd

Supported capabilities: ANSI/VT100 subset (CSI cursor movement, SGR colors/attributes, ED/EL erase, scroll regions, alternate screen buffer).

## Consequences

- Parser can be unit-tested without rendering infrastructure
- Screen buffer is a pure data structure, testable in isolation
- Dirty region rendering avoids full-screen repaint
- Scrollback uses ring buffer with configurable limit (default 10K lines)
- Partial escape sequences buffered with 100ms timeout

## Risks

- Missing escape sequences break applications (mitigate: implement VT100 core + test against vim/tmux patterns)
- High-throughput output may cause memory pressure (mitigate: bounded scrollback, test with large output)

## Rollback

Individual renderer backends can be swapped (serial, framebuffer, GUI window) without changing parser or screen buffer.
