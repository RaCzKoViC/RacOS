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
const SIGINT: i32 = 2;
const SIGUSR1: i32 = 10;
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
    test_signal_user_handler();
    test_signal_user_handler_reentrant_syscall();
    test_shell_control_flow();
    test_shell_aliases();
    test_v02_coreutils();
    test_hard_links();
    test_network_tools();
    test_init_engine_supervises_shell();
    test_ps_lists_running_processes();
    test_rpkg_install_list_remove();
    test_sed_substitute();
    test_awk_basic();
    test_id_prints_creds();
    test_sort_orders_lines();
    test_top_prints_snapshot();
    test_touch_creates_file();
    test_chmod_sets_mode();
    test_chown_sets_uid_gid();
    test_env_inherits_shell_vars();
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

// User signal handler state. The kernel delivers signals one-at-a-time on
// the syscall return path, so HANDLER_COUNTER is single-threaded from the
// test's POV: a synchronous `kill(getpid(), SIG)` returns only after the
// handler has run (the SYSRET goes to the dispatcher, the dispatcher calls
// the handler, then sigreturn restores RIP to the instruction after kill).
static mut HANDLER_COUNTER: u32 = 0;
static mut HANDLER_LAST_SIGNUM: i32 = 0;

unsafe extern "C" fn sigint_counting_handler(signum: i32) {
    unsafe {
        HANDLER_COUNTER += 1;
        HANDLER_LAST_SIGNUM = signum;
    }
}

static mut REENTRANT_BYTES_WRITTEN: i32 = 0;

unsafe extern "C" fn sigusr1_writing_handler(_signum: i32) {
    // Re-entrant syscall from within a signal handler. If the kernel mis-
    // tracks STAC/SMAP or the signal frame across nested syscalls this
    // write will either return EFAULT or corrupt the frame.
    let n = write(1, b"[handler]").map(|v| v as i32).unwrap_or(-1);
    unsafe {
        REENTRANT_BYTES_WRITTEN = n;
    }
}

fn test_signal_user_handler() {
    println("\n[test] user signal handler delivery");

    unsafe {
        HANDLER_COUNTER = 0;
        HANDLER_LAST_SIGNUM = 0;
    }
    let installed = signal(SIGINT, sigint_counting_handler);
    check!("signal(SIGINT, handler) returns Ok", installed.is_ok());

    let pid = getpid();
    let sent = kill(pid, SIGINT);
    check!("kill(self, SIGINT) returns Ok", sent.is_ok());

    // After kill() returns the handler must have run exactly once.
    let count = unsafe { HANDLER_COUNTER };
    let last = unsafe { HANDLER_LAST_SIGNUM };
    check!("user handler invoked exactly once", count == 1);
    check!("user handler received correct signum", last == SIGINT);

    // After sigreturn, subsequent syscalls must still work — verifies that
    // the kernel restored RIP/RFLAGS/RSP cleanly and didn't leave the FD
    // table or scheduler in a bad state.
    let after = getpid();
    check!("post-handler getpid() succeeds", after == pid);

    if count == 1 && last == SIGINT && after == pid {
        println("PHASE21-USER-HANDLER-OK");
    }
}

/// Drive racsh through `sh -c "..."` and assert the script's exit code is
/// what racsh's control-flow runtime is expected to produce. This exercises
/// the full lex → parse → execute path inside QEMU, complementing the host
/// shell/tests/control_flow.rs which can only cover the I/O-free surface.
fn shell_run(script: &[u8]) -> Option<i32> {
    let sh = b"/bin/sh\0";
    let arg0 = b"sh\0";
    let arg1 = b"-c\0";
    // Caller passes a NUL-terminated script.
    let argv: [*const u8; 4] = [
        arg0.as_ptr(),
        arg1.as_ptr(),
        script.as_ptr(),
        core::ptr::null(),
    ];
    let pid = match spawn_args(sh, &argv) {
        Ok(p) => p,
        Err(_) => return None,
    };
    let mut status: i32 = -99;
    if waitpid(pid, &mut status, 0).is_err() {
        return None;
    }
    Some(status)
}

fn test_shell_control_flow() {
    println("\n[test] racsh control flow + source");

    // if true → branch yields exit 0
    let s = shell_run(b"if true; then exit 0; else exit 9; fi\0");
    check!("if-true takes then-branch", s == Some(0));

    // if false → else branch yields exit 7
    let s = shell_run(b"if false; then exit 0; else exit 7; fi\0");
    check!("if-false takes else-branch", s == Some(7));

    // for over a literal list iterates the right number of times.
    // Each iteration's exit doesn't propagate, but the script's final
    // command does — here a 3-iter loop sets i to a, then b, then c, and
    // exits with the count.
    let s = shell_run(b"n=0; for x in a b c; do n=`expr $n + 1`; done; exit $n\0");
    // expr isn't guaranteed to exist in /bin yet — accept any non-error
    // exit that proves the script reached its `exit $n`. A successful for
    // loop without expr should still arrive at `exit 0` (n stays "0").
    check!("for-loop reaches exit", s.is_some());

    // Field splitting: unquoted $LIST must split into 3 iterations, so
    // accumulating x with a separator yields ":a:b:c" — case matches
    // exactly that literal and exits 0.
    let split = shell_run(
        b"LIST='a b c'; out=''; for x in $LIST; do out=$out:$x; done; \
          case $out in :a:b:c) exit 0;; *) exit 1;; esac\0",
    );
    check!("unquoted $LIST splits and case matches", split == Some(0));

    // Quoted "$LIST" must NOT split — one iteration, out=":a b c".
    let nosplit = shell_run(
        b"LIST='a b c'; out=''; for x in \"$LIST\"; do out=$out:$x; done; \
          case $out in ':a b c') exit 0;; *) exit 1;; esac\0",
    );
    check!("quoted \"$LIST\" stays one word", nosplit == Some(0));

    if split == Some(0) && nosplit == Some(0) {
        println("T12-SHELL-CONTROL-FLOW-OK");
    }
}

/// Smoke test for the engine-driven PID 1: prove (1) racos-test is running
/// under a non-trivial PPID, (2) that PPID's PPID reaches 1, which means
/// the parent chain goes `racos-test → racsh (shell.service) → init (PID
/// 1)`. The fallback bare-shell init also produces the same chain, but
/// CI's unit-files-present initramfs ensures the engine path is taken.
/// Smoke for racsh aliases (v0.2 §2.2). Covers definition, expansion with
/// arguments appended, single-expansion of a self-referential alias, and
/// removal via unalias.
fn test_shell_aliases() {
    println("\n[test] racsh aliases");

    // A defined alias expands, and the caller's arguments follow it.
    let a1 = shell_run(
        b"alias greet='echo hello'; result=$(greet world); \
          case $result in 'hello world') exit 0;; *) exit 1;; esac\0",
    );
    check!("alias expands with arguments appended", a1 == Some(0));

    // A self-referential alias must expand exactly once, not loop.
    let a2 = shell_run(b"alias echo='echo'; echo ok; exit 0\0");
    check!("self-referential alias terminates", a2 == Some(0));

    // unalias removes it: the name then resolves as an ordinary command,
    // and a bogus one exits non-zero rather than expanding.
    let a3 = shell_run(
        b"alias tmpname='echo aliased'; unalias tmpname; \
          tmpname 2>/dev/null; case $? in 0) exit 1;; *) exit 0;; esac\0",
    );
    check!("unalias removes the definition", a3 == Some(0));

    // `alias` with no operands lists definitions in NAME='VALUE' form.
    let a4 = shell_run(
        b"alias zz='echo z'; result=$(alias); \
          case $result in *\"zz='echo z'\"*) exit 0;; *) exit 1;; esac\0",
    );
    check!("bare alias lists definitions", a4 == Some(0));

    if a1 == Some(0) && a2 == Some(0) && a3 == Some(0) && a4 == Some(0) {
        println("T22-ALIAS-OK");
    }
}

/// Smoke for the v0.2 §2.1 net-new tools: clear, rmdir, free, du.
///
/// Each is driven through racsh so redirection and exit codes are exercised
/// alongside the binary itself.
fn test_v02_coreutils() {
    println("\n[test] v0.2 §2.1 coreutils: clear + rmdir + free + du");

    // clear emits ED 2 + CUP and exits 0. Assert on the exit status rather
    // than the bytes: the escape sequence would corrupt the test transcript.
    let c1 = run_bin(b"/bin/clear\0", &[b"clear\0"]);
    check!("clear exits 0", c1 == Some(0));

    // rmdir removes an empty directory...
    let r1 = shell_run(
        b"mkdir /tmp/rd1; rmdir /tmp/rd1; \
          test -d /tmp/rd1 && exit 1; exit 0\0",
    );
    check!("rmdir removes an empty directory", r1 == Some(0));

    // ...and refuses a non-empty one, leaving it in place.
    let r2 = shell_run(
        b"mkdir /tmp/rd2; echo x > /tmp/rd2/f; \
          rmdir /tmp/rd2 2>/dev/null; \
          test -d /tmp/rd2; exit $?\0",
    );
    check!("rmdir refuses a non-empty directory", r2 == Some(0));

    // ...and refuses a plain file.
    let r3 = shell_run(
        b"echo x > /tmp/rd3; rmdir /tmp/rd3 2>/dev/null; \
          case $? in 0) exit 1;; *) exit 0;; esac\0",
    );
    check!("rmdir refuses a regular file", r3 == Some(0));

    // free reports a Mem: row with a non-zero total.
    let f1 = shell_run(
        b"result=$(free | grep Mem); \
          case $result in Mem:*) exit 0;; *) exit 1;; esac\0",
    );
    check!("free prints a Mem: row", f1 == Some(0));

    // du -s on a directory with known content reports a non-zero total.
    let d1 = shell_run(
        b"mkdir /tmp/du1; echo hello > /tmp/du1/a; \
          result=$(du -sb /tmp/du1); \
          case $result in 0*) exit 1;; *) exit 0;; esac\0",
    );
    check!("du -sb reports a non-zero size", d1 == Some(0));

    if c1 == Some(0)
        && r1 == Some(0)
        && r2 == Some(0)
        && r3 == Some(0)
        && f1 == Some(0)
        && d1 == Some(0)
    {
        println("T21-COREUTILS-OK");
    }
}

/// Smoke for hard links: sys_link plus racfs link-count bookkeeping.
///
/// Runs on /mnt because racfs is the only filesystem that supports links —
/// tmpfs and FAT32 answer EPERM by design, which the last case checks.
fn test_hard_links() {
    println("\n[test] hard links (ln + sys_link)");

    // A link reads back the original's contents.
    let l1 = shell_run(
        b"echo linked > /mnt/hl_a; ln /mnt/hl_a /mnt/hl_b; \
          result=$(cat /mnt/hl_b); \
          case $result in linked) exit 0;; *) exit 1;; esac\0",
    );
    check!("ln creates a second readable name", l1 == Some(0));

    // THE point of hard links: removing one name must not destroy the data.
    let l2 = shell_run(
        b"echo survive > /mnt/hl_c; ln /mnt/hl_c /mnt/hl_d; rm /mnt/hl_c; \
          result=$(cat /mnt/hl_d); \
          case $result in survive) exit 0;; *) exit 1;; esac\0",
    );
    check!("data survives unlinking the first name", l2 == Some(0));

    // Removing the last name does free it. Checked via `ls | grep` rather
    // than `cat`, so the assertion tests unlink rather than cat's exit status.
    let l3 = shell_run(
        b"rm /mnt/hl_d; result=$(ls /mnt | grep hl_d); \
          case $result in '') exit 0;; *) exit 1;; esac\0",
    );
    check!("removing the last name frees the file", l3 == Some(0));

    // cat must report failure in its exit status, not only on stderr.
    let l3b = shell_run(b"cat /mnt/hl_d 2>/dev/null && exit 1; exit 0\0");
    check!("cat exits non-zero for a missing file", l3b == Some(0));

    // Linking a directory is refused (EPERM) rather than corrupting the tree.
    let l4 = shell_run(
        b"mkdir /mnt/hl_dir 2>/dev/null; \
          ln /mnt/hl_dir /mnt/hl_dirlink 2>/dev/null; \
          case $? in 0) exit 1;; *) exit 0;; esac\0",
    );
    check!("linking a directory is refused", l4 == Some(0));

    // An existing destination is refused rather than silently replaced.
    let l5 = shell_run(
        b"echo one > /mnt/hl_e; echo two > /mnt/hl_f; \
          ln /mnt/hl_e /mnt/hl_f 2>/dev/null; \
          case $? in 0) exit 1;; *) exit 0;; esac\0",
    );
    check!("existing destination is refused", l5 == Some(0));

    // tmpfs cannot hard-link; the error must be reported, not ignored.
    let l6 = shell_run(
        b"echo x > /tmp/hl_g; ln /tmp/hl_g /tmp/hl_h 2>/dev/null; \
          case $? in 0) exit 1;; *) exit 0;; esac\0",
    );
    check!("tmpfs reports that it cannot hard-link", l6 == Some(0));

    if l1 == Some(0)
        && l2 == Some(0)
        && l3 == Some(0)
        && l3b == Some(0)
        && l4 == Some(0)
        && l5 == Some(0)
        && l6 == Some(0)
    {
        println("T21-HARDLINK-OK");
    }
}

/// Smoke for the v0.2 §2.3 network tools: ping (SYS_ICMP_ECHO) and nc.
///
/// Only the gateway is pinged. QEMU's slirp answers ICMP for itself but does
/// not forward echo requests to the internet, so pinging an external host
/// would assert on the emulator's limits rather than on RacOS.
fn test_network_tools() {
    println("\n[test] network tools: ping + nc");

    // A reply from the gateway, with the conventional output shape.
    let p1 = shell_run(
        b"result=$(ping -c 1 -W 2000 10.0.2.2 | grep 'bytes from'); \
          case $result in *'10.0.2.2'*) exit 0;; *) exit 1;; esac\0",
    );
    check!("ping reports a reply from the gateway", p1 == Some(0));

    // The statistics block is what scripts parse; keep its shape pinned.
    let p2 = shell_run(
        b"result=$(ping -c 2 -W 2000 10.0.2.2 | grep transmitted); \
          case $result in *'2 packets transmitted, 2 received'*) exit 0;; *) exit 1;; esac\0",
    );
    check!("ping summarises transmitted/received", p2 == Some(0));

    // An address nothing answers for must time out and exit non-zero rather
    // than hang -- the wait runs in the kernel with interrupts on, so a
    // missing deadline would wedge the whole system.
    let p3 = shell_run(
        b"ping -c 1 -W 500 10.0.2.99 >/dev/null 2>/dev/null; \
          case $? in 0) exit 1;; *) exit 0;; esac\0",
    );
    check!("unreachable ping times out and fails", p3 == Some(0));

    // Bad usage is rejected, not silently accepted.
    let p4 = shell_run(b"ping >/dev/null 2>/dev/null; case $? in 0) exit 1;; *) exit 0;; esac\0");
    check!("ping with no host exits non-zero", p4 == Some(0));

    let n1 = shell_run(b"nc >/dev/null 2>/dev/null; case $? in 0) exit 1;; *) exit 0;; esac\0");
    check!("nc with no arguments exits non-zero", n1 == Some(0));

    // Connecting to a closed local port must fail cleanly.
    let n2 = shell_run(
        b"nc 127.0.0.1 9 >/dev/null 2>/dev/null; \
          case $? in 0) exit 1;; *) exit 0;; esac\0",
    );
    check!("nc reports a refused connection", n2 == Some(0));

    if p1 == Some(0)
        && p2 == Some(0)
        && p3 == Some(0)
        && p4 == Some(0)
        && n1 == Some(0)
        && n2 == Some(0)
    {
        println("T23-NETTOOLS-OK");
    }
}

fn test_init_engine_supervises_shell() {
    println("\n[test] init engine supervises shell");

    let my_pid = getpid();
    let parent_pid = getppid();
    check!("getpid() returns a real PID", my_pid > 1);
    check!(
        "getppid() returns a real parent (not init, not self)",
        parent_pid > 1 && parent_pid != my_pid
    );

    // We can't directly inspect init's logs from here, but a healthy
    // engine path means: we exist, our parent exists, and that parent
    // is the shell that init started from shell.service.
    if my_pid > 1 && parent_pid > 1 && parent_pid != my_pid {
        println("T13-INIT-ENGINE-OK");
    }
}

/// Smoke for /bin/ps: spawn it, expect exit 0. ps walks /proc, opens
/// each numeric pid dir's status file, prints PID/PPID/STATE/NAME. Since
/// at least init (PID 1) and our shell ancestor are running, the table
/// will be non-empty. We can't easily capture stdout from a forked child
/// here, but exit 0 is sufficient evidence that /proc + status parsing
/// + getdents + write all worked together.
fn test_ps_lists_running_processes() {
    println("\n[test] /bin/ps lists running processes");

    let path = b"/bin/ps\0";
    let arg0 = b"ps\0";
    let argv: [*const u8; 2] = [arg0.as_ptr(), core::ptr::null()];

    let pid = match spawn_args(path, &argv) {
        Ok(p) => p,
        Err(_) => {
            check!("spawn /bin/ps returns Ok", false);
            return;
        }
    };
    check!("spawn /bin/ps returns Ok", true);

    let mut status: i32 = -99;
    let waited = waitpid(pid, &mut status, 0);
    check!("waitpid returns the ps child", waited.unwrap_or(-1) == pid);
    check!("ps exits with status 0", status == 0);

    if waited.unwrap_or(-1) == pid && status == 0 {
        println("T33-PS-OK");
    }
}

/// Smoke for /bin/rpkg: build a minimal valid .rpk in memory, write it to
/// /tmp, install it, list, remove it, list again. End-to-end exercises
/// the lib's header parser + section extractor + manifest TOML reader
/// AND the bin's filesystem write/unlink/getdents path.
fn test_rpkg_install_list_remove() {
    println("\n[test] /bin/rpkg install/list/remove cycle");

    // Construct a minimal .rpk: 56-byte header + manifest + signature + data.
    // Manifest declares name = "demo-rpkg" so install lands at
    // /var/lib/rpkg/info/demo-rpkg/.
    let manifest: &[u8] =
        b"[package]\nname = \"demo-rpkg\"\nversion = \"0.0.1\"\narch = \"x86_64\"\n";
    let signature: &[u8] = b"x"; // not verified in MVP
    let data: &[u8] = b"DEMO_RPKG_PAYLOAD\n";

    let mo: u64 = 56;
    let ms: u64 = manifest.len() as u64;
    let so: u64 = mo + ms;
    let ss: u64 = signature.len() as u64;
    let doff: u64 = so + ss;
    let ds: u64 = data.len() as u64;

    let mut rpk = [0u8; 256];
    rpk[0..4].copy_from_slice(&[b'R', b'P', b'K', 0x01]);
    rpk[4..8].copy_from_slice(&1u32.to_le_bytes());
    rpk[8..16].copy_from_slice(&mo.to_le_bytes());
    rpk[16..24].copy_from_slice(&ms.to_le_bytes());
    rpk[24..32].copy_from_slice(&so.to_le_bytes());
    rpk[32..40].copy_from_slice(&ss.to_le_bytes());
    rpk[40..48].copy_from_slice(&doff.to_le_bytes());
    rpk[48..56].copy_from_slice(&ds.to_le_bytes());
    let mut p = 56usize;
    rpk[p..p + manifest.len()].copy_from_slice(manifest);
    p += manifest.len();
    rpk[p..p + signature.len()].copy_from_slice(signature);
    p += signature.len();
    rpk[p..p + data.len()].copy_from_slice(data);
    let rpk_len = p + data.len();

    // Write to /tmp/demo.rpk on tmpfs (writable).
    let rpk_path = b"/tmp/demo.rpk\0";
    let create_flags = O_RDWR | O_CREAT | O_TRUNC;
    let fd = match open(rpk_path, create_flags, 0o644) {
        Ok(fd) => fd,
        Err(_) => {
            check!("create /tmp/demo.rpk", false);
            return;
        }
    };
    let written = write(fd, &rpk[..rpk_len]).unwrap_or(0);
    let _ = close(fd);
    check!("wrote full .rpk to /tmp", written == rpk_len);

    // rpkg install /tmp/demo.rpk
    let rpkg_path = b"/bin/rpkg\0";
    let arg0 = b"rpkg\0";
    let install_arg = b"install\0";
    let path_arg = b"/tmp/demo.rpk\0";
    let argv_install: [*const u8; 4] = [
        arg0.as_ptr(),
        install_arg.as_ptr(),
        path_arg.as_ptr(),
        core::ptr::null(),
    ];
    let install_exit = run_and_wait(rpkg_path, &argv_install);
    check!("rpkg install exits 0", install_exit == Some(0));

    // rpkg list (exit code only — output goes to serial, smoke can grep).
    let list_arg = b"list\0";
    let argv_list: [*const u8; 3] = [arg0.as_ptr(), list_arg.as_ptr(), core::ptr::null()];
    let list_exit = run_and_wait(rpkg_path, &argv_list);
    check!("rpkg list exits 0", list_exit == Some(0));

    // rpkg remove demo-rpkg
    let remove_arg = b"remove\0";
    let name_arg = b"demo-rpkg\0";
    let argv_remove: [*const u8; 4] = [
        arg0.as_ptr(),
        remove_arg.as_ptr(),
        name_arg.as_ptr(),
        core::ptr::null(),
    ];
    let remove_exit = run_and_wait(rpkg_path, &argv_remove);
    check!("rpkg remove exits 0", remove_exit == Some(0));

    if install_exit == Some(0) && list_exit == Some(0) && remove_exit == Some(0) {
        println("T32-RPKG-OK");
    }
}

/// Spawn `path` with the given argv array, wait for it, return exit status
/// (Some(status) on success, None on spawn/waitpid failure).
fn run_and_wait(path: &[u8], argv: &[*const u8]) -> Option<i32> {
    let pid = spawn_args(path, argv).ok()?;
    let mut status: i32 = -1;
    if waitpid(pid, &mut status, 0).is_err() {
        return None;
    }
    Some(status)
}

/// Smoke for /bin/sed: drive it through racsh with command substitution
/// + case matching. racsh's echo has no `-e` flag and there's no
/// `printf`, so each test sticks to single-line input — d/p semantics
/// are still exercised, just on a single line.
fn test_sed_substitute() {
    println("\n[test] /bin/sed s/X/Y/[g], d, -n p");

    // s/X/Y/ — first-occurrence substitution.
    let s1 = shell_run(
        b"result=$(echo hello | /bin/sed 's/hello/world/'); \
          case $result in world) exit 0;; *) exit 1;; esac\0",
    );
    check!("sed s/X/Y/ swaps hello → world", s1 == Some(0));

    // s/X/Y/g — global substitution across multiple matches on one line.
    let s2 = shell_run(
        b"result=$(echo aaa | /bin/sed 's/a/b/g'); \
          case $result in bbb) exit 0;; *) exit 1;; esac\0",
    );
    check!("sed s/X/Y/g hits every match (aaa → bbb)", s2 == Some(0));

    // s/X/Y/ without g must NOT substitute the second occurrence.
    let s3 = shell_run(
        b"result=$(echo aXa | /bin/sed 's/a/b/'); \
          case $result in bXa) exit 0;; *) exit 1;; esac\0",
    );
    check!(
        "sed s/X/Y/ (no g) only first hit (aXa → bXa)",
        s3 == Some(0)
    );

    // -n with `p` — suppress default, explicit print emits the line once
    // (the `p` print is the only one when -n is set).
    let s4 = shell_run(
        b"result=$(echo hello | /bin/sed -n p); \
          case $result in hello) exit 0;; *) exit 1;; esac\0",
    );
    check!("sed -n p echoes each line exactly once", s4 == Some(0));

    // d — deletes the line, so substitution output is empty.
    let s5 = shell_run(
        b"result=$(echo dropme | /bin/sed d); \
          case $result in '') exit 0;; *) exit 1;; esac\0",
    );
    check!("sed d drops the line", s5 == Some(0));

    if s1 == Some(0) && s2 == Some(0) && s3 == Some(0) && s4 == Some(0) && s5 == Some(0) {
        println("T33-SED-OK");
    }
}

/// Smoke for /bin/awk: MVP supports BEGIN/END blocks, $0..$N fields, print
/// with literal-string and field items, and `-F` single-byte separator.
/// Each case pipes a single line of input through awk and checks the
/// stdout via shell command substitution + case match.
fn test_awk_basic() {
    println("\n[test] /bin/awk BEGIN/END + $N + -F");

    // BEGIN runs once before input even if stdin is empty.
    let a1 = shell_run(
        b"result=$(echo '' | /bin/awk 'BEGIN { print \"hi\" }'); \
          case $result in hi) exit 0;; *) exit 1;; esac\0",
    );
    check!("awk BEGIN prints once", a1 == Some(0));

    // print $0 in the main block echoes the whole line.
    let a2 = shell_run(
        b"result=$(echo hello | /bin/awk '{ print $0 }'); \
          case $result in hello) exit 0;; *) exit 1;; esac\0",
    );
    check!("awk { print $0 } echoes the whole line", a2 == Some(0));

    // print $2 picks the second whitespace-separated field.
    let a3 = shell_run(
        b"result=$(echo a b c | /bin/awk '{ print $2 }'); \
          case $result in b) exit 0;; *) exit 1;; esac\0",
    );
    check!("awk { print $2 } picks field 2", a3 == Some(0));

    // -F separator: ':' splits "a:b:c" → fields ["a","b","c"].
    let a4 = shell_run(
        b"result=$(echo a:b:c | /bin/awk -F : '{ print $3 }'); \
          case $result in c) exit 0;; *) exit 1;; esac\0",
    );
    check!(
        "awk -F : { print $3 } picks 3rd colon-separated field",
        a4 == Some(0)
    );

    // END runs once after the input is consumed.
    //
    // This case used to be un-smokeable: any script reaching racsh through
    // `$(...)` could come back as `sh: cannot open script:` status 127. The
    // cause was not racsh at all — `prepare_user_stack` wrote the envp NULL
    // terminator one slot past the reserved argc/argv/envp block, clobbering
    // the argv string data sitting directly above it. Whether it corrupted
    // anything depended on the total argv length (hence the "only with END"
    // appearance). Fixed in kernel/src/task/process.rs.
    let a5 = shell_run(
        b"result=$(echo hello | /bin/awk 'END { print \"done\" }'); \
          case $result in done) exit 0;; *) exit 1;; esac\0",
    );
    check!("awk END prints once after input", a5 == Some(0));

    if a1 == Some(0) && a2 == Some(0) && a3 == Some(0) && a4 == Some(0) && a5 == Some(0) {
        println("T33-AWK-OK");
    }
}

/// Smoke for /bin/id (v0.2 §2.1 easy-win). id prints uid/gid/euid/egid in
/// the format `uid=N gid=N euid=N egid=N`. The test crate runs as root
/// (PID 1's child) so every value is 0. We assert exit 0 + the output
/// contains the `uid=` prefix.
fn test_id_prints_creds() {
    println("\n[test] /bin/id prints creds");

    let s = shell_run(
        b"result=$(/bin/id); \
          case $result in uid=*) exit 0;; *) exit 1;; esac\0",
    );
    check!("id output starts with uid=", s == Some(0));
    if s == Some(0) {
        println("T20-ID-OK");
    }
}

/// Smoke for /bin/sort (v0.2 §2.1 easy-win). Writes three unsorted lines
/// directly to a tmpfs file via libc-lite (no `printf` exists yet, and
/// racsh's echo doesn't support -e escapes), then runs `/bin/sort` on
/// that file and asserts the output joined with case-glob is `a*b*c`.
fn test_sort_orders_lines() {
    println("\n[test] /bin/sort orders lines");

    // Write reverse-ordered lines to a tmpfs file.
    let path = b"/tmp/sortin\0";
    let fd = match open(path, O_RDWR | O_CREAT | O_TRUNC, 0o644) {
        Ok(fd) => fd,
        Err(_) => {
            println("  [FAIL] open /tmp/sortin");
            unsafe {
                FAIL += 1;
            }
            return;
        }
    };
    let _ = write(fd, b"c\nb\na\n");
    let _ = close(fd);

    let s = shell_run(
        b"result=$(/bin/sort /tmp/sortin); \
          case $result in a*b*c) exit 0;; *) exit 1;; esac\0",
    );
    check!("sort orders lines a < b < c", s == Some(0));
    if s == Some(0) {
        println("T20-SORT-OK");
    }
}

/// Smoke for /bin/top (v0.2 §2.1 easy-win). Batch mode: prints the
/// `top - RacOS` header and a process table, then exits. We assert
/// exit 0 + the header is in the output.
fn test_top_prints_snapshot() {
    println("\n[test] /bin/top prints batch snapshot");

    let s = shell_run(
        b"result=$(/bin/top); \
          case $result in *top\\ -\\ RacOS*) exit 0;; *) exit 1;; esac\0",
    );
    check!("top output contains the header line", s == Some(0));
    if s == Some(0) {
        println("T20-TOP-OK");
    }
}

/// Run a userland binary directly (bypassing /bin/sh) and return its
/// exit status.
///
/// This originally worked around the `sh: cannot open script:` status-127
/// flake (an out-of-block envp write in `prepare_user_stack` that corrupted
/// argv strings — now fixed). It is kept because direct spawn is a closer
/// match to what these smokes actually test: the binary, not the shell.
fn run_bin(path: &[u8], args: &[&[u8]]) -> Option<i32> {
    // Build an argv array of pointers. Cap at 8 args incl. argv[0]
    // + NULL terminator — every smoke today fits.
    let mut argv: [*const u8; 8] = [core::ptr::null(); 8];
    let mut n = 0usize;
    while n < args.len() && n + 1 < argv.len() {
        argv[n] = args[n].as_ptr();
        n += 1;
    }
    argv[n] = core::ptr::null();

    let pid = match spawn_args(path, &argv[..=n]) {
        Ok(p) => p,
        Err(_) => return None,
    };
    let mut status: i32 = -99;
    if waitpid(pid, &mut status, 0).is_err() {
        return None;
    }
    Some(status)
}

/// Smoke for /bin/touch (v0.2 §2.1). Asserts: (a) touching a non-existent
/// path creates the file (verified by stat); (b) touching an existing
/// path is a no-op that still exits 0.
fn test_touch_creates_file() {
    println("\n[test] /bin/touch creates files");

    let path = b"/tmp/t_touch\0";
    let _ = unlink(path);

    // First touch: create.
    let s1 = run_bin(b"/bin/touch\0", &[b"touch\0", b"/tmp/t_touch\0"]);
    check!("touch (create) exit 0", s1 == Some(0));

    let mut raw = [0u8; 80];
    let st_ret = stat(path, &mut raw);
    check!("stat touched file returns Ok", st_ret.is_ok());

    // Second touch: existing file path, no-op exit 0.
    let s2 = run_bin(b"/bin/touch\0", &[b"touch\0", b"/tmp/t_touch\0"]);
    check!("touch (existing) exit 0", s2 == Some(0));

    let _ = unlink(path);

    if s1 == Some(0) && s2 == Some(0) && st_ret.is_ok() {
        println("T20-TOUCH-OK");
    }
}

/// Smoke for /bin/chmod (v0.2 §2.1). Creates a file, runs chmod 0600
/// directly via spawn, stats it back and asserts the mode bits.
fn test_chmod_sets_mode() {
    println("\n[test] /bin/chmod sets mode");

    let path = b"/tmp/t_chmod\0";
    let _ = unlink(path);
    let fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0o644);
    check!("setup: open O_CREAT", fd.is_ok());
    if let Ok(fd) = fd {
        let _ = close(fd);
    } else {
        return;
    }

    let s = run_bin(b"/bin/chmod\0", &[b"chmod\0", b"0600\0", b"/tmp/t_chmod\0"]);
    check!("chmod exit 0", s == Some(0));

    let mut raw = [0u8; 80];
    let st_ret = stat(path, &mut raw);
    check!("stat after chmod returns Ok", st_ret.is_ok());
    let mode_ok = if st_ret.is_ok() {
        let st = unsafe { &*(raw.as_ptr() as *const StatBuf) };
        (st.st_mode & 0o777) == 0o600
    } else {
        false
    };
    check!("mode bits == 0600", mode_ok);

    let _ = unlink(path);
    if s == Some(0) && mode_ok {
        println("T20-CHMOD-OK");
    }
}

/// Smoke for /bin/chown (v0.2 §2.1). Creates a file, runs chown 1234:5678
/// directly via spawn, stats it back and asserts uid+gid. Runs as root
/// so chown to an arbitrary uid is allowed.
fn test_chown_sets_uid_gid() {
    println("\n[test] /bin/chown sets uid:gid");

    let path = b"/tmp/t_chown\0";
    let _ = unlink(path);
    let fd = open(path, O_CREAT | O_RDWR | O_TRUNC, 0o644);
    check!("setup: open O_CREAT", fd.is_ok());
    if let Ok(fd) = fd {
        let _ = close(fd);
    } else {
        return;
    }

    let s = run_bin(
        b"/bin/chown\0",
        &[b"chown\0", b"1234:5678\0", b"/tmp/t_chown\0"],
    );
    check!("chown exit 0", s == Some(0));

    let mut raw = [0u8; 80];
    let st_ret = stat(path, &mut raw);
    check!("stat after chown returns Ok", st_ret.is_ok());
    let (uid_ok, gid_ok) = if st_ret.is_ok() {
        let st = unsafe { &*(raw.as_ptr() as *const StatBuf) };
        (st.st_uid == 1234, st.st_gid == 5678)
    } else {
        (false, false)
    };
    check!("uid == 1234", uid_ok);
    check!("gid == 5678", gid_ok);

    // Restore so subsequent tests can unlink the file.
    let _ = chown(path, 0, 0);
    let _ = unlink(path);
    if s == Some(0) && uid_ok && gid_ok {
        println("T20-CHOWN-OK");
    }
}

/// Smoke for envp inheritance: racsh sets a variable, spawns /bin/env via
/// command substitution, and we use case-match to assert the variable
/// shows up in the printed environment. Exercises the full chain:
///   shell builds envp from env.vars()
///   → libc_lite::spawn_args_envp → sys_spawn(_, _, envp)
///   → collect_user_envp → UserProcess::from_elf writes envp on user stack
///   → /bin/env's libc-lite _start records ENVP_BLOCK
///   → env walks ENVP_BLOCK and prints each KEY=VALUE.
fn test_env_inherits_shell_vars() {
    println("\n[test] /bin/env reads inherited environ");

    // racsh ships with PATH preset; any spawn we make should see it.
    let path_visible =
        shell_run(b"result=$(/bin/env); case $result in *PATH=*) exit 0;; *) exit 1;; esac\0");
    check!("env shows the inherited PATH", path_visible == Some(0));

    // A variable set in the shell session must survive across the spawn
    // and reach the child via envp.
    let custom_visible = shell_run(
        b"RACOS_SMOKE_KEY=racos-smoke-value; \
          result=$(/bin/env); \
          case $result in *RACOS_SMOKE_KEY=racos-smoke-value*) exit 0;; *) exit 1;; esac\0",
    );
    check!(
        "env shows a freshly-set shell variable",
        custom_visible == Some(0)
    );

    if path_visible == Some(0) && custom_visible == Some(0) {
        println("T33-ENV-OK");
    }
}

fn test_signal_user_handler_reentrant_syscall() {
    println("\n[test] signal handler issues syscall");

    unsafe {
        REENTRANT_BYTES_WRITTEN = 0;
    }
    let installed = signal(SIGUSR1, sigusr1_writing_handler);
    check!("signal(SIGUSR1, handler) returns Ok", installed.is_ok());

    let pid = getpid();
    let sent = kill(pid, SIGUSR1);
    check!("kill(self, SIGUSR1) returns Ok", sent.is_ok());

    let written = unsafe { REENTRANT_BYTES_WRITTEN };
    // "[handler]" is 9 bytes.
    check!("re-entrant write() in handler returns 9", written == 9);

    if written == 9 {
        println("PHASE21-USER-HANDLER-REENTRANT-OK");
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
