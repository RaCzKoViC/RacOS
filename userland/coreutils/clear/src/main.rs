// clear — blank the terminal and home the cursor.
//
// racsh has always bound Ctrl-L to this, but `clear` is muscle memory and its
// absence reads as a broken system rather than a missing convenience.
//
// Emits the same two CSI sequences every terminal understands, in a single
// write: ED 2 (erase entire display) then CUP with no arguments (cursor to
// 1,1). One write matters here — RacTerm and the framebuffer console parse CSI
// per write, so splitting the sequence would print the tail literally.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate libc_lite;

/// ESC [ 2 J   erase whole screen
/// ESC [ H     cursor home
const CLEAR: &[u8] = b"\x1B[2J\x1B[H";

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    match libc_lite::write(1, CLEAR) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}
