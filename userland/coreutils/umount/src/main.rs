#![no_std]
#![no_main]

use libc_lite::{print, println, umount, write};

fn cstr_len(p: *const u8) -> usize {
    let mut n = 0usize;
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// umount — detach a filesystem from its mount path.
/// Usage: umount <path>
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        println("usage: umount <path>");
        return 1;
    }
    let arg_ptr = unsafe { *argv.add(1) };
    if arg_ptr.is_null() {
        return 1;
    }
    // Need a NUL-terminated slice for the syscall (validate_user_string).
    let len = cstr_len(arg_ptr);
    let slice = unsafe { core::slice::from_raw_parts(arg_ptr, len + 1) };

    match umount(slice) {
        Ok(()) => 0,
        Err(_) => {
            print("umount: failed for ");
            let _ = write(2, &slice[..len]);
            print("\n");
            1
        }
    }
}
