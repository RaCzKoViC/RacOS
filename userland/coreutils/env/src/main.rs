#![no_std]
#![no_main]

/// env — print the inherited environment, one `KEY=VALUE` line per entry.
///
/// Walks the `environ` block that libc-lite's `_start` recorded from the
/// argv/envp layout the kernel placed on this process's stack
/// (kernel/src/task/process.rs). If no environment was inherited (e.g.
/// invoked directly by the kernel with no envp), prints nothing and
/// exits 0.
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let mut p = libc_lite::environ();
    if p.is_null() {
        return 0;
    }
    loop {
        let entry = unsafe { *p };
        if entry.is_null() {
            break;
        }
        let mut len = 0usize;
        while unsafe { *entry.add(len) } != 0 {
            len += 1;
        }
        let bytes = unsafe { core::slice::from_raw_parts(entry, len) };
        let _ = libc_lite::write(1, bytes);
        let _ = libc_lite::write(1, b"\n");
        p = unsafe { p.add(1) };
    }
    0
}
