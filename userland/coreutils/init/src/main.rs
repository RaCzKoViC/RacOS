// racinit — RacOS init process (PID 1) — minimal bring-up version.
//
// Responsibility:
//  1. Become session leader.
//  2. Spawn /bin/sh from initramfs (racsh) wired to console.
//  3. Wait for it. If it exits, respawn after a short backoff so PID 1
//     never returns (kernel treats PID 1 exit as fatal).
//  4. Drain orphan zombies non-blockingly between supervises.
//
// Unit-file driven service supervision lives in the `init` library crate
// (engine.rs) and will replace this once enough syscalls are stable.
// During Sprint 2 bring-up we keep this path deliberately small so any
// failure on the "kernel → user mode → racsh" boundary is easy to isolate.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate libc_lite;

const SHELL_PATH: &[u8] = b"/bin/sh\0";

#[allow(unsafe_code)] // C ABI entry point: linker symbol exemption only
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let _ = libc_lite::write(1, b"[init] RacInit starting (PID 1)\n");

    // PID 1 is the session leader. Ignore errors during bring-up.
    let _ = libc_lite::setsid();

    loop {
        match spawn_shell() {
            Ok(pid) => {
                let _ = libc_lite::write(1, b"[init] spawned /bin/sh, waiting...\n");
                wait_for(pid);
                let _ = libc_lite::write(1, b"[init] /bin/sh exited, respawning in 1s\n");
                let _ = libc_lite::nanosleep(1, 0);
            }
            Err(_) => {
                let _ = libc_lite::write(2, b"[init] cannot spawn /bin/sh, retrying in 5s\n");
                let _ = libc_lite::nanosleep(5, 0);
            }
        }
        reap_zombies();
    }
}

fn spawn_shell() -> Result<i32, i64> {
    let argv: [*const u8; 2] = [SHELL_PATH.as_ptr(), core::ptr::null()];
    libc_lite::spawn_args(SHELL_PATH, &argv)
}

fn wait_for(target_pid: i32) {
    // Wait specifically for the shell. Reap any other reparented children
    // in passing and keep waiting for the target.
    loop {
        let mut status: i32 = 0;
        match libc_lite::waitpid(-1, &mut status, 0) {
            Ok(pid) if pid == target_pid => return,
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

fn reap_zombies() {
    loop {
        let mut status: i32 = 0;
        match libc_lite::waitpid(-1, &mut status, libc_lite::WNOHANG) {
            Ok(pid) if pid > 0 => continue,
            _ => return,
        }
    }
}
