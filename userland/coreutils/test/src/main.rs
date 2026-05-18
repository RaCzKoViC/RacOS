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
            unsafe { PASS += 1; }
            print("  [PASS] ");
            println($name);
        } else {
            unsafe { FAIL += 1; }
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
    test_spawn_wait();
    test_chdir_getcwd();
    test_security_syscalls();

    println("");
    let (pass, fail) = unsafe { (PASS, FAIL) };
    print("=== Results: ");
    print_u32(pass);
    print(" passed, ");
    print_u32(fail);
    println(" failed ===");

    if fail > 0 { 1 } else { 0 }
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

fn test_spawn_wait() {
    println("\n[test] spawn/wait");
    let pid = spawn(b"bin/true\0");
    check!("spawn bin/true returns Ok", pid.is_ok());
    if pid.is_ok() {
        let mut status: i32 = -1;
        let ret = wait(&mut status);
        check!("wait returns child pid", ret.is_ok());
        check!("child exit status is 0", status == 0);
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
