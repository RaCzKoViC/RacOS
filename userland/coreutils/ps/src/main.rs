#![no_std]
#![no_main]

use libc_lite;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // Simple ps: list processes from /proc
    let proc_dir = b"/proc";
    let fd = match unsafe { libc_lite::open(proc_dir, 0, 0) } {
        Ok(fd) => fd,
        Err(_) => {
            unsafe { libc_lite::println("ps: cannot open /proc") };
            return 1;
        }
    };

    unsafe { libc_lite::println("PID\tPPID\tSTATE\tNAME") };

    let mut buf = [0u8; 4096];
    loop {
        let n = match unsafe { libc_lite::getdents(fd, &mut buf) } {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => {
                unsafe { libc_lite::println("ps: error reading /proc") };
                unsafe { libc_lite::close(fd) };
                return 1;
            }
        };

        let mut offset = 0;
        while offset < n {
            let entry = &buf[offset..];
            if entry.len() < 18 {
                break;
            }
            let ino = u64::from_le_bytes([entry[0], entry[1], entry[2], entry[3], entry[4], entry[5], entry[6], entry[7]]);
            let file_type = entry[16];
            let name_len = entry[17] as usize;
            if name_len == 0 || offset + 18 + name_len > n {
                break;
            }
            let name = &entry[18..18 + name_len];
            if file_type == 4 && name != b"." && name != b".." { // directory
                if let Ok(pid_str) = core::str::from_utf8(name) {
                    if let Ok(pid) = pid_str.parse::<u32>() {
                        print_process_info(pid);
                    }
                }
            }
            offset += 18 + name_len;
        }
    }

    unsafe { libc_lite::close(fd) };
    0
}

fn print_process_info(pid: u32) {
    // Read /proc/<pid>/stat for basic info
    let mut path = [0u8; 32];
    let path_str = format_pid_path(pid, &mut path);
    let fd = match unsafe { libc_lite::open(path_str.as_bytes(), 0, 0) } {
        Ok(fd) => fd,
        Err(_) => return,
    };

    let mut buf = [0u8; 256];
    let n = match unsafe { libc_lite::read(fd, &mut buf) } {
        Ok(n) => n,
        Err(_) => {
            unsafe { libc_lite::close(fd) };
            return;
        }
    };
    unsafe { libc_lite::close(fd) };

    if n == 0 {
        return;
    }

    let stat = unsafe { core::str::from_utf8_unchecked(&buf[..n]) };
    let fields: alloc::vec::Vec<&str> = stat.split_whitespace().collect();
    if fields.len() < 4 {
        return;
    }

    let ppid = fields[3].parse::<u32>().unwrap_or(0);
    let state = fields[2];
    let name = fields[1].trim_matches('(').trim_matches(')');

    unsafe { libc_lite::print(&pid.to_string()) };
    unsafe { libc_lite::print("\t") };
    unsafe { libc_lite::print(&ppid.to_string()) };
    unsafe { libc_lite::print("\t") };
    unsafe { libc_lite::print(state) };
    unsafe { libc_lite::print("\t") };
    unsafe { libc_lite::println(name) };
}

fn format_pid_path(pid: u32, buf: &mut [u8; 32]) -> &str {
    let s = alloc::format!("/proc/{}/stat", pid);
    let bytes = s.as_bytes();
    buf[..bytes.len()].copy_from_slice(bytes);
    unsafe { core::str::from_utf8_unchecked(&buf[..bytes.len()]) }
}

extern crate alloc;