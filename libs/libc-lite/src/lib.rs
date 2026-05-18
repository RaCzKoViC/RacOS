//! libc-lite — Minimalna biblioteka systemowa dla RacOS userland
//!
//! Dostarcza:
//! - punkt wejścia `_start` wywołujący `main` użytkownika
//! - wrappery inline-asm dla instrukcji SYSCALL
//! - publiczne funkcje syscall (exit, read, write, open, close, ...)
//! - panic handler dla userland

#![no_std]

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ─────────────────────────────────────────────────
// Numery syscalli (KERNEL_ABI.md §5)
// ─────────────────────────────────────────────────

pub const SYS_EXIT: u64 = 0;
pub const SYS_READ: u64 = 1;
pub const SYS_WRITE: u64 = 2;
pub const SYS_OPEN: u64 = 3;
pub const SYS_CLOSE: u64 = 4;
pub const SYS_STAT: u64 = 5;
pub const SYS_MMAP: u64 = 6;
pub const SYS_MUNMAP: u64 = 7;
pub const SYS_PIPE: u64 = 8;
pub const SYS_DUP: u64 = 9;
pub const SYS_DUP2: u64 = 10;
pub const SYS_EXEC: u64 = 11;
pub const SYS_SPAWN: u64 = 12;
pub const SYS_WAIT: u64 = 13;
pub const SYS_GETPID: u64 = 14;
pub const SYS_CHDIR: u64 = 15;
pub const SYS_IOCTL: u64 = 16;
pub const SYS_KILL: u64 = 17;
pub const SYS_GETCWD: u64 = 18;
pub const SYS_SETPGID: u64 = 19;
pub const SYS_GETPGID: u64 = 20;
pub const SYS_SETSID: u64 = 21;
pub const SYS_CLOCK_GETTIME: u64 = 22;
pub const SYS_GETDENTS: u64 = 23;
pub const SYS_MKDIR: u64 = 24;
pub const SYS_UNLINK: u64 = 25;
pub const SYS_FORK: u64 = 26;
pub const SYS_SIGACTION: u64 = 27;
pub const SYS_SIGRETURN: u64 = 28;
pub const SYS_POLL: u64 = 29;
pub const SYS_GETPPID: u64 = 30;
pub const SYS_GETUID: u64 = 31;
pub const SYS_GETGID: u64 = 32;
pub const SYS_SETUID: u64 = 33;
pub const SYS_SETGID: u64 = 34;
pub const SYS_GETEUID: u64 = 35;
pub const SYS_GETEGID: u64 = 36;
pub const SYS_NANOSLEEP: u64 = 37;
pub const SYS_TRUNCATE: u64 = 38;
pub const SYS_FSTAT: u64 = 39;
pub const SYS_LSEEK: u64 = 40;
pub const SYS_ACCESS: u64 = 41;
pub const SYS_CHMOD: u64 = 42;
pub const SYS_CHOWN: u64 = 43;
pub const SYS_UMASK: u64 = 44;
pub const SYS_LINK: u64 = 45;
pub const SYS_SYMLINK: u64 = 46;
pub const SYS_READLINK: u64 = 47;
pub const SYS_RENAME: u64 = 48;
pub const SYS_FCNTL: u64 = 49;
pub const SYS_ISATTY: u64 = 50;
pub const SYS_SOCKET: u64 = 51;
pub const SYS_BIND: u64 = 52;
pub const SYS_LISTEN: u64 = 53;
pub const SYS_ACCEPT: u64 = 54;
pub const SYS_CONNECT: u64 = 55;
pub const SYS_SEND: u64 = 56;
pub const SYS_RECV: u64 = 57;
pub const SYS_SHUTDOWN: u64 = 58;
pub const SYS_GETSOCKNAME: u64 = 59;
pub const SYS_GETPEERNAME: u64 = 60;
pub const SYS_SETSOCKOPT: u64 = 61;
pub const SYS_GETSOCKOPT: u64 = 62;
pub const SYS_WAITPID: u64 = 63;
pub const SYS_PIPE2: u64 = 64;
pub const SYS_UNAME: u64 = 65;
pub const SYS_MOUNT: u64 = 66;
pub const SYS_UMOUNT: u64 = 67;
pub const SYS_MPROTECT: u64 = 68;
pub const SYS_FSYNC: u64 = 69;
pub const SYS_FTRUNCATE: u64 = 70;
pub const SYS_WRITEV: u64 = 71;
pub const SYS_READV: u64 = 72;
pub const SYS_SCHED_YIELD: u64 = 73;
pub const SYS_REBOOT: u64 = 74;
pub const SYS_HOSTNAME: u64 = 75;
pub const SYS_GETRANDOM: u64 = 76;
pub const SYS_CLONE: u64 = 77;
pub const SYS_PTHREAD_CREATE: u64 = 0x400;

// ─────────────────────────────────────────────────
// Surowe wywołania syscall (inline asm)
// ─────────────────────────────────────────────────
// Konwencja x86_64 SYSCALL:
//   RAX = numer, RDI/RSI/RDX/R10/R8/R9 = argumenty
//   Zwrot w RAX. RCX i R11 niszczone przez CPU.

#[inline(always)]
pub unsafe fn syscall0(nr: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        lateout("rax") ret,
        clobber_abi("sysv64"),
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall1(nr: u64, a1: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        lateout("rax") ret,
        clobber_abi("sysv64"),
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall2(nr: u64, a1: u64, a2: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        in("rsi") a2,
        lateout("rax") ret,
        clobber_abi("sysv64"),
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rax") ret,
        clobber_abi("sysv64"),
        options(nostack),
    );
    ret
}

// ─────────────────────────────────────────────────
// Wrappery publiczne (API standardowe)
// ─────────────────────────────────────────────────

pub fn pthread_create(routine: u64, arg: u64) -> i64 {
    unsafe { syscall2(SYS_PTHREAD_CREATE, routine, arg) }
}

#[inline(always)]
pub unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        lateout("rax") ret,
        clobber_abi("sysv64"),
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall5(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        lateout("rax") ret,
        clobber_abi("sysv64"),
        options(nostack),
    );
    ret
}

#[inline(always)]
pub unsafe fn syscall6(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        in("r9") a6,
        lateout("rax") ret,
        clobber_abi("sysv64"),
        options(nostack),
    );
    ret
}

// ─────────────────────────────────────────────────
// Publiczne API syscalli
// ─────────────────────────────────────────────────

/// Zakończ proces z kodem wyjścia.
pub fn exit(status: i32) -> ! {
    unsafe { syscall1(SYS_EXIT, status as u64); }
    loop {}
}

/// Odczytaj z deskryptora pliku.
pub fn read(fd: i32, buf: &mut [u8]) -> Result<usize, i64> {
    let ret = unsafe { syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as usize) }
}

/// Zapisz do deskryptora pliku.
pub fn write(fd: i32, buf: &[u8]) -> Result<usize, i64> {
    let ret = unsafe { syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as usize) }
}

/// Otwórz plik.
pub fn open(path: &[u8], flags: u32, mode: u32) -> Result<i32, i64> {
    let ret = unsafe { syscall3(SYS_OPEN, path.as_ptr() as u64, flags as u64, mode as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Zamknij deskryptor pliku.
pub fn close(fd: i32) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_CLOSE, fd as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Pobierz informacje o pliku.
pub fn stat(path: &[u8], buf: &mut [u8; 80]) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_STAT, path.as_ptr() as u64, buf.as_mut_ptr() as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Utwórz anonimowy pipe.
pub fn pipe(fds: &mut [i32; 2]) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_PIPE, fds.as_mut_ptr() as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Zduplikuj deskryptor pliku.
pub fn dup(oldfd: i32) -> Result<i32, i64> {
    let ret = unsafe { syscall1(SYS_DUP, oldfd as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Zduplikuj deskryptor na konkretny numer.
pub fn dup2(oldfd: i32, newfd: i32) -> Result<i32, i64> {
    let ret = unsafe { syscall2(SYS_DUP2, oldfd as u64, newfd as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Uruchom nowy program w bieżącym procesie (zastępuje obraz).
pub fn exec(path: &[u8]) -> Result<(), i64> {
    let ret = unsafe { syscall3(SYS_EXEC, path.as_ptr() as u64, 0, 0) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Utwórz nowy proces potomny z ELF.
pub fn spawn(path: &[u8]) -> Result<i32, i64> {
    let ret = unsafe { syscall3(SYS_SPAWN, path.as_ptr() as u64, 0, 0) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Utwórz nowy proces potomny z ELF, przekazując argumenty.
/// `argv` to tablica wskaźników do null-terminated stringów, zakończona NULL.
pub fn spawn_args(path: &[u8], argv: &[*const u8]) -> Result<i32, i64> {
    let ret = unsafe { syscall3(SYS_SPAWN, path.as_ptr() as u64, argv.as_ptr() as u64, 0) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Czekaj na zakończenie procesu potomnego.
/// Zwraca (pid, exit_status).
pub fn wait(status: &mut i32) -> Result<i32, i64> {
    let ret = unsafe { syscall3(SYS_WAIT, -1i32 as u64, status as *mut i32 as u64, 0) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Pobierz PID bieżącego procesu.
pub fn getpid() -> i32 {
    unsafe { syscall0(SYS_GETPID) as i32 }
}

/// Wyślij sygnał do procesu.
pub fn kill(pid: i32, sig: i32) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_KILL, pid as u64, sig as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Zmień bieżący katalog roboczy.
pub fn chdir(path: &[u8]) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_CHDIR, path.as_ptr() as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Pobierz bieżący katalog roboczy.
pub fn getcwd(buf: &mut [u8]) -> Result<usize, i64> {
    let ret = unsafe { syscall2(SYS_GETCWD, buf.as_mut_ptr() as u64, buf.len() as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as usize) }
}

/// Ustaw grupę procesów.
pub fn setpgid(pid: u32, pgid: u32) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_SETPGID, pid as u64, pgid as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Pobierz grupę procesów.
pub fn getpgid(pid: u32) -> Result<u32, i64> {
    let ret = unsafe { syscall1(SYS_GETPGID, pid as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as u32) }
}

/// Utwórz nową sesję.
pub fn setsid() -> Result<u32, i64> {
    let ret = unsafe { syscall0(SYS_SETSID) };
    if ret < 0 { Err(ret) } else { Ok(ret as u32) }
}

/// Kontrola urządzenia I/O.
pub fn ioctl(fd: i32, request: u32, arg: u64) -> Result<i64, i64> {
    let ret = unsafe { syscall3(SYS_IOCTL, fd as u64, request as u64, arg) };
    if ret < 0 { Err(ret) } else { Ok(ret) }
}

/// Timespec — czas w sekundach + nanosekundach.
#[repr(C)]
pub struct Timespec {
    pub tv_sec: u64,
    pub tv_nsec: u64,
}

pub const CLOCK_REALTIME: u32 = 0;
pub const CLOCK_MONOTONIC: u32 = 1;

/// Pobierz aktualny czas z zegara `clock_id`.
pub fn clock_gettime(clock_id: u32, tp: &mut Timespec) -> Result<(), i64> {
    let ret = unsafe {
        syscall2(SYS_CLOCK_GETTIME, clock_id as u64, tp as *mut Timespec as u64)
    };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Wpis katalogowy z getdents.
pub struct DirEntry {
    pub ino: u64,
    pub file_type: u8,
    pub name_len: u8,
    pub name: [u8; 255],
}

/// Odczytaj wpisy katalogowe z otwartego fd katalogu.
/// Zwraca liczbę bajtów (0 = koniec).
pub fn getdents(fd: i32, buf: &mut [u8]) -> Result<usize, i64> {
    let ret = unsafe {
        syscall3(SYS_GETDENTS, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
    };
    if ret < 0 { Err(ret) } else { Ok(ret as usize) }
}

/// Utwórz katalog.
pub fn mkdir(path: &[u8], mode: u32) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_MKDIR, path.as_ptr() as u64, mode as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Usuń plik lub pusty katalog.
pub fn unlink(path: &[u8]) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_UNLINK, path.as_ptr() as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Fork the current process. Returns 0 in child, child PID in parent.
pub fn fork() -> Result<i32, i64> {
    let ret = unsafe { syscall0(SYS_FORK) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Clone flags.
pub const CLONE_VM: u32 = 0x00000100;
pub const CLONE_THREAD: u32 = 0x00010000;

/// Clone the current process/thread.
/// For threads: clone(CLONE_VM | CLONE_THREAD, stack, 0, 0, 0)
pub fn clone(flags: u32, stack: *mut u8, ptid: i32, tls: i32, ctid: *mut u8) -> Result<i32, i64> {
    let ret = unsafe { syscall5(SYS_CLONE, flags as u64, stack as u64, ptid as u64, tls as u64, ctid as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Signal action structure.
#[repr(C)]
pub struct SigAction {
    pub handler: u64,
    pub flags: u32,
    pub mask: u32,
}

pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;

/// Install a signal handler.
pub fn sigaction(sig: i32, act: Option<&SigAction>, oldact: Option<&mut SigAction>) -> Result<(), i64> {
    let act_ptr = act.map(|a| a as *const SigAction as u64).unwrap_or(0);
    let oldact_ptr = oldact.map(|a| a as *mut SigAction as u64).unwrap_or(0);
    let ret = unsafe { syscall3(SYS_SIGACTION, sig as u64, act_ptr, oldact_ptr) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// PollFd for poll().
#[repr(C)]
pub struct PollFd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

pub const POLLIN: i16 = 0x0001;
pub const POLLOUT: i16 = 0x0004;
pub const POLLERR: i16 = 0x0008;
pub const POLLHUP: i16 = 0x0010;
pub const POLLNVAL: i16 = 0x0020;

/// Poll file descriptors.
pub fn poll(fds: &mut [PollFd], timeout_ms: i32) -> Result<i32, i64> {
    let ret = unsafe {
        syscall3(SYS_POLL, fds.as_mut_ptr() as u64, fds.len() as u64, timeout_ms as u64)
    };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

/// Get parent PID.
pub fn getppid() -> i32 {
    unsafe { syscall0(SYS_GETPPID) as i32 }
}

/// Get real UID.
pub fn getuid() -> u32 {
    unsafe { syscall0(SYS_GETUID) as u32 }
}

/// Get real GID.
pub fn getgid() -> u32 {
    unsafe { syscall0(SYS_GETGID) as u32 }
}

/// Set UID.
pub fn setuid(uid: u32) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_SETUID, uid as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Set GID.
pub fn setgid(gid: u32) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_SETGID, gid as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Get effective UID.
pub fn geteuid() -> u32 {
    unsafe { syscall0(SYS_GETEUID) as u32 }
}

/// Get effective GID.
pub fn getegid() -> u32 {
    unsafe { syscall0(SYS_GETEGID) as u32 }
}

/// Sleep for specified duration.
pub fn nanosleep(sec: u64, nsec: u64) -> Result<(), i64> {
    let ts = Timespec { tv_sec: sec, tv_nsec: nsec };
    let ret = unsafe { syscall2(SYS_NANOSLEEP, &ts as *const Timespec as u64, 0) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Sleep for given milliseconds.
pub fn sleep_ms(ms: u64) -> Result<(), i64> {
    nanosleep(ms / 1000, (ms % 1000) * 1_000_000)
}

/// Stat a file by fd.
pub fn fstat(fd: i32, buf: &mut [u8; 80]) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_FSTAT, fd as u64, buf.as_mut_ptr() as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Seek within a file.
pub fn lseek(fd: i32, offset: i64, whence: i32) -> Result<i64, i64> {
    let ret = unsafe { syscall3(SYS_LSEEK, fd as u64, offset as u64, whence as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret) }
}
pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;

/// Check file accessibility.
pub fn access(path: &[u8], mode: u32) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_ACCESS, path.as_ptr() as u64, mode as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}
pub const F_OK: u32 = 0;
pub const R_OK: u32 = 4;
pub const W_OK: u32 = 2;
pub const X_OK: u32 = 1;

/// Change file permissions.
pub fn chmod(path: &[u8], mode: u32) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_CHMOD, path.as_ptr() as u64, mode as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Change file ownership.
pub fn chown(path: &[u8], uid: u32, gid: u32) -> Result<(), i64> {
    let ret = unsafe { syscall3(SYS_CHOWN, path.as_ptr() as u64, uid as u64, gid as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Set file creation mask.
pub fn umask(mask: u32) -> u32 {
    unsafe { syscall1(SYS_UMASK, mask as u64) as u32 }
}

/// Rename a file.
pub fn rename(old: &[u8], new: &[u8]) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_RENAME, old.as_ptr() as u64, new.as_ptr() as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// File control.
pub fn fcntl(fd: i32, cmd: i32, arg: u64) -> Result<i64, i64> {
    let ret = unsafe { syscall3(SYS_FCNTL, fd as u64, cmd as u64, arg) };
    if ret < 0 { Err(ret) } else { Ok(ret) }
}

/// Check if fd is a terminal.
pub fn isatty(fd: i32) -> bool {
    unsafe { syscall1(SYS_ISATTY, fd as u64) > 0 }
}

/// Wait for specific PID.
pub fn waitpid(pid: i32, status: &mut i32, options: u32) -> Result<i32, i64> {
    let ret = unsafe {
        syscall3(SYS_WAITPID, pid as u64, status as *mut i32 as u64, options as u64)
    };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}
pub const WNOHANG: u32 = 1;

/// UTS name buffer (5 × 65 bytes).
#[repr(C)]
pub struct UtsName {
    pub sysname: [u8; 65],
    pub nodename: [u8; 65],
    pub release: [u8; 65],
    pub version: [u8; 65],
    pub machine: [u8; 65],
}

impl UtsName {
    pub const fn zeroed() -> Self {
        UtsName {
            sysname: [0; 65],
            nodename: [0; 65],
            release: [0; 65],
            version: [0; 65],
            machine: [0; 65],
        }
    }
}

/// Get system name information.
pub fn uname(buf: &mut UtsName) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_UNAME, buf as *mut UtsName as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Send fsync to ensure data is written.
pub fn fsync(fd: i32) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_FSYNC, fd as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Yield the CPU.
pub fn sched_yield() -> Result<(), i64> {
    let ret = unsafe { syscall0(SYS_SCHED_YIELD) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Reboot the system.
pub fn reboot(cmd: u32) -> Result<(), i64> {
    let ret = unsafe { syscall1(SYS_REBOOT, cmd as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}
pub const REBOOT_POWER_OFF: u32 = 0x4321;
pub const REBOOT_RESTART: u32 = 0x1234;

/// Get or set hostname.
pub fn hostname_get(buf: &mut [u8]) -> Result<usize, i64> {
    let ret = unsafe { syscall3(SYS_HOSTNAME, buf.as_mut_ptr() as u64, buf.len() as u64, 0) };
    if ret < 0 { Err(ret) } else { Ok(ret as usize) }
}

pub fn hostname_set(name: &[u8]) -> Result<(), i64> {
    let ret = unsafe { syscall3(SYS_HOSTNAME, name.as_ptr() as u64, name.len() as u64, 1) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

/// Get random bytes.
pub fn getrandom(buf: &mut [u8]) -> Result<usize, i64> {
    let ret = unsafe { syscall3(SYS_GETRANDOM, buf.as_mut_ptr() as u64, buf.len() as u64, 0) };
    if ret < 0 { Err(ret) } else { Ok(ret as usize) }
}

/// Memory-protect a region.
pub fn mprotect(addr: u64, len: usize, prot: u32) -> Result<(), i64> {
    let ret = unsafe { syscall3(SYS_MPROTECT, addr, len as u64, prot as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}
pub const PROT_NONE: u32 = 0;
pub const PROT_READ: u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC: u32 = 4;

// ─────────────────────────────────────────────────
// Networking helpers (Phase D MVP)
// ─────────────────────────────────────────────────

pub const AF_INET: i32 = 2;
pub const SOCK_STREAM: i32 = 1;

pub const SHUT_RD: i32 = 0;
pub const SHUT_WR: i32 = 1;
pub const SHUT_RDWR: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SockAddrIn {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: u32,
}

impl SockAddrIn {
    pub const fn new_loopback(port: u16) -> Self {
        SockAddrIn {
            // Kernel expects LE family and BE port/ip bytes in memory.
            sin_family: AF_INET as u16,
            sin_port: port.to_be(),
            sin_addr: 0x7F00_0001u32.to_be(),
        }
    }

    pub fn port_host(&self) -> u16 {
        u16::from_be(self.sin_port)
    }

    pub fn ip_host(&self) -> u32 {
        u32::from_be(self.sin_addr)
    }
}

pub fn socket(domain: i32, stype: i32, protocol: i32) -> Result<i32, i64> {
    let ret = unsafe { syscall3(SYS_SOCKET, domain as u64, stype as u64, protocol as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

pub fn bind(fd: i32, addr: &SockAddrIn) -> Result<(), i64> {
    let ret = unsafe {
        syscall3(
            SYS_BIND,
            fd as u64,
            addr as *const SockAddrIn as u64,
            core::mem::size_of::<SockAddrIn>() as u64,
        )
    };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

pub fn listen(fd: i32, backlog: i32) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_LISTEN, fd as u64, backlog as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

pub fn accept(fd: i32, addr: Option<&mut SockAddrIn>) -> Result<i32, i64> {
    let mut len = core::mem::size_of::<SockAddrIn>() as u32;
    let (addr_ptr, len_ptr) = if let Some(a) = addr {
        (a as *mut SockAddrIn as u64, &mut len as *mut u32 as u64)
    } else {
        (0u64, 0u64)
    };
    let ret = unsafe { syscall3(SYS_ACCEPT, fd as u64, addr_ptr, len_ptr) };
    if ret < 0 { Err(ret) } else { Ok(ret as i32) }
}

pub fn connect(fd: i32, addr: &SockAddrIn) -> Result<(), i64> {
    let ret = unsafe {
        syscall3(
            SYS_CONNECT,
            fd as u64,
            addr as *const SockAddrIn as u64,
            core::mem::size_of::<SockAddrIn>() as u64,
        )
    };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

pub fn send(fd: i32, buf: &[u8], flags: u32) -> Result<usize, i64> {
    let ret = unsafe { syscall4(SYS_SEND, fd as u64, buf.as_ptr() as u64, buf.len() as u64, flags as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as usize) }
}

pub fn recv(fd: i32, buf: &mut [u8], flags: u32) -> Result<usize, i64> {
    let ret = unsafe { syscall4(SYS_RECV, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64, flags as u64) };
    if ret < 0 { Err(ret) } else { Ok(ret as usize) }
}

pub fn shutdown(fd: i32, how: i32) -> Result<(), i64> {
    let ret = unsafe { syscall2(SYS_SHUTDOWN, fd as u64, how as u64) };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

pub fn getsockname(fd: i32, addr: &mut SockAddrIn) -> Result<(), i64> {
    let mut len = core::mem::size_of::<SockAddrIn>() as u32;
    let ret = unsafe {
        syscall3(
            SYS_GETSOCKNAME,
            fd as u64,
            addr as *mut SockAddrIn as u64,
            &mut len as *mut u32 as u64,
        )
    };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

pub fn getpeername(fd: i32, addr: &mut SockAddrIn) -> Result<(), i64> {
    let mut len = core::mem::size_of::<SockAddrIn>() as u32;
    let ret = unsafe {
        syscall3(
            SYS_GETPEERNAME,
            fd as u64,
            addr as *mut SockAddrIn as u64,
            &mut len as *mut u32 as u64,
        )
    };
    if ret < 0 { Err(ret) } else { Ok(()) }
}

// ─────────────────────────────────────────────────
// Helpery do wypisywania tekstu
// ─────────────────────────────────────────────────

/// Zapisz string do stdout (fd 1).
pub fn print(s: &str) {
    let _ = write(1, s.as_bytes());
}

/// Zapisz string + newline do stdout.
pub fn println(s: &str) {
    print(s);
    let _ = write(1, b"\n");
}

/// Zapisz string do stderr (fd 2).
pub fn eprint(s: &str) {
    let _ = write(2, s.as_bytes());
}

/// Zapisz string + newline do stderr.
pub fn eprintln(s: &str) {
    eprint(s);
    let _ = write(2, b"\n");
}

// ─────────────────────────────────────────────────
// Punkt wejścia _start
// ─────────────────────────────────────────────────

/// Punkt wejścia procesu userland.
/// Parsuje argc/argv ze stosu (umieszczone przez kernel) i wywołuje main().
///
/// Układ stosu na wejściu:
///   RSP → argc (u64)
///   RSP+8 → argv[0] (pointer to string)
///   RSP+16 → argv[1]
///   ...
///   RSP+8*(argc) → argv[argc-1]
///   RSP+8*(argc+1) → NULL
///
/// Program użytkownika musi zdefiniować:
/// ```
/// #[no_mangle]
/// pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 { ... }
/// ```
#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    // Read argc and argv pointer from the stack.
    // The kernel pushed: argc, argv[0], argv[1], ..., NULL
    // RSP points to argc on entry.
    let argc: u64;
    let argv: *const *const u8;
    core::arch::asm!(
        "mov {}, [rsp]",      // argc
        "lea {}, [rsp + 8]",  // argv = &argv[0]
        out(reg) argc,
        out(reg) argv,
        options(nostack, nomem),
    );

    extern "C" {
        fn main(argc: i32, argv: *const *const u8) -> i32;
    }
    let code = main(argc as i32, argv);
    exit(code);
}

// ─────────────────────────────────────────────────
// Panic handler dla userland
// ─────────────────────────────────────────────────

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    eprintln("PANIC in userland process");
    exit(101);
}

// ─────────────────────────────────────────────────
// Bump allocator for userland (feature: "alloc")
// ─────────────────────────────────────────────────

/// Simple bump allocator for no_std userland programs that need `alloc`.
///
/// Uses a static 256 KiB heap. Programs that need alloc should enable
/// the "alloc" feature and the allocator is automatically registered.
#[cfg(feature = "alloc")]
mod bump_alloc {
    use core::alloc::{GlobalAlloc, Layout};
    use core::sync::atomic::{AtomicUsize, Ordering};

    const HEAP_SIZE: usize = 256 * 1024; // 256 KiB

    #[repr(C, align(16))]
    struct Heap {
        data: [u8; HEAP_SIZE],
    }

    static mut HEAP: Heap = Heap {
        data: [0; HEAP_SIZE],
    };

    static HEAP_POS: AtomicUsize = AtomicUsize::new(0);

    pub struct BumpAllocator;

    unsafe impl GlobalAlloc for BumpAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let size = layout.size();
            let align = layout.align();

            loop {
                let pos = HEAP_POS.load(Ordering::Relaxed);
                let aligned = (pos + align - 1) & !(align - 1);
                let new_pos = aligned + size;

                if new_pos > HEAP_SIZE {
                    return core::ptr::null_mut();
                }

                if HEAP_POS
                    .compare_exchange_weak(pos, new_pos, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    let heap_ptr = core::ptr::addr_of_mut!(HEAP.data) as *mut u8;
                    return unsafe { heap_ptr.add(aligned) };
                }
            }
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
            // Bump allocator doesn't free — acceptable for short-lived shell commands
        }
    }

    #[cfg(not(test))]
    #[global_allocator]
    static ALLOCATOR: BumpAllocator = BumpAllocator;
}
