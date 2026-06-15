#![no_std]
#![no_main]
#![deny(unsafe_code)]

use libc_lite as _;

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    0
}
