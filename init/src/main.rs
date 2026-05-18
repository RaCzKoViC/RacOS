// RacInit (PID 1) — user-space init for RacOS.
//
// Responsibility:
//  1. Spawn /bin/sh from initramfs (racsh).
//  2. Loop reaping zombies; if the shell exits, respawn it
//     so PID 1 never exits (kernel panic on PID 1 exit).
//
// This is the minimal "wire racsh from initramfs" path. Service
// management (units, dependency graph) lives in the `racinit`
// library crate and will be wired in later, once syscalls for
// directory iteration / signals are stable.

#![no_std]
#![no_main]

extern crate libc_lite;

const SHELL_PATH: &[u8] = b"/bin/sh\0";

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    let _ = libc_lite::write(1, b"[init] RacInit v0.1.0 starting (PID 1)\n");

    loop {
        match spawn_shell() {
            Ok(pid) => {
                let _ = libc_lite::write(1, b"[init] spawned /bin/sh, waiting...\n");
                wait_for_child(pid);
                let _ = libc_lite::write(1, b"[init] /bin/sh exited, restarting in 1s\n");
                let _ = libc_lite::nanosleep(1, 0);
            }
            Err(_) => {
                // Could not spawn — back off and retry. Avoid tight loop
                // that would saturate the scheduler if /bin/sh is missing.
                let _ = libc_lite::write(2, b"[init] failed to spawn /bin/sh, retrying in 5s\n");
                let _ = libc_lite::nanosleep(5, 0);
            }
        }

        reap_zombies();
    }
}

fn spawn_shell() -> Result<i32, i64> {
    // argv = ["/bin/sh", NULL] — racsh reads PATH from its own env.
    let argv: [*const u8; 2] = [SHELL_PATH.as_ptr(), core::ptr::null()];
    libc_lite::spawn_args(SHELL_PATH, &argv)
}

fn wait_for_child(target_pid: i32) {
    // Wait specifically for the shell. Any other children reaped along the
    // way (orphaned, reparented to init) are silently discarded.
    loop {
        let mut status: i32 = 0;
        match libc_lite::waitpid(-1, &mut status, 0) {
            Ok(pid) if pid == target_pid => return,
            Ok(_) => {
                // Reaped some other child; keep waiting for the shell.
                continue;
            }
            Err(_) => {
                // ECHILD or other error — nothing more to wait on.
                return;
            }
        }
    }
}

fn reap_zombies() {
    // Drain any pending zombies non-blockingly.
    loop {
        let mut status: i32 = 0;
        match libc_lite::waitpid(-1, &mut status, libc_lite::WNOHANG) {
            Ok(pid) if pid > 0 => continue,
            _ => return,
        }
    }
}
