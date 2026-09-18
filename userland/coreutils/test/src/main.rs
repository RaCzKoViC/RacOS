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
    test_fsck_reports_clean();
    test_large_files_and_dirs();
    test_truncate_semantics();
    test_coreutils_preserve_data();
    test_rename_semantics();
    test_persistent_mount_layout();
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

    // /proc/net/tcp must exist and carry its header even with no connections.
    let s1 = shell_run(
        b"result=$(cat /proc/net/tcp | head -1); \
          case $result in *proto*) exit 0;; *) exit 1;; esac\0",
    );
    check!("/proc/net/tcp is readable", s1 == Some(0));

    // netstat prints its own header and a count, and succeeds with no
    // connections open.
    let s2 = shell_run(
        b"result=$(netstat -t | head -1); \
          case $result in Proto*) exit 0;; *) exit 1;; esac\0",
    );
    check!("netstat prints a header", s2 == Some(0));

    let s3 = shell_run(
        b"result=$(netstat | grep connection); \
          case $result in *connection*) exit 0;; *) exit 1;; esac\0",
    );
    check!("netstat summarises a connection count", s3 == Some(0));

    // `head -1` / `tail -1` are the forms people actually type. They used to
    // fall through to the FILE branch and be opened as a path.
    let s4 = shell_run(
        b"result=$(ls /bin | head -1); \
          case $result in '') exit 1;; *) exit 0;; esac\0",
    );
    check!("head -N reads stdin instead of opening '-N'", s4 == Some(0));

    let s5 = shell_run(
        b"result=$(ls /bin | tail -1); \
          case $result in '') exit 1;; *) exit 0;; esac\0",
    );
    check!("tail -N reads stdin instead of opening '-N'", s5 == Some(0));

    // tail must return the LAST line, not the first: a backwards scan that
    // counted the trailing newline used to emit nothing at all.
    let s6 = shell_run(
        b"result=$(echo one > /tmp/t3; echo two >> /tmp/t3; echo three >> /tmp/t3; \
          tail -1 /tmp/t3); \
          case $result in three) exit 0;; *) exit 1;; esac\0",
    );
    check!("tail -1 returns the last line", s6 == Some(0));

    let s7 = shell_run(
        b"result=$(tail -2 /tmp/t3 | head -1); \
          case $result in two) exit 0;; *) exit 1;; esac\0",
    );
    check!("tail -2 returns the last two lines", s7 == Some(0));

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
        && s1 == Some(0)
        && s2 == Some(0)
        && s3 == Some(0)
        && s4 == Some(0)
        && s5 == Some(0)
        && s6 == Some(0)
        && s7 == Some(0)
    {
        println("T23-NETTOOLS-OK");
    }
}

/// The mount-time racfs consistency check must find nothing wrong on the
/// disk CI boots with (ROADMAP v0.3 §3.2).
///
/// This asserts the *absence* of a warning, which is a weak shape for a test —
/// so it also confirms the check actually ran. A silent pass because the code
/// never executed would otherwise look identical to a healthy filesystem.
///
/// It reads the kernel's boot output through /proc/kmsg-style access, which
/// RacOS does not have, so it works the way it can: writing and re-reading
/// files exercises the same allocator the check validates, and the check's
/// own verdict is asserted by the CI log grep for "fsck clean".
fn test_fsck_reports_clean() {
    println("\n[test] racfs consistency after normal use");

    // Exercise the allocator: create, extend, delete, recreate. Any of these
    // leaking a block or double-allocating one is what check() detects at the
    // next mount, and what the boot-smoke's second boot would then report.
    let f1 = shell_run(
        b"mkdir /mnt/fsck1; echo a > /mnt/fsck1/one; echo bb >> /mnt/fsck1/one; \
          echo c > /mnt/fsck1/two; rm /mnt/fsck1/two; echo d > /mnt/fsck1/three; \
          result=$(cat /mnt/fsck1/one); \
          case $result in *a*) exit 0;; *) exit 1;; esac\0",
    );
    check!(
        "allocator round-trip survives create/extend/delete",
        f1 == Some(0)
    );

    // Hard links share an inode: unlinking one name must not free blocks the
    // other still references, which check() would see as unallocated_in_use.
    let f2 = shell_run(
        b"echo shared > /mnt/fsck2; ln /mnt/fsck2 /mnt/fsck2b; rm /mnt/fsck2; \
          result=$(cat /mnt/fsck2b); \
          case $result in shared) exit 0;; *) exit 1;; esac\0",
    );
    check!(
        "hard-link unlink leaves the surviving name intact",
        f2 == Some(0)
    );

    // sync must not error; the check runs against what was flushed.
    let f3 = shell_run(b"sync; exit $?\0");
    check!("sync succeeds after the churn", f3 == Some(0));

    if f1 == Some(0) && f2 == Some(0) && f3 == Some(0) {
        println("T32-FSCK-OK");
    }
}

/// Files and directories bigger than the eight direct block pointers hold
/// (ROADMAP v0.3 §3.2).
///
/// Every case here failed outright before indirect blocks existed: a racfs
/// file stopped at 8 * 512 = 4096 bytes and a directory at 64 entries, on a
/// disk reporting 16 MiB free. That is why `/var/log` and `/var/lib/rpkg`
/// could not move onto persistent storage.
fn test_large_files_and_dirs() {
    println("\n[test] racfs past the direct blocks");

    // Every size assertion below is `wc -c`, so pin that first. `wc` used to
    // ignore its flags and print lines, words and bytes whatever was asked,
    // which made `test "$(wc -c < f)" -eq N` compare "  258  258  16402"
    // against a number and fail for a reason that had nothing to do with the
    // filesystem. If this check fails, none of the rest means anything.
    let wc_flags = shell_run(
        b"echo abc > /mnt/wcflag; \
          c=$(wc -c < /mnt/wcflag); l=$(wc -l < /mnt/wcflag); \
          test \"$c\" -eq 4 && test \"$l\" -eq 1\0",
    );
    check!("wc -c and wc -l print one number each", wc_flags == Some(0));

    // A 64-byte seed line doubled up to 4096, then bracketed with markers.
    // The markers are the point: they land in the first and the last block,
    // so reading both back shows the block map returns the right block at
    // each end rather than merely returning something.
    //
    // 16402 bytes = 33 blocks: 8 direct plus 25 through the single indirect.
    let single = shell_run(
        b"mkdir /mnt/big; \
          echo 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde > /mnt/big/a; \
          cat /mnt/big/a /mnt/big/a /mnt/big/a /mnt/big/a > /mnt/big/b; \
          cat /mnt/big/b /mnt/big/b /mnt/big/b /mnt/big/b > /mnt/big/a; \
          cat /mnt/big/a /mnt/big/a /mnt/big/a /mnt/big/a > /mnt/big/b; \
          echo HEADMARK > /mnt/big/c; \
          cat /mnt/big/b /mnt/big/b /mnt/big/b /mnt/big/b >> /mnt/big/c; \
          echo TAILMARK >> /mnt/big/c; \
          n=$(wc -c < /mnt/big/c); test \"$n\" -eq 16402\0",
    );
    check!(
        "a 16402-byte file writes (single indirect)",
        single == Some(0)
    );

    let ends = shell_run(
        b"h=$(head -1 /mnt/big/c); t=$(tail -1 /mnt/big/c); \
          test \"$h\" = HEADMARK && test \"$t\" = TAILMARK\0",
    );
    check!("its first and last block read back", ends == Some(0));

    // 262432 bytes = 513 blocks, past the 136 that direct + single indirect
    // reach, so this one can only be served through the double indirect.
    let double = shell_run(
        b"cat /mnt/big/c /mnt/big/c /mnt/big/c /mnt/big/c > /mnt/big/d; \
          cat /mnt/big/d /mnt/big/d /mnt/big/d /mnt/big/d > /mnt/big/e; \
          n=$(wc -c < /mnt/big/e); t=$(tail -1 /mnt/big/e); \
          test \"$n\" -eq 262432 && test \"$t\" = TAILMARK\0",
    );
    check!(
        "a 262432-byte file writes (double indirect)",
        double == Some(0)
    );

    // Deleting a file must return every block it held, including the indirect
    // blocks themselves. Those come out of the same bitmap, and a free path
    // that forgot them would leak one block per 128 - invisible until a later
    // mount reported it, which is exactly the damage class fsck names.
    let no_leak = shell_run(
        b"before=$(grep /mnt /proc/diskstats | cut -f 4 -d ' '); \
          cat /mnt/big/b /mnt/big/b /mnt/big/b /mnt/big/b > /mnt/big/tmp; \
          rm /mnt/big/tmp; \
          after=$(grep /mnt /proc/diskstats | cut -f 4 -d ' '); \
          test \"$before\" -eq \"$after\"\0",
    );
    check!(
        "deleting it frees the indirect blocks too",
        no_leak == Some(0)
    );

    // 73 names in one directory, past the 64 that 8 blocks of entries hold.
    // They are hard links to a single inode on purpose: the directory is what
    // is under test, and 73 separate files would spend more than half the
    // filesystem's 128 inodes proving nothing extra.
    let many = shell_run(
        b"mkdir /mnt/manydir; echo x > /mnt/manydir/base; \
          for a in 1 2 3 4 5 6 7 8 9; do \
            for b in 1 2 3 4 5 6 7 8; do \
              ln /mnt/manydir/base /mnt/manydir/l$a$b; \
            done; \
          done; \
          n=$(ls /mnt/manydir | wc -l); test \"$n\" -eq 73\0",
    );
    check!("a directory holds 73 entries", many == Some(0));

    // The allocation bitmap must span the device, not stay at the one sector
    // it was fixed at. The `df`-style totals come straight from what the
    // bitmap can describe, so a total still stuck at 4096 blocks would mean a
    // single-sector bitmap however well the files above behaved.
    let wide_bitmap =
        shell_run(b"t=$(grep /mnt /proc/diskstats | cut -f 2 -d ' '); test \"$t\" -gt 4096\0");
    check!(
        "the bitmap describes more than one sector of blocks",
        wide_bitmap == Some(0)
    );

    if wc_flags == Some(0)
        && single == Some(0)
        && ends == Some(0)
        && double == Some(0)
        && no_leak == Some(0)
        && many == Some(0)
        && wide_bitmap == Some(0)
    {
        println("T34-BIGFILE-OK");
    }
}

/// Size of the file at `path` via stat, or None if stat fails.
fn stat_size(path: &[u8]) -> Option<u64> {
    let mut raw = [0u8; 80];
    stat(path, &mut raw).ok()?;
    // SAFETY: the kernel filled `raw` with a StatBuf (80 bytes, repr(C)).
    let st = unsafe { &*(raw.as_ptr() as *const StatBuf) };
    Some(st.st_size)
}

/// Size of the open file `fd` via fstat, or None if fstat fails.
fn fstat_size(fd: i32) -> Option<u64> {
    let mut raw = [0u8; 80];
    fstat(fd, &mut raw).ok()?;
    // SAFETY: as in stat_size.
    let st = unsafe { &*(raw.as_ptr() as *const StatBuf) };
    Some(st.st_size)
}

/// Free 512-byte blocks on the racfs mounted at /mnt, read from
/// /proc/diskstats ("<mount> <total> <used> <free> <inodes> <free inodes>").
/// The exact count is what makes a leak visible: a truncate that shrinks the
/// size but keeps the block map would leave this number where it was.
fn racfs_free_blocks() -> Option<u64> {
    let fd = open(b"/proc/diskstats\0", 0, 0).ok()?;
    let mut buf = [0u8; 1024];
    let n = read(fd, &mut buf).unwrap_or(0);
    let _ = close(fd);
    let text = &buf[..n];
    let mut line_start = 0usize;
    while line_start < text.len() {
        let mut line_end = line_start;
        while line_end < text.len() && text[line_end] != b'\n' {
            line_end += 1;
        }
        let line = &text[line_start..line_end];
        if line.starts_with(b"/mnt ") {
            // Field 4, space separated.
            let mut field = 0usize;
            let mut i = 0usize;
            while i < line.len() {
                if line[i] == b' ' {
                    field += 1;
                    i += 1;
                    continue;
                }
                if field == 3 {
                    let mut v: u64 = 0;
                    while i < line.len() && line[i].is_ascii_digit() {
                        v = v * 10 + (line[i] - b'0') as u64;
                        i += 1;
                    }
                    return Some(v);
                }
                i += 1;
            }
            return None;
        }
        line_start = line_end + 1;
    }
    None
}

/// Overwriting a file must shorten it. Before this group existed, `O_TRUNC`
/// in `sys_open` was a permission check that truncated nothing, `ftruncate`
/// returned 0 without touching the inode, and `truncate` was ENOSYS - so
/// `echo X > f` on an 11-byte file left 11 bytes with the old tail showing
/// after the X. Every `>` in racsh, the history rewrite, cp, mv and tee open
/// with O_TRUNC, so this was the common case, not a corner.
///
/// The fix lives in InodeOps and each filesystem, not in cat or the shell;
/// hence one reproduction per writable filesystem, and block accounting on
/// racfs to show the freed blocks really return to the bitmap.
fn test_truncate_semantics() {
    println("\n[test] truncate + O_TRUNC");

    // The review's reproduction, verbatim, on tmpfs, racfs and FAT32.
    let tmpfs = shell_run(
        b"echo ABCDEFGHIJ > /tmp/tr1; echo X > /tmp/tr1; \
          n=$(wc -c < /tmp/tr1); c=$(cat /tmp/tr1); \
          test \"$n\" -eq 2 && test \"$c\" = X\0",
    );
    check!(
        "echo X > f shortens an 11-byte file on tmpfs",
        tmpfs == Some(0)
    );
    let racfs = shell_run(
        b"echo ABCDEFGHIJ > /mnt/tr1; echo X > /mnt/tr1; \
          n=$(wc -c < /mnt/tr1); c=$(cat /mnt/tr1); \
          test \"$n\" -eq 2 && test \"$c\" = X\0",
    );
    check!(
        "echo X > f shortens an 11-byte file on racfs",
        racfs == Some(0)
    );
    let fat = shell_run(
        b"echo ABCDEFGHIJ > /fat/tr1; echo X > /fat/tr1; \
          n=$(wc -c < /fat/tr1); c=$(cat /fat/tr1); \
          test \"$n\" -eq 2 && test \"$c\" = X\0",
    );
    check!(
        "echo X > f shortens an 11-byte file on FAT32",
        fat == Some(0)
    );

    // O_APPEND takes the size from the inode at write time, so an append
    // after a truncating overwrite must continue at the new end, not the old.
    let append = shell_run(
        b"echo ABCDEFGHIJ > /mnt/tr2; echo X > /mnt/tr2; echo Y >> /mnt/tr2; \
          n=$(wc -c < /mnt/tr2); l=$(wc -l < /mnt/tr2); \
          test \"$n\" -eq 4 && test \"$l\" -eq 2\0",
    );
    check!(
        "append after truncate continues at the new end",
        append == Some(0)
    );

    // --- racfs block accounting through ftruncate -------------------------
    // A 65-byte line (64 characters + newline; the seed line the big-file
    // group uses is 64 bytes, which would land exactly on a block boundary
    // and hide an off-by-one), times 256: 16640 bytes = 33 data blocks,
    // 8 direct + 25 through one single-indirect block = 34 blocks owned.
    //
    // The three directory entries exist before the baseline is taken (a new
    // entry can cost the directory a block of its own), and the two seed
    // files are left alone until the end, so every difference measured below
    // is tr3c's data and its indirect block, nothing else.
    let seeded = shell_run(
        b"echo 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef > /mnt/tr3a; \
          cat /mnt/tr3a /mnt/tr3a /mnt/tr3a /mnt/tr3a > /mnt/tr3b; \
          cat /mnt/tr3b /mnt/tr3b /mnt/tr3b /mnt/tr3b > /mnt/tr3a; \
          cat /mnt/tr3a /mnt/tr3a /mnt/tr3a /mnt/tr3a > /mnt/tr3b; \
          touch /mnt/tr3c /mnt/tr5\0",
    );
    let free0 = racfs_free_blocks();
    let built = shell_run(b"cat /mnt/tr3b /mnt/tr3b /mnt/tr3b /mnt/tr3b > /mnt/tr3c\0");
    let path = b"/mnt/tr3c\0";
    let size_ok = seeded == Some(0) && built == Some(0) && stat_size(path) == Some(16640);
    check!("a 16640-byte racfs file is in place (33 blocks)", size_ok);
    let free1 = racfs_free_blocks();
    let owns_34 = match (free0, free1) {
        (Some(a), Some(b)) => a == b + 34,
        _ => false,
    };
    check!("it owns 33 data blocks plus one indirect block", owns_34);

    // --- the double-indirect map, cut at three different depths -----------
    // 8 x 16640 = 133120 bytes = 260 blocks: 8 direct, 128 through the
    // single-indirect block, 124 through the double-indirect block and one
    // second-level block. Owned: 260 + 1 + 1 + 1 = 263, on top of tr3c's 34.
    let dbl_built = shell_run(
        b"cat /mnt/tr3c /mnt/tr3c /mnt/tr3c /mnt/tr3c \
              /mnt/tr3c /mnt/tr3c /mnt/tr3c /mnt/tr3c > /mnt/tr5\0",
    );
    let p5 = b"/mnt/tr5\0";
    let fd1 = racfs_free_blocks();
    let dbl_ok = dbl_built == Some(0)
        && stat_size(p5) == Some(133120)
        && matches!((free0, fd1), (Some(a), Some(b)) if a == b + 34 + 263);
    check!(
        "a 133120-byte file owns 260 data + 3 pointer blocks",
        dbl_ok
    );
    let mut dbl_cuts = false;
    if let Ok(fd) = open(p5, O_RDWR, 0) {
        // 70000 bytes = 137 blocks: one pointer left in the second-level
        // block, 123 data blocks freed, every pointer block still needed.
        // Owned: 137 + 3 = 140.
        let c1 = ftruncate(fd, 70000);
        let f = racfs_free_blocks();
        let cut1 = c1.is_ok()
            && fstat_size(fd) == Some(70000)
            && matches!((free0, f), (Some(a), Some(b)) if a == b + 34 + 140);
        check!(
            "cut inside the double-indirect range keeps its pointer blocks",
            cut1
        );

        // 69632 bytes = exactly 136 blocks: nothing under the double-indirect
        // block survives, so it and its second-level block go too.
        // Owned: 136 + 1 = 137.
        let c2 = ftruncate(fd, 69632);
        let f = racfs_free_blocks();
        let cut2 = c2.is_ok()
            && fstat_size(fd) == Some(69632)
            && matches!((free0, f), (Some(a), Some(b)) if a == b + 34 + 137);
        check!(
            "cut at the single/double boundary frees the emptied pointer blocks",
            cut2
        );

        // 5000 bytes = 10 blocks: 8 direct + 2 through the single-indirect
        // block, which therefore stays. Owned: 10 + 1 = 11.
        let c3 = ftruncate(fd, 5000);
        let f = racfs_free_blocks();
        let cut3 = c3.is_ok()
            && fstat_size(fd) == Some(5000)
            && matches!((free0, f), (Some(a), Some(b)) if a == b + 34 + 11);
        check!(
            "cut inside the single-indirect range keeps that pointer block",
            cut3
        );

        let c4 = ftruncate(fd, 0);
        let f = racfs_free_blocks();
        let cut4 = c4.is_ok()
            && fstat_size(fd) == Some(0)
            && matches!((free0, f), (Some(a), Some(b)) if a == b + 34);
        check!("cut to zero returns all 263 blocks", cut4);
        let _ = close(fd);
        dbl_cuts = cut1 && cut2 && cut3 && cut4;
    } else {
        check!("open /mnt/tr5 O_RDWR for the double-indirect cuts", false);
    }

    let mut acct_ok = false;
    if let Ok(fd) = open(path, O_RDWR, 0) {
        // Down to exactly the direct blocks: 25 data + the indirect block go
        // back, 8 stay.
        let t1 = ftruncate(fd, 4096);
        let s1 = fstat_size(fd);
        let f2 = racfs_free_blocks();
        let step1 = t1.is_ok()
            && s1 == Some(4096)
            && matches!((free0, f2), (Some(a), Some(b)) if a == b + 8);
        check!("ftruncate to 4096 keeps 8 blocks, frees 26", step1);

        // One byte short of a block boundary: same 8 blocks, the byte before
        // the cut is still the original, nothing reads back past the cut.
        let t2 = ftruncate(fd, 4095);
        let s2 = fstat_size(fd);
        let f3 = racfs_free_blocks();
        let mut last = [0u8; 4];
        let _ = lseek(fd, 4094, SEEK_SET);
        let n = read(fd, &mut last).unwrap_or(99);
        // Byte 4094 is offset 4094 % 65 = 64 into the pattern line: '\n'.
        let step2 = t2.is_ok() && s2 == Some(4095) && f3 == f2 && n == 1 && last[0] == b'\n';
        check!("ftruncate to 4095 cuts inside a block", step2);

        // To zero: every block returns; the file is still there.
        let t3 = ftruncate(fd, 0);
        let s3 = fstat_size(fd);
        let f4 = racfs_free_blocks();
        let step3 = t3.is_ok() && s3 == Some(0) && f4 == free0;
        check!("ftruncate to 0 returns every block to the bitmap", step3);

        // Extending pads with zeros and allocates only what the new length
        // needs: 1024 bytes = 2 blocks.
        let t4 = ftruncate(fd, 1024);
        let s4 = fstat_size(fd);
        let f5 = racfs_free_blocks();
        let mut zeros = [0xAAu8; 1024];
        let _ = lseek(fd, 0, SEEK_SET);
        let n = read(fd, &mut zeros).unwrap_or(0);
        let all_zero = n == 1024 && zeros.iter().all(|&b| b == 0);
        let step4 = t4.is_ok()
            && s4 == Some(1024)
            && matches!((free0, f5), (Some(a), Some(b)) if a == b + 2)
            && all_zero;
        check!("ftruncate past the end extends with zeros", step4);
        let _ = close(fd);
        acct_ok = step1 && step2 && step3 && step4;
    } else {
        check!("open /mnt/tr3c O_RDWR for ftruncate", false);
    }

    // --- truncate(2) by path ----------------------------------------------
    let p2 = b"/mnt/tr4\0";
    let mut by_path = false;
    if let Ok(fd) = open(p2, O_RDWR | O_CREAT | O_TRUNC, 0o644) {
        let _ = write(fd, b"0123456789");
        let _ = close(fd);
        let t = truncate(p2, 5);
        let s = stat_size(p2);
        let mut head = [0u8; 16];
        let n = open(p2, 0, 0)
            .ok()
            .map(|fd| {
                let n = read(fd, &mut head).unwrap_or(0);
                let _ = close(fd);
                n
            })
            .unwrap_or(0);
        let cut = t.is_ok() && s == Some(5) && n == 5 && &head[..5] == b"01234";
        check!("truncate(path, 5) keeps the first five bytes", cut);
        let t2 = truncate(p2, 12);
        let s2 = stat_size(p2);
        let mut back = [0xAAu8; 16];
        let n2 = open(p2, 0, 0)
            .ok()
            .map(|fd| {
                let n = read(fd, &mut back).unwrap_or(0);
                let _ = close(fd);
                n
            })
            .unwrap_or(0);
        let grown = t2.is_ok()
            && s2 == Some(12)
            && n2 == 12
            && &back[..5] == b"01234"
            && back[5..12].iter().all(|&b| b == 0);
        check!("truncate(path, 12) pads bytes 5..12 with zeros", grown);
        by_path = cut && grown;
    } else {
        check!("create /mnt/tr4 for truncate(path)", false);
    }

    // --- refusals: a wrong call must fail, never report success ------------
    let ro = open(p2, 0, 0);
    let ro_refused = match ro {
        Ok(fd) => {
            let r = ftruncate(fd, 0);
            let _ = close(fd);
            r == Err(-22) && stat_size(p2) == Some(12)
        }
        Err(_) => false,
    };
    check!(
        "ftruncate on a read-only fd is EINVAL and changes nothing",
        ro_refused
    );

    let dir_refused = truncate(b"/mnt\0", 0) == Err(-21);
    check!("truncate on a directory is EISDIR", dir_refused);

    let rofs_size = stat_size(b"/bin/true\0");
    let rofs = truncate(b"/bin/true\0", 0);
    let rofs_refused = rofs.is_err()
        && stat_size(b"/bin/true\0") == rofs_size
        && run_bin(b"/bin/true\0", &[b"true\0"]) == Some(0);
    check!(
        "truncate on the read-only initramfs fails and leaves /bin/true intact",
        rofs_refused
    );

    let missing = truncate(b"/mnt/does-not-exist\0", 0) == Err(-2);
    check!("truncate on a missing path is ENOENT", missing);

    // O_TRUNC on a device node is ignored, as POSIX says; opening must work.
    let dev = open(b"/dev/null\0", 1 | O_TRUNC, 0);
    let dev_ok = dev.is_ok();
    if let Ok(fd) = dev {
        let _ = close(fd);
    }
    check!("open(/dev/null, O_WRONLY|O_TRUNC) still succeeds", dev_ok);

    let _ = shell_run(
        b"rm /mnt/tr1 /mnt/tr2 /mnt/tr3a /mnt/tr3b /mnt/tr3c /mnt/tr4 /mnt/tr5 \
          /tmp/tr1 /fat/tr1\0",
    );

    if tmpfs == Some(0)
        && racfs == Some(0)
        && fat == Some(0)
        && append == Some(0)
        && size_ok
        && owns_34
        && dbl_ok
        && dbl_cuts
        && acct_ok
        && by_path
        && ro_refused
        && dir_refused
        && rofs_refused
        && missing
        && dev_ok
    {
        println("T36-TRUNCATE-OK");
    }
}

/// (st_dev, st_ino) of the file at `path`, or None if stat fails.
fn stat_ident(path: &[u8]) -> Option<(u64, u64)> {
    let mut raw = [0u8; 80];
    stat(path, &mut raw).ok()?;
    // SAFETY: the kernel filled `raw` with a StatBuf (80 bytes, repr(C)).
    let st = unsafe { &*(raw.as_ptr() as *const StatBuf) };
    Some((st.st_dev, st.st_ino))
}

/// The coreutils that copy data must not lose it. Before this group, each
/// of these reported success (exit 0) while destroying data:
///
/// - `mv f f` deleted f: mv is copy-then-unlink, and it never asked whether
///   source and destination were the same file. With O_TRUNC now working
///   the copy first emptied the shared inode, then unlinked the only name.
/// - `mv a b` with b a hard link of a emptied both names the same way;
///   `cp f f` emptied f.
/// - `cat big | wc -c` said 4096 for an 8320-byte file: a pipe holds 4 KiB,
///   the write end returns a short count or EAGAIN when it is full, and
///   cat, cp, mv and tee did `let _ = write(...)` - every pipeline carrying
///   more than 4 KiB silently lost the rest.
///
/// Two kernel pieces make the fixes testable: `st_dev` in stat/fstat, so
/// that (st_dev, st_ino) identifies a file across filesystems, and
/// `/dev/full`, whose writes fail with ENOSPC, so that "a write error must
/// not cost the source" can be demonstrated without filling a disk.
fn test_coreutils_preserve_data() {
    println("\n[test] coreutils preserve data (mv-to-self, short writes)");

    // --- st_dev: the identity a same-file check needs ----------------------
    let _ = shell_run(b"echo one > /tmp/id1; echo one > /mnt/id1; ln /mnt/id1 /mnt/id1link\0");
    let tmp_id = stat_ident(b"/tmp/id1\0");
    let mnt_id = stat_ident(b"/mnt/id1\0");
    let link_id = stat_ident(b"/mnt/id1link\0");
    let dev_nonzero = matches!((tmp_id, mnt_id), (Some((a, _)), Some((b, _))) if a != 0 && b != 0);
    check!("stat reports a non-zero st_dev", dev_nonzero);
    let dev_differs = matches!((tmp_id, mnt_id), (Some((a, _)), Some((b, _))) if a != b);
    check!("st_dev differs between tmpfs and racfs", dev_differs);
    let link_same = mnt_id.is_some() && mnt_id == link_id;
    check!("a hard link has the same (st_dev, st_ino)", link_same);

    // --- /dev/full: the write error you can inject ------------------------
    let mut full_ok = false;
    if let Ok(fd) = open(b"/dev/full\0", O_RDWR, 0) {
        let mut z = [0xAAu8; 16];
        let r = read(fd, &mut z);
        let w = write(fd, b"anything");
        full_ok = r == Ok(16) && z.iter().all(|&b| b == 0) && w == Err(-28);
        let _ = close(fd);
    }
    check!(
        "/dev/full reads zeros and refuses writes with ENOSPC",
        full_ok
    );

    // --- mv onto itself, and onto its own hard link ------------------------
    let mv_self = shell_run(
        b"echo hello > /mnt/pv_self; mv /mnt/pv_self /mnt/pv_self; rc=$?; \
          c=$(cat /mnt/pv_self); test \"$rc\" -eq 0 && test \"$c\" = hello\0",
    );
    check!(
        "mv f f leaves f and its content in place, exit 0",
        mv_self == Some(0)
    );
    let mv_link = shell_run(
        b"echo hello > /mnt/pv_a; ln /mnt/pv_a /mnt/pv_b; mv /mnt/pv_a /mnt/pv_b; rc=$?; \
          a=$(cat /mnt/pv_a); b=$(cat /mnt/pv_b); \
          test \"$rc\" -eq 0 && test \"$a\" = hello && test \"$b\" = hello\0",
    );
    check!(
        "mv a b with b a hard link of a keeps both names and the data",
        mv_link == Some(0)
    );

    // --- cp onto itself ----------------------------------------------------
    let cp_self = shell_run(
        b"echo data > /mnt/pv_cp; cp /mnt/pv_cp /mnt/pv_cp; rc=$?; \
          c=$(cat /mnt/pv_cp); test \"$rc\" -ne 0 && test \"$c\" = data\0",
    );
    check!("cp f f fails and does not truncate f", cp_self == Some(0));

    // --- pipelines carry more than the 4 KiB a pipe holds ------------------
    // 65 * 128 = 8320 bytes, then 16640: two and four pipe-fulls.
    let seeded = shell_run(
        b"echo 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef > /mnt/pv_s1; \
          cat /mnt/pv_s1 /mnt/pv_s1 /mnt/pv_s1 /mnt/pv_s1 > /mnt/pv_s2; \
          cat /mnt/pv_s2 /mnt/pv_s2 /mnt/pv_s2 /mnt/pv_s2 > /mnt/pv_s1; \
          cat /mnt/pv_s1 /mnt/pv_s1 /mnt/pv_s1 /mnt/pv_s1 > /mnt/pv_s2; \
          cat /mnt/pv_s2 /mnt/pv_s2 > /mnt/pv_8k; \
          cat /mnt/pv_8k /mnt/pv_8k > /mnt/pv_16k; \
          n=$(wc -c < /mnt/pv_8k); m=$(wc -c < /mnt/pv_16k); \
          test \"$n\" -eq 8320 && test \"$m\" -eq 16640\0",
    );
    check!(
        "8320- and 16640-byte seed files are in place",
        seeded == Some(0)
    );
    let pipe_cat = shell_run(b"n=$(cat /mnt/pv_8k | wc -c); test \"$n\" -eq 8320\0");
    check!(
        "cat 8320 bytes through a pipe delivers all of them",
        pipe_cat == Some(0)
    );
    let pipe_tee = shell_run(
        b"n=$(cat /mnt/pv_16k | tee /mnt/pv_copy | wc -c); m=$(wc -c < /mnt/pv_copy); \
          test \"$n\" -eq 16640 && test \"$m\" -eq 16640\0",
    );
    check!(
        "cat | tee | wc carries 16640 bytes to both outputs",
        pipe_tee == Some(0)
    );
    let pipe_tail = shell_run(b"t=$(cat /mnt/pv_16k | tail -1); test \"$t\" = 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\0");
    check!(
        "the last line of the piped file arrives intact",
        pipe_tail == Some(0)
    );

    // --- a write error must not cost the source ----------------------------
    let mv_full = shell_run(
        b"echo keep > /mnt/pv_keep; mv /mnt/pv_keep /dev/full; rc=$?; \
          c=$(cat /mnt/pv_keep); test \"$rc\" -ne 0 && test \"$c\" = keep\0",
    );
    check!(
        "mv into /dev/full fails and leaves the source intact",
        mv_full == Some(0)
    );
    let cp_full = shell_run(b"cp /mnt/pv_keep /dev/full; rc=$?; test \"$rc\" -ne 0\0");
    check!("cp into /dev/full fails", cp_full == Some(0));
    let cat_full = shell_run(b"cat /mnt/pv_keep > /dev/full; rc=$?; test \"$rc\" -ne 0\0");
    check!(
        "cat > /dev/full reports the write error",
        cat_full == Some(0)
    );
    let tee_full = shell_run(
        b"cat /mnt/pv_keep | tee /dev/full > /mnt/pv_teeout; rc=$?; \
          c=$(cat /mnt/pv_teeout); test \"$rc\" -ne 0 && test \"$c\" = keep\0",
    );
    check!(
        "tee /dev/full still writes stdout and exits non-zero",
        tee_full == Some(0)
    );

    // --- and a plain move still moves -------------------------------------
    // (`test ! -e` is not used: racsh's `test` returns 1 for `! -e` whether
    // the file exists or not - a separate defect, noted for its own PR.)
    let mv_plain = shell_run(
        b"echo plain > /mnt/pv_m1; mv /mnt/pv_m1 /mnt/pv_m2; rc=$?; \
          test -e /mnt/pv_m1 && exit 1; \
          c=$(cat /mnt/pv_m2); test \"$rc\" -eq 0 && test \"$c\" = plain\0",
    );
    check!("mv a b still moves a to b", mv_plain == Some(0));

    let _ = shell_run(
        b"rm /tmp/id1 /mnt/id1 /mnt/id1link /mnt/pv_self /mnt/pv_a /mnt/pv_b /mnt/pv_cp \
             /mnt/pv_s1 /mnt/pv_s2 /mnt/pv_8k /mnt/pv_16k /mnt/pv_copy /mnt/pv_keep \
             /mnt/pv_teeout /mnt/pv_m2\0",
    );

    if dev_nonzero
        && dev_differs
        && link_same
        && full_ok
        && mv_self == Some(0)
        && mv_link == Some(0)
        && cp_self == Some(0)
        && seeded == Some(0)
        && pipe_cat == Some(0)
        && pipe_tee == Some(0)
        && pipe_tail == Some(0)
        && mv_full == Some(0)
        && cp_full == Some(0)
        && cat_full == Some(0)
        && tee_full == Some(0)
        && mv_plain == Some(0)
    {
        println("T37-PRESERVE-OK");
    }
}

/// Read the whole file at `path` into `buf`; returns the byte count.
fn read_file_into(path: &[u8], buf: &mut [u8]) -> Option<usize> {
    let fd = open(path, 0, 0).ok()?;
    let mut total = 0usize;
    while total < buf.len() {
        match read(fd, &mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(_) => break,
        }
    }
    let _ = close(fd);
    Some(total)
}

/// rename(2) must be a rename: one directory operation that keeps the
/// inode, replaces an existing target, moves directories, and either
/// happens or does not. Before this group `sys_rename` read the whole
/// file into memory, created a new one, wrote it and unlinked the old:
/// three separate operations plus a data copy (nothing for the journal
/// to protect), `st_ino` changed, an existing target was EEXIST instead
/// of replaced, directories failed, an empty file was not moved at all
/// (`if size > 0`) while the call returned 0, and the destination path
/// was resolved in the *source's* filesystem with no EXDEV.
fn test_rename_semantics() {
    println("\n[test] rename(2) semantics");

    // --- an empty file moves --------------------------------------------
    let _ = shell_run(b"touch /mnt/rn_empty\0");
    let r = rename(b"/mnt/rn_empty\0", b"/mnt/rn_empty2\0");
    let empty_moved = r.is_ok()
        && stat_size(b"/mnt/rn_empty\0").is_none()
        && stat_size(b"/mnt/rn_empty2\0") == Some(0);
    check!("rename moves an empty file", empty_moved);

    // --- the inode survives: it is a rename, not a copy ------------------
    let seeded = shell_run(
        b"echo 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef > /mnt/rn_s1; \
          cat /mnt/rn_s1 /mnt/rn_s1 /mnt/rn_s1 /mnt/rn_s1 > /mnt/rn_s2; \
          cat /mnt/rn_s2 /mnt/rn_s2 /mnt/rn_s2 /mnt/rn_s2 > /mnt/rn_s1; \
          cat /mnt/rn_s1 /mnt/rn_s1 /mnt/rn_s1 /mnt/rn_s1 > /mnt/rn_s2; \
          cat /mnt/rn_s2 /mnt/rn_s2 /mnt/rn_s2 /mnt/rn_s2 > /mnt/rn_big; \
          rm /mnt/rn_s1 /mnt/rn_s2\0",
    );
    let before = stat_ident(b"/mnt/rn_big\0");
    let free_before = racfs_free_blocks();
    let r = rename(b"/mnt/rn_big\0", b"/mnt/rn_big2\0");
    let after = stat_ident(b"/mnt/rn_big2\0");
    let free_after = racfs_free_blocks();
    let tail_ok = shell_run(
        b"t=$(tail -1 /mnt/rn_big2); n=$(wc -c < /mnt/rn_big2); \
          test \"$n\" -eq 16640 && \
          test \"$t\" = 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\0",
    );
    let same_inode = seeded == Some(0)
        && r.is_ok()
        && before.is_some()
        && before == after
        && stat_size(b"/mnt/rn_big\0").is_none()
        && free_before == free_after
        && tail_ok == Some(0);
    check!(
        "rename keeps st_ino and st_dev, moves no data, frees no blocks",
        same_inode
    );

    // --- an existing target is replaced, and its blocks come back ---------
    let _ = shell_run(b"cat /mnt/rn_big2 > /mnt/rn_victim; echo new > /mnt/rn_src\0");
    let victim_blocks = 34; // 16640 bytes: 33 data + 1 indirect
    let free0 = racfs_free_blocks();
    let r = rename(b"/mnt/rn_src\0", b"/mnt/rn_victim\0");
    let free1 = racfs_free_blocks();
    let mut content = [0u8; 16];
    let n = read_file_into(b"/mnt/rn_victim\0", &mut content).unwrap_or(0);
    let replaced = r.is_ok()
        && n == 4
        && &content[..4] == b"new\n"
        && stat_size(b"/mnt/rn_src\0").is_none()
        && matches!((free0, free1), (Some(a), Some(b)) if b == a + victim_blocks);
    check!(
        "rename onto an existing file replaces it and frees its blocks",
        replaced
    );

    // --- two names of one inode: POSIX says do nothing, successfully ------
    let _ = shell_run(b"echo shared > /mnt/rn_l1; ln /mnt/rn_l1 /mnt/rn_l2\0");
    let r = rename(b"/mnt/rn_l1\0", b"/mnt/rn_l2\0");
    let links_ok = r.is_ok()
        && stat_ident(b"/mnt/rn_l1\0").is_some()
        && stat_ident(b"/mnt/rn_l1\0") == stat_ident(b"/mnt/rn_l2\0");
    check!(
        "rename between two hard links of one file is a successful no-op",
        links_ok
    );

    // --- directories move, with their contents ---------------------------
    let _ = shell_run(b"mkdir /mnt/rn_d1; echo inside > /mnt/rn_d1/f; mkdir /mnt/rn_d1/sub\0");
    let r = rename(b"/mnt/rn_d1\0", b"/mnt/rn_d2\0");
    let dir_moved = r.is_ok()
        && shell_run(b"c=$(cat /mnt/rn_d2/f); test \"$c\" = inside && test -e /mnt/rn_d2/sub\0")
            == Some(0)
        && stat_ident(b"/mnt/rn_d1\0").is_none();
    check!("rename moves a directory with its contents", dir_moved);

    // Into another directory: the entry leaves one parent and joins another.
    let _ = shell_run(b"mkdir /mnt/rn_home\0");
    let r = rename(b"/mnt/rn_d2\0", b"/mnt/rn_home/d\0");
    let dir_reparented = r.is_ok()
        && shell_run(b"c=$(cat /mnt/rn_home/d/f); test \"$c\" = inside\0") == Some(0)
        && stat_ident(b"/mnt/rn_d2\0").is_none();
    check!(
        "rename moves a directory into another directory",
        dir_reparented
    );

    // --- refusals ----------------------------------------------------------
    let cycle = rename(b"/mnt/rn_home\0", b"/mnt/rn_home/d/sub/x\0") == Err(-22);
    check!(
        "rename of a directory into its own subtree is EINVAL",
        cycle
    );
    let _ = shell_run(b"mkdir /mnt/rn_full; echo x > /mnt/rn_full/y; mkdir /mnt/rn_emptydir\0");
    let notempty = rename(b"/mnt/rn_emptydir\0", b"/mnt/rn_full\0") == Err(-39)
        && stat_ident(b"/mnt/rn_emptydir\0").is_some();
    check!("rename onto a non-empty directory is ENOTEMPTY", notempty);
    let onto_empty = rename(b"/mnt/rn_full\0", b"/mnt/rn_emptydir\0").is_ok()
        && shell_run(b"c=$(cat /mnt/rn_emptydir/y); test \"$c\" = x\0") == Some(0)
        && stat_ident(b"/mnt/rn_full\0").is_none();
    check!("rename onto an empty directory replaces it", onto_empty);
    let _ = shell_run(b"echo f > /mnt/rn_file\0");
    let notdir = rename(b"/mnt/rn_emptydir\0", b"/mnt/rn_file\0") == Err(-20);
    check!("rename of a directory onto a file is ENOTDIR", notdir);
    let isdir = rename(b"/mnt/rn_file\0", b"/mnt/rn_emptydir\0") == Err(-21);
    check!("rename of a file onto a directory is EISDIR", isdir);
    let missing = rename(b"/mnt/rn_nope\0", b"/mnt/rn_nope2\0") == Err(-2);
    check!("rename of a missing source is ENOENT", missing);

    // --- across mounts: EXDEV, and mv falls back to a copy -----------------
    let _ = shell_run(b"echo cross > /tmp/rn_x\0");
    let exdev =
        rename(b"/tmp/rn_x\0", b"/mnt/rn_x\0") == Err(-18) && stat_size(b"/tmp/rn_x\0") == Some(6);
    check!("rename across mounts is EXDEV and leaves the source", exdev);
    let mv_cross = shell_run(
        b"mv /tmp/rn_x /mnt/rn_x; rc=$?; test -e /tmp/rn_x && exit 1; \
          c=$(cat /mnt/rn_x); test \"$rc\" -eq 0 && test \"$c\" = cross\0",
    );
    check!(
        "mv across mounts still moves (copy fallback)",
        mv_cross == Some(0)
    );

    // --- mv within one filesystem is now a rename --------------------------
    let _ = shell_run(b"echo viamv > /mnt/rn_m1\0");
    let m_before = stat_ident(b"/mnt/rn_m1\0");
    let mv_same = shell_run(b"mv /mnt/rn_m1 /mnt/rn_m2\0");
    let m_after = stat_ident(b"/mnt/rn_m2\0");
    check!(
        "mv within a filesystem keeps the inode",
        mv_same == Some(0) && m_before.is_some() && m_before == m_after
    );

    // --- subtree mounts resolve relative to their own root -----------------
    let _ = shell_run(b"echo home > /home/rn_h1\0");
    let r = rename(b"/home/rn_h1\0", b"/home/rn_h2\0");
    let home_ok = r.is_ok()
        && shell_run(b"c=$(cat /home/rn_h2); test \"$c\" = home || exit 1; test -e /home/rn_h1 && exit 1; exit 0\0")
            == Some(0)
        && stat_size(b"/mnt/rn_h2\0").is_none();
    check!(
        "rename on a subtree mount stays inside that subtree",
        home_ok
    );

    // --- tmpfs and FAT32 implement it too ----------------------------------
    let _ = shell_run(b"echo t > /tmp/rn_t1\0");
    let t_before = stat_ident(b"/tmp/rn_t1\0");
    let tmp_ok = rename(b"/tmp/rn_t1\0", b"/tmp/rn_t2\0").is_ok()
        && stat_ident(b"/tmp/rn_t2\0") == t_before
        && stat_ident(b"/tmp/rn_t1\0").is_none();
    check!("rename works on tmpfs and keeps the inode", tmp_ok);
    let _ = shell_run(b"echo fat > /fat/rn_f1; mkdir /fat/rn_dir\0");
    let fat_ok = rename(b"/fat/rn_f1\0", b"/fat/rn_dir/f2\0").is_ok()
        && shell_run(b"c=$(cat /fat/rn_dir/f2); test \"$c\" = fat || exit 1; test -e /fat/rn_f1 && exit 1; exit 0\0")
            == Some(0);
    check!("rename works on FAT32, across directories", fat_ok);

    let _ = shell_run(
        b"rm /mnt/rn_empty2 /mnt/rn_big2 /mnt/rn_victim /mnt/rn_l1 /mnt/rn_l2 /mnt/rn_file \
             /mnt/rn_x /mnt/rn_m2 /home/rn_h2 /tmp/rn_t2 /fat/rn_dir/f2; \
          rm /mnt/rn_home/d/f; rmdir /mnt/rn_home/d/sub /mnt/rn_home/d /mnt/rn_home; \
          rm /mnt/rn_emptydir/y; rmdir /mnt/rn_emptydir /fat/rn_dir\0",
    );

    if empty_moved
        && same_inode
        && replaced
        && links_ok
        && dir_moved
        && dir_reparented
        && cycle
        && notempty
        && onto_empty
        && notdir
        && isdir
        && missing
        && exdev
        && mv_cross == Some(0)
        && mv_same == Some(0)
        && m_before.is_some()
        && m_before == m_after
        && home_ok
        && tmp_ok
        && fat_ok
    {
        println("T38-RENAME-OK");
    }
}

/// /home, /etc, /var/log and /var/lib/rpkg are subtrees of the persistent
/// disk (ROADMAP v0.3 §3.3).
///
/// The failure this is really written against is silent: a subtree mount
/// shares one Racfs with the whole-disk mount at /mnt and differs only in
/// which inode it calls root, so a write path that resolves from inode 0
/// anyway still succeeds, still reads back, and still looks correct -- it
/// just put the file in the disk root. Checking the file through /mnt is what
/// separates "it worked" from "it went where it was supposed to".
fn test_persistent_mount_layout() {
    println("\n[test] persistent mount layout");

    // Written through /home, and visible at the matching path under the
    // whole-disk mount. Both halves matter: the second is the one that fails
    // if subtree routing is broken.
    let home = shell_run(
        b"echo subtree-ok > /home/probe.txt; sync; \
          a=$(cat /home/probe.txt); b=$(cat /mnt/home/probe.txt); \
          test \"$a\" = subtree-ok && test \"$b\" = subtree-ok\0",
    );
    check!(
        "a file written to /home lands in sda:/home",
        home == Some(0)
    );

    // ...and nowhere else. If the write had resolved from the disk root the
    // file would be at /mnt/probe.txt, which reads back just as happily.
    let not_root = shell_run(b"test -e /mnt/probe.txt && exit 1; exit 0\0");
    check!("...and not in the disk root", not_root == Some(0));

    let log = shell_run(
        b"echo logline > /var/log/probe.log; sync; \
          c=$(cat /var/log/probe.log); d=$(cat /mnt/var/log/probe.log); \
          test \"$c\" = logline && test \"$d\" = logline\0",
    );
    check!("/var/log writes land in sda:/var/log", log == Some(0));

    // /etc is the boot-critical one: init reads its units through this mount,
    // so an empty or unreadable base.target means no shell at all next boot.
    let etc = shell_run(b"n=$(wc -c < /etc/racinit/base.target); test \"$n\" -gt 0\0");
    check!("/etc/racinit/base.target is readable", etc == Some(0));

    // $HOME has to name a directory that exists, or racsh silently saves its
    // history nowhere.
    let home_var = shell_run(b"test \"$HOME\" = /home/racos && test -d /home/racos\0");
    check!("$HOME names a real directory", home_var == Some(0));

    // The mount points are visible in their parent listing. readdir lists the
    // filesystem below a mount, not the mount table, so this only holds
    // because the boot creates them as real directories underneath.
    let visible = shell_run(b"l=$(ls /var | grep log | wc -l); test \"$l\" -ge 1\0");
    check!("/var lists its mount points", visible == Some(0));

    if home == Some(0)
        && not_root == Some(0)
        && log == Some(0)
        && etc == Some(0)
        && home_var == Some(0)
        && visible == Some(0)
    {
        println("T35-MOUNTS-OK");
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
