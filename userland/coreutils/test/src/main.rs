//! racos-test — Userland system test suite
//!
//! Runs assertions against kernel syscalls and I/O.
//! Exit code 0 = all tests passed, non-zero = failure count.

#![no_std]
#![no_main]

use libc_lite::*;

static mut PASS: u32 = 0;
static mut FAIL: u32 = 0;

const O_RDWR: u32 = 0x0002;
const O_CREAT: u32 = 0x0040;
const O_TRUNC: u32 = 0x0200;
const SIGTERM: i32 = 15;
const TIOCGWINSZ: u32 = 0x5413;
const TIOCSWINSZ: u32 = 0x5414;
const TIOCGPGRP: u32 = 0x540F;
const TIOCSPGRP: u32 = 0x5410;
const EXEC_LOOP_ITERS: u32 = 50;
const MEMFREE_LEAK_TOLERANCE_KB: u32 = 256;
const POLL_TIMEOUT_MS: i32 = 25;
const POLL_TIMEOUT_MIN_MS: u64 = 15;

#[repr(C)]
struct StatBuf {
    st_dev: u64,
    st_ino: u64,
    st_mode: u32,
    st_nlink: u32,
    st_uid: u32,
    st_gid: u32,
    st_size: u64,
    st_atime: u64,
    st_mtime: u64,
    st_ctime: u64,
    st_rdev_major: u32,
    st_rdev_minor: u32,
}

macro_rules! check {
    ($name:expr, $cond:expr) => {
        if $cond {
            unsafe {
                PASS += 1;
            }
            print("  [PASS] ");
            println($name);
        } else {
            unsafe {
                FAIL += 1;
            }
            print("  [FAIL] ");
            println($name);
        }
    };
}

fn print_u32(n: u32) {
    if n == 0 {
        print("0");
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    let mut v = n;
    while v > 0 {
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        let _ = write(1, &buf[i..i + 1]);
    }
}

fn print_i32(n: i32) {
    if n < 0 {
        print("-");
        print_u32(n.wrapping_neg() as u32);
    } else {
        print_u32(n as u32);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    println("=== RacOS System Test Suite ===");

    test_getpid();
    test_write_stdout();
    test_write_stderr();
    test_open_close();
    test_dev_null();
    test_dev_zero();
    test_open_nonexistent();
    test_pipe();
    test_dup();
    test_poll_timeout();
    test_spawn_wait();
    test_signal_default_terminate();
    test_sigchld_waitpid();
    test_exec_loop_memory_cleanup();
    test_tty_ioctl_state();
    test_chdir_getcwd();
    test_security_syscalls();

    println("");
    let (pass, fail) = unsafe { (PASS, FAIL) };
    print("=== Results: ");
    print_u32(pass);
    print(" passed, ");
    print_u32(fail);
    println(" failed ===");

    if fail > 0 {
        1
    } else {
        0
    }
}

// ─────────────────────────────────────────────────
// Test functions
// ─────────────────────────────────────────────────

fn test_getpid() {
    println("\n[test] getpid");
    let pid = getpid();
    check!("getpid returns > 0", pid > 0);
}

fn test_write_stdout() {
    println("\n[test] write(stdout)");
    let n = write(1, b"test output\n");
    check!("write returns Ok", n.is_ok());
    check!("write returns correct count", n.unwrap_or(0) == 12);
}

fn test_write_stderr() {
    println("\n[test] write(stderr)");
    let n = write(2, b"stderr test\n");
    check!("write(2) returns Ok", n.is_ok());
}

fn test_open_close() {
    println("\n[test] open/close");
    let fd = open(b"/dev/null\0", 0, 0);
    check!("open /dev/null succeeds", fd.is_ok());
    if let Ok(fd) = fd {
        let ret = close(fd);
        check!("close returns Ok", ret.is_ok());
    }
}

fn test_dev_null() {
    println("\n[test] /dev/null read/write");
    let fd = open(b"/dev/null\0", 2, 0); // O_RDWR
    check!("open /dev/null O_RDWR", fd.is_ok());
    if let Ok(fd) = fd {
        let n = write(fd, b"discarded");
        check!("write to /dev/null Ok", n.is_ok());
        check!("write to /dev/null count=9", n.unwrap_or(0) == 9);

        let mut buf = [0u8; 16];
        let n = read(fd, &mut buf);
        check!("/dev/null read returns 0 (EOF)", n.unwrap_or(99) == 0);

        let _ = close(fd);
    }
}

fn test_dev_zero() {
    println("\n[test] /dev/zero read");
    let fd = open(b"/dev/zero\0", 0, 0);
    check!("open /dev/zero", fd.is_ok());
    if let Ok(fd) = fd {
        let mut buf = [0xFFu8; 8];
        let n = read(fd, &mut buf);
        check!("/dev/zero read returns 8", n.unwrap_or(0) == 8);
        check!("/dev/zero data is all zeros", buf.iter().all(|&b| b == 0));
        let _ = close(fd);
    }
}

fn test_open_nonexistent() {
    println("\n[test] open nonexistent file");
    let fd = open(b"/no/such/file\0", 0, 0);
    check!("open nonexistent returns Err", fd.is_err());
}

fn test_pipe() {
    println("\n[test] pipe");
    let mut fds = [0i32; 2];
    let ret = pipe(&mut fds);
    check!("pipe() returns Ok", ret.is_ok());
    if ret.is_ok() {
        let n = write(fds[1], b"hello pipe");
        check!("pipe write returns 10", n.unwrap_or(0) == 10);

        let mut buf = [0u8; 32];
        let n = read(fds[0], &mut buf);
        check!("pipe read returns 10", n.unwrap_or(0) == 10);
        check!("pipe data matches", &buf[..10] == b"hello pipe");

        let _ = close(fds[0]);
        let _ = close(fds[1]);
    }
}

fn test_dup() {
    println("\n[test] dup/dup2");
    let fd = open(b"/dev/null\0", 1, 0); // O_WRONLY
    check!("open for dup", fd.is_ok());
    if let Ok(fd) = fd {
        let fd2 = dup(fd);
        check!("dup returns new fd", fd2.is_ok());
        if let Ok(fd2) = fd2 {
            check!("dup fd differs", fd2 != fd);
            let n = write(fd2, b"dup test");
            check!("write via dup'd fd", n.is_ok());
            let _ = close(fd2);
        }

        let fd3 = dup2(fd, 10);
        check!("dup2 returns target fd", fd3.unwrap_or(-1) == 10);
        if fd3.is_ok() {
            let _ = close(10);
        }
        let _ = close(fd);
    }
}

fn test_poll_timeout() {
    println("\n[test] poll timeout");

    let before = monotonic_ms();
    check!("clock_gettime before poll", before.is_some());
    let before = match before {
        Some(value) => value,
        None => return,
    };

    let mut fds: [PollFd; 0] = [];
    let ret = poll(&mut fds, POLL_TIMEOUT_MS);
    check!("poll([]) returns Ok", ret.is_ok());
    check!("poll([]) returns timeout", ret.unwrap_or(-1) == 0);

    let after = monotonic_ms();
    check!("clock_gettime after poll", after.is_some());
    if let Some(after) = after {
        let elapsed = after.saturating_sub(before);
        print("  poll elapsed=");
        print_u32(elapsed as u32);
        println(" ms");
        check!(
            "poll([]) waits at least minimum timeout",
            elapsed >= POLL_TIMEOUT_MIN_MS
        );
        if ret.unwrap_or(-1) == 0 && elapsed >= POLL_TIMEOUT_MIN_MS {
            println("POLL-TIMEOUT-OK");
        }
    }
}

fn test_spawn_wait() {
    println("\n[test] spawn/wait");
    let pid = spawn(b"/bin/true\0");
    check!("spawn /bin/true returns Ok", pid.is_ok());
    if pid.is_ok() {
        let mut status: i32 = -1;
        let ret = wait(&mut status);
        check!("wait returns child pid", ret.is_ok());
        check!("child exit status is 0", status == 0);
    }
}

fn monotonic_ms() -> Option<u64> {
    let mut ts = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    match clock_gettime(CLOCK_MONOTONIC, &mut ts) {
        Ok(()) => Some(
            ts.tv_sec
                .saturating_mul(1000)
                .saturating_add(ts.tv_nsec / 1_000_000),
        ),
        Err(_) => None,
    }
}

fn test_signal_default_terminate() {
    println("\n[test] signal default termination");

    let path = b"/bin/sleep\0";
    let arg0 = b"sleep\0";
    let arg1 = b"5\0";
    let argv: [*const u8; 3] = [arg0.as_ptr(), arg1.as_ptr(), core::ptr::null()];

    let pid = spawn_args(path, &argv);
    check!("spawn /bin/sleep returns Ok", pid.is_ok());
    if let Ok(pid) = pid {
        let killed = kill(pid, SIGTERM);
        check!("kill(SIGTERM) returns Ok", killed.is_ok());

        let mut status: i32 = -99;
        let waited = waitpid(pid, &mut status, 0);
        check!(
            "waitpid returns signalled child",
            waited.unwrap_or(-1) == pid
        );
        check!("SIGTERM default action exits non-zero", status != 0);
        if waited.is_ok() && status != 0 {
            println("PHASE21-SIGNAL-TERM-OK");
        }
    }
}

fn test_sigchld_waitpid() {
    println("\n[test] SIGCHLD wait wakeup");

    let path = b"/bin/sleep\0";
    let arg0 = b"sleep\0";
    let arg1 = b"5\0";
    let argv: [*const u8; 3] = [arg0.as_ptr(), arg1.as_ptr(), core::ptr::null()];

    let pid = spawn_args(path, &argv);
    check!("spawn child for SIGCHLD test", pid.is_ok());
    if let Ok(pid) = pid {
        let mut status: i32 = -99;
        let before = waitpid(pid, &mut status, WNOHANG);
        check!(
            "waitpid(WNOHANG) reports running child",
            before.unwrap_or(-1) == 0
        );

        let killed = kill(pid, SIGTERM);
        check!("kill child for SIGCHLD test", killed.is_ok());

        let waited = waitpid(pid, &mut status, 0);
        check!(
            "blocking waitpid wakes for child",
            waited.unwrap_or(-1) == pid
        );
        check!("SIGCHLD wait status is non-zero", status != 0);
        if waited.is_ok() && status != 0 {
            println("PHASE21-SIGCHLD-WAIT-OK");
        }
    }
}

fn test_exec_loop_memory_cleanup() {
    println("\n[test] exec loop memory cleanup");

    let before = read_memfree_kb();
    check!("read /proc/meminfo before loop", before.is_some());
    let before = match before {
        Some(v) => v,
        None => return,
    };

    let mut all_ok = true;
    let mut last_status: i32 = 0;
    for _ in 0..EXEC_LOOP_ITERS {
        let pid = spawn(b"/bin/true\0");
        if let Ok(pid) = pid {
            let mut status: i32 = -1;
            last_status = status;
            match waitpid(pid, &mut status, 0) {
                Ok(waited) if waited == pid && status == 0 => last_status = status,
                Ok(_) => {
                    last_status = status;
                    all_ok = false;
                }
                _ => all_ok = false,
            }
        } else {
            last_status = -999;
            all_ok = false;
        }
    }

    let after = read_memfree_kb();
    check!("read /proc/meminfo after loop", after.is_some());
    let after = match after {
        Some(v) => v,
        None => return,
    };

    let leaked = before.saturating_sub(after);
    print("  MemFree before=");
    print_u32(before);
    print(" kB after=");
    print_u32(after);
    print(" kB leaked=");
    print_u32(leaked);
    println(" kB");

    check!("exec loop children exit cleanly", all_ok);
    check!(
        "exec loop memory delta within tolerance",
        leaked <= MEMFREE_LEAK_TOLERANCE_KB
    );
    if all_ok && leaked <= MEMFREE_LEAK_TOLERANCE_KB {
        println("PHASE21-EXEC-LOOP-OK");
    } else {
        print("  exec-loop status before=");
        print_u32(before);
        print(" after=");
        print_u32(after);
        print(" leaked=");
        print_u32(leaked);
        print(" last_status=");
        print_i32(last_status);
        println("");
    }
}

fn read_memfree_kb() -> Option<u32> {
    let fd = open(b"/proc/meminfo\0", 0, 0).ok()?;
    let mut buf = [0u8; 256];
    let n = read(fd, &mut buf).ok()?;
    let _ = close(fd);
    parse_memfree_kb(&buf[..n])
}

fn parse_memfree_kb(buf: &[u8]) -> Option<u32> {
    let key = b"MemFree:";
    let mut i = 0usize;
    while i + key.len() <= buf.len() {
        if &buf[i..i + key.len()] == key {
            let mut j = i + key.len();
            while j < buf.len() && (buf[j] == b' ' || buf[j] == b'\t') {
                j += 1;
            }
            let mut value = 0u32;
            let mut saw_digit = false;
            while j < buf.len() && buf[j] >= b'0' && buf[j] <= b'9' {
                value = value
                    .saturating_mul(10)
                    .saturating_add((buf[j] - b'0') as u32);
                saw_digit = true;
                j += 1;
            }
            return if saw_digit { Some(value) } else { None };
        }
        i += 1;
    }
    None
}

fn test_tty_ioctl_state() {
    println("\n[test] TTY ioctl state");

    let master_fd = open(b"/dev/ptmx\0", O_RDWR, 0);
    check!("open /dev/ptmx", master_fd.is_ok());
    let slave_fd = open(b"/dev/pts0\0", O_RDWR, 0);
    check!("open /dev/pts0", slave_fd.is_ok());

    let (master_fd, slave_fd) = match (master_fd, slave_fd) {
        (Ok(master_fd), Ok(slave_fd)) => (master_fd, slave_fd),
        (Ok(master_fd), Err(_)) => {
            let _ = close(master_fd);
            return;
        }
        (Err(_), Ok(slave_fd)) => {
            let _ = close(slave_fd);
            return;
        }
        (Err(_), Err(_)) => return,
    };

    check!("isatty(/dev/ptmx)", isatty(master_fd));
    check!("isatty(/dev/pts0)", isatty(slave_fd));

    let bad_ws = [0u16, 80u16];
    let bad_resize = ioctl(master_fd, TIOCSWINSZ, bad_ws.as_ptr() as u64);
    check!("TIOCSWINSZ rejects zero rows", bad_resize.is_err());

    let new_ws = [40u16, 100u16];
    let resize = ioctl(master_fd, TIOCSWINSZ, new_ws.as_ptr() as u64);
    check!("TIOCSWINSZ on /dev/ptmx", resize.is_ok());

    let mut got_ws = [0u16; 2];
    let get_ws = ioctl(slave_fd, TIOCGWINSZ, got_ws.as_mut_ptr() as u64);
    check!("TIOCGWINSZ on /dev/pts0", get_ws.is_ok());
    check!("winsize round-trip rows", got_ws[0] == new_ws[0]);
    check!("winsize round-trip cols", got_ws[1] == new_ws[1]);

    let pgid = getpgid(0).unwrap_or(0);
    check!("getpgid(0) for TIOCSPGRP", pgid > 0);
    let set_fg = ioctl(slave_fd, TIOCSPGRP, &pgid as *const u32 as u64);
    check!("TIOCSPGRP on /dev/pts0", set_fg.is_ok());

    let mut got_pgid = 0u32;
    let get_fg = ioctl(master_fd, TIOCGPGRP, &mut got_pgid as *mut u32 as u64);
    check!("TIOCGPGRP on /dev/ptmx", get_fg.is_ok());
    check!("foreground pgid round-trip", got_pgid == pgid);

    let null_fd = open(b"/dev/null\0", O_RDWR, 0);
    check!(
        "open /dev/null for TTY ioctl negative checks",
        null_fd.is_ok()
    );
    let mut non_tty_ws = [0u16; 2];
    let mut non_tty_pgid = pgid;
    let mut non_tty_rejected = false;
    if let Ok(null_fd) = null_fd {
        let get_ws_non_tty = ioctl(null_fd, TIOCGWINSZ, non_tty_ws.as_mut_ptr() as u64);
        let set_ws_non_tty = ioctl(null_fd, TIOCSWINSZ, new_ws.as_ptr() as u64);
        let get_fg_non_tty = ioctl(null_fd, TIOCGPGRP, &mut non_tty_pgid as *mut u32 as u64);
        let set_fg_non_tty = ioctl(null_fd, TIOCSPGRP, &pgid as *const u32 as u64);
        check!("TIOCGWINSZ rejects /dev/null", get_ws_non_tty.is_err());
        check!("TIOCSWINSZ rejects /dev/null", set_ws_non_tty.is_err());
        check!("TIOCGPGRP rejects /dev/null", get_fg_non_tty.is_err());
        check!("TIOCSPGRP rejects /dev/null", set_fg_non_tty.is_err());
        check!("isatty(/dev/null) is false", !isatty(null_fd));
        non_tty_rejected = get_ws_non_tty.is_err()
            && set_ws_non_tty.is_err()
            && get_fg_non_tty.is_err()
            && set_fg_non_tty.is_err()
            && !isatty(null_fd);
        let _ = close(null_fd);
    }

    let _ = close(slave_fd);
    let _ = close(master_fd);
    let closed_master_isatty = isatty(master_fd);
    check!(
        "isatty(closed /dev/ptmx fd) is false",
        !closed_master_isatty
    );

    if resize.is_ok()
        && get_ws.is_ok()
        && got_ws == new_ws
        && set_fg.is_ok()
        && got_pgid == pgid
        && non_tty_rejected
        && !closed_master_isatty
    {
        println("TTY-IOCTL-OK");
    }
}

fn test_chdir_getcwd() {
    println("\n[test] chdir/getcwd");
    let ret = chdir(b"/dev\0");
    check!("chdir /dev returns Ok", ret.is_ok());

    let mut buf = [0u8; 128];
    let len = getcwd(&mut buf);
    check!("getcwd returns Ok", len.is_ok());
    if let Ok(len) = len {
        check!("getcwd length > 0", len > 0);
        check!("cwd is /dev", &buf[..len] == b"/dev");
    }

    let _ = chdir(b"/\0");
}

fn test_security_syscalls() {
    println("\n[test] security syscalls (Phase C)");

    let uid = getuid();
    let euid = geteuid();
    let gid = getgid();
    let egid = getegid();
    check!("uid==euid", uid == euid);
    check!("gid==egid", gid == egid);

    let old_mask = umask(0o027);
    let prev = umask(old_mask);
    check!("umask returns previous mask", prev == 0o027);

    let path = b"/tmp/sec_perm_test\0";
    let fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0o666);
    check!("open O_CREAT security test file", fd.is_ok());
    if let Ok(fd) = fd {
        let _ = write(fd, b"sec");
        let _ = close(fd);
    } else {
        return;
    }

    let chmod_ret = chmod(path, 0o600);
    check!("chmod 0600 returns Ok", chmod_ret.is_ok());

    let access_r = access(path, R_OK);
    let access_w = access(path, W_OK);
    let access_x = access(path, X_OK);
    check!("access R_OK after chmod", access_r.is_ok());
    check!("access W_OK after chmod", access_w.is_ok());
    check!("access X_OK denied after chmod 0600", access_x.is_err());

    let chown_ret = chown(path, uid, gid);
    check!("chown to current uid/gid returns Ok", chown_ret.is_ok());

    let mut raw = [0u8; 80];
    let st_ret = stat(path, &mut raw);
    check!("stat security file returns Ok", st_ret.is_ok());
    if st_ret.is_ok() {
        let st = unsafe { &*(raw.as_ptr() as *const StatBuf) };
        check!("stat mode low bits == 0600", (st.st_mode & 0o777) == 0o600);
        check!("stat uid matches", st.st_uid == uid);
        check!("stat gid matches", st.st_gid == gid);
    }

    let _ = unlink(path);
}
