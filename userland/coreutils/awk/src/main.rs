#![no_std]
#![no_main]

use libc_lite;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        unsafe { libc_lite::println("awk: missing program") };
        return 1;
    }

    let program = unsafe { arg_name(argv, 1) };
    if program.is_empty() {
        unsafe { libc_lite::println("awk: empty program") };
        return 1;
    }

    // Simple MVP: only '{print}' or '{print $1}' etc.
    let print_all = program == b"{print}";
    let print_field = if program.starts_with(b"{print $") && program.ends_with(b"}") {
        let inner = &program[7..program.len() - 1];
        inner.iter().position(|&b| b == b'}').map(|_| inner)
    } else {
        None
    };

    // Read from stdin
    let mut buf = [0u8; 1024];
    loop {
        match unsafe { libc_lite::read(0, &mut buf) } {
            Ok(0) => break,
            Ok(n) => {
                let data = &buf[..n];
                let lines = data.split(|&b| b == b'\n');
                for line in lines {
                    if line.is_empty() && n > 0 && data.last() != Some(&b'\n') {
                        continue;
                    }
                    process_line(line, print_all, print_field);
                }
            }
            Err(_) => {
                unsafe { libc_lite::println("awk: read error") };
                return 1;
            }
        }
    }

    0
}

fn process_line(line: &[u8], print_all: bool, print_field: Option<&[u8]>) {
    if print_all {
        unsafe { libc_lite::write(1, line) };
        unsafe { libc_lite::write(1, b"\n") };
        return;
    }

    if let Some(field_spec) = print_field {
        if field_spec == b"0" {
            unsafe { libc_lite::write(1, line) };
            unsafe { libc_lite::write(1, b"\n") };
            return;
        }
        // Simple: assume $1, $2, etc.
        if let Ok(field_num) = core::str::from_utf8(field_spec).unwrap_or("").parse::<usize>() {
            let fields: alloc::vec::Vec<&[u8]> = line.split(|&b| b == b' ').collect();
            if field_num > 0 && field_num <= fields.len() {
                unsafe { libc_lite::write(1, fields[field_num - 1]) };
            }
        }
        unsafe { libc_lite::write(1, b"\n") };
    }
}

unsafe fn arg_name(argv: *const *const u8, idx: usize) -> &'static [u8] {
    let ptr = *argv.offset(idx as isize);
    if ptr.is_null() {
        return b"";
    }
    let mut len = 0;
    while *ptr.offset(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}

extern crate alloc;