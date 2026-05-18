#![no_std]
#![no_main]

use libc_lite::{open, close, getdents, println, print, exit};

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // Determine target path from argv or default to "."
    let path: &[u8] = if argc > 1 {
        let arg_ptr = unsafe { *argv.add(1) };
        if !arg_ptr.is_null() {
            let mut len = 0;
            unsafe { while *arg_ptr.add(len) != 0 { len += 1; } }
            unsafe { core::slice::from_raw_parts(arg_ptr, len + 1) } // includes null
        } else {
            b".\0"
        }
    } else {
        b".\0"
    };
    let fd = match open(path, 0, 0) {
        Ok(fd) => fd,
        Err(_) => match open(b"/\0", 0, 0) {
            Ok(fd) => fd,
            Err(_) => {
                println("ls: cannot open directory");
                exit(1);
            }
        },
    };

    let mut buf = [0u8; 4096];
    match getdents(fd, &mut buf) {
        Ok(nbytes) => {
            let mut offset = 0;
            while offset + 10 <= nbytes {
                // Parse entry: ino(8) + type(1) + name_len(1) + name(N)
                let file_type = buf[offset + 8];
                let name_len = buf[offset + 9] as usize;
                if offset + 10 + name_len > nbytes {
                    break;
                }
                let name = &buf[offset + 10..offset + 10 + name_len];

                // Type indicator
                if file_type == 2 {
                    // Directory
                    print(unsafe { core::str::from_utf8_unchecked(name) });
                    println("/");
                } else {
                    // Regular file or other
                    println(unsafe { core::str::from_utf8_unchecked(name) });
                }

                offset += 10 + name_len;
            }
        }
        Err(_) => {
            println("ls: cannot read directory");
        }
    }

    let _ = close(fd);
    0
}
