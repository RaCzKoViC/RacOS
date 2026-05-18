#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        unsafe { libc_lite::println("Usage: rpkg install <file.rpk>") };
        return 1;
    }

    // Simple argv parsing - assume safe
    let cmd_ptr = unsafe { *argv.offset(1) };
    if cmd_ptr.is_null() {
        return 1;
    }
    let cmd = unsafe { cstr_to_str(cmd_ptr) };
    let cmd = match cmd {
        Some(s) => s,
        None => return 1,
    };

    match cmd {
        "install" => {
            if argc < 3 {
                unsafe { libc_lite::println("Usage: rpkg install <file.rpk>") };
                return 1;
            }
            let file_ptr = unsafe { *argv.offset(2) };
            if file_ptr.is_null() {
                return 1;
            }
            let file_path = unsafe { cstr_to_str(file_ptr) };
            let file_path = match file_path {
                Some(s) => s,
                None => return 1,
            };
            cmd_install(file_path)
        }
        _ => {
            unsafe { libc_lite::println("Unknown command") };
            1
        }
    }
}

unsafe fn cstr_to_str(ptr: *const u8) -> Option<&'static str> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.offset(len) != 0 {
        len += 1;
        if len > 1024 { // safety limit
            return None;
        }
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8(slice).ok()
}

fn cmd_install(file_path: &str) -> i32 {
    // Read the .rpk file
    let fd = match unsafe { libc_lite::open(file_path.as_bytes(), 0, 0) } {
        Ok(fd) => fd,
        Err(_) => {
            unsafe { libc_lite::println("Failed to open file") };
            return 1;
        }
    };

    // Get file size (assume small for MVP)
    let mut stat_buf = [0u8; 80];
    if unsafe { libc_lite::fstat(fd, &mut stat_buf) }.is_err() {
        unsafe { libc_lite::close(fd) };
        unsafe { libc_lite::println("Failed to stat file") };
        return 1;
    }
    let size = u64::from_le_bytes([
        stat_buf[48], stat_buf[49], stat_buf[50], stat_buf[51],
        stat_buf[52], stat_buf[53], stat_buf[54], stat_buf[55],
    ]);

    if size > 1024 * 1024 { // 1MB limit
        unsafe { libc_lite::close(fd) };
        unsafe { libc_lite::println("File too large") };
        return 1;
    }

    let mut buf = alloc::vec![0u8; size as usize];
    let n = match unsafe { libc_lite::read(fd, &mut buf) } {
        Ok(n) => n,
        Err(_) => {
            unsafe { libc_lite::close(fd) };
            unsafe { libc_lite::println("Failed to read file") };
            return 1;
        }
    };
    unsafe { libc_lite::close(fd) };

    if n != size as usize {
        unsafe { libc_lite::println("Incomplete read") };
        return 1;
    }

    // Parse and show install plan
    match rpkg::build_install_plan(&buf, file_path, "/var/lib/rpkg") {
        Ok(plan) => {
            unsafe { libc_lite::println("Install plan:") };
            unsafe { libc_lite::print("Package: ") };
            if let Some(ref name) = plan.package_name {
                unsafe { libc_lite::print(name) };
            } else {
                unsafe { libc_lite::print("(unknown)") };
            }
            unsafe { libc_lite::print(" ") };
            if let Some(ref ver) = plan.package_version {
                unsafe { libc_lite::print(ver) };
            } else {
                unsafe { libc_lite::print("(unknown)") };
            }
            unsafe { libc_lite::println("") };
            unsafe { libc_lite::print("Source: ") };
            unsafe { libc_lite::println(&plan.source_file) };
            unsafe { libc_lite::print("DB dir: ") };
            unsafe { libc_lite::println(&plan.info_dir) };
            0
        }
        Err(_) => {
            unsafe { libc_lite::println("Invalid .rpk file") };
            1
        }
    }
}

extern crate libc_lite;
extern crate rpkg;