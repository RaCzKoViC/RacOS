#![no_std]
#![no_main]
#![deny(unsafe_code)]

use libc_lite;

/// env — print environment (stub: prints PWD and PATH from getcwd).
/// In RacOS userland, env vars aren't inherited yet, so this is minimal.
#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // Print PWD
    let mut buf = [0u8; 256];
    if let Ok(n) = libc_lite::getcwd(&mut buf) {
        let _ = libc_lite::write(1, b"PWD=");
        let _ = libc_lite::write(1, &buf[..n]);
        let _ = libc_lite::write(1, b"\n");
    }
    // Print PATH (hardcoded default since env inheritance not yet implemented)
    let _ = libc_lite::write(1, b"PATH=/bin:/sbin\n");
    0
}
