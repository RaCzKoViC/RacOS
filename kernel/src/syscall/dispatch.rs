// RaCore — Syscall dispatch table
//
// Routes syscall numbers to handler functions per KERNEL_ABI.md §5.

use super::error::{SyscallError, SyscallResult, result_to_raw};
use super::handlers;

/// Syscall numbers (KERNEL_ABI.md §5).
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
pub const SYS_GETHOSTBYNAME: u64 = 78;
pub const SYS_PTHREAD_CREATE: u64 = 0x400;

/// Main syscall dispatcher called from the assembly entry stub.
///
/// # Arguments
/// * `nr` — syscall number (from RAX)
/// * `arg1`-`arg6` — syscall arguments (from RDI, RSI, RDX, R10, R8, R9)
///
/// # Returns
/// Raw i64 to be placed in RAX.
#[no_mangle]
pub extern "C" fn syscall_dispatch(
    nr: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) -> i64 {
    let result: SyscallResult = match nr {
        SYS_EXIT => {
            handlers::sys_exit(arg1 as i32);
            // sys_exit does not return, but the compiler doesn't know that
            // from the match arm's perspective
            Ok(0)
        }
        SYS_READ => handlers::sys_read(arg1 as i32, arg2 as *mut u8, arg3 as usize),
        SYS_WRITE => handlers::sys_write(arg1 as i32, arg2 as *const u8, arg3 as usize),
        SYS_OPEN => handlers::sys_open(arg1 as *const u8, arg2 as u32, arg3 as u32),
        SYS_CLOSE => handlers::sys_close(arg1 as i32),
        SYS_STAT => handlers::sys_stat(arg1 as *const u8, arg2 as *mut u8),
        SYS_GETPID => handlers::sys_getpid(),
        SYS_MMAP => handlers::sys_mmap(arg1, arg2 as usize, arg3 as u32, arg4 as u32, arg5 as i32, arg6),
        SYS_MUNMAP => handlers::sys_munmap(arg1, arg2 as usize),
        SYS_PIPE => handlers::sys_pipe(arg1 as *mut i32),
        SYS_DUP => handlers::sys_dup(arg1 as i32),
        SYS_DUP2 => handlers::sys_dup2(arg1 as i32, arg2 as i32),
        SYS_EXEC => handlers::sys_exec(arg1 as *const u8, arg2, arg3),
        SYS_SPAWN => handlers::sys_spawn(arg1 as *const u8, arg2, arg3),
        SYS_WAIT => handlers::sys_wait(arg1 as i32, arg2, arg3 as u32),
        SYS_CHDIR => handlers::sys_chdir(arg1 as *const u8),
        SYS_IOCTL => handlers::sys_ioctl(arg1 as i32, arg2 as u32, arg3),
        SYS_KILL => handlers::sys_kill(arg1 as i32, arg2 as i32),
        SYS_GETCWD => handlers::sys_getcwd(arg1 as *mut u8, arg2 as usize),
        SYS_SETPGID => handlers::sys_setpgid(arg1 as u32, arg2 as u32),
        SYS_GETPGID => handlers::sys_getpgid(arg1 as u32),
        SYS_SETSID => handlers::sys_setsid(),
        SYS_CLOCK_GETTIME => handlers::sys_clock_gettime(arg1 as u32, arg2 as *mut u8),
        SYS_GETDENTS => handlers::sys_getdents(arg1 as i32, arg2 as *mut u8, arg3 as usize),
        SYS_MKDIR => handlers::sys_mkdir(arg1 as *const u8, arg2 as u32),
        SYS_UNLINK => handlers::sys_unlink(arg1 as *const u8),
        SYS_FORK => handlers::sys_fork(),
        SYS_SIGACTION => handlers::sys_sigaction(arg1 as i32, arg2 as *const u8, arg3 as *mut u8),
        SYS_SIGRETURN => handlers::sys_sigreturn(),
        SYS_POLL => handlers::sys_poll(arg1 as *mut u8, arg2 as u32, arg3 as i32),
        SYS_GETPPID => handlers::sys_getppid(),
        SYS_GETUID => handlers::sys_getuid(),
        SYS_GETGID => handlers::sys_getgid(),
        SYS_SETUID => handlers::sys_setuid(arg1 as u32),
        SYS_SETGID => handlers::sys_setgid(arg1 as u32),
        SYS_GETEUID => handlers::sys_geteuid(),
        SYS_GETEGID => handlers::sys_getegid(),
        SYS_NANOSLEEP => handlers::sys_nanosleep(arg1 as *const u8, arg2 as *mut u8),
        SYS_TRUNCATE => handlers::sys_truncate(arg1 as *const u8, arg2),
        SYS_FSTAT => handlers::sys_fstat(arg1 as i32, arg2 as *mut u8),
        SYS_LSEEK => handlers::sys_lseek(arg1 as i32, arg2 as i64, arg3 as i32),
        SYS_ACCESS => handlers::sys_access(arg1 as *const u8, arg2 as u32),
        SYS_CHMOD => handlers::sys_chmod(arg1 as *const u8, arg2 as u32),
        SYS_CHOWN => handlers::sys_chown(arg1 as *const u8, arg2 as u32, arg3 as u32),
        SYS_UMASK => handlers::sys_umask(arg1 as u32),
        SYS_LINK => handlers::sys_link(arg1 as *const u8, arg2 as *const u8),
        SYS_SYMLINK => handlers::sys_symlink(arg1 as *const u8, arg2 as *const u8),
        SYS_READLINK => handlers::sys_readlink(arg1 as *const u8, arg2 as *mut u8, arg3 as usize),
        SYS_RENAME => handlers::sys_rename(arg1 as *const u8, arg2 as *const u8),
        SYS_FCNTL => handlers::sys_fcntl(arg1 as i32, arg2 as i32, arg3),
        SYS_ISATTY => handlers::sys_isatty(arg1 as i32),
        SYS_SOCKET => handlers::sys_socket(arg1 as i32, arg2 as i32, arg3 as i32),
        SYS_BIND => handlers::sys_bind(arg1 as i32, arg2 as *const u8, arg3 as u32),
        SYS_LISTEN => handlers::sys_listen(arg1 as i32, arg2 as i32),
        SYS_ACCEPT => handlers::sys_accept(arg1 as i32, arg2 as *mut u8, arg3 as *mut u32),
        SYS_CONNECT => handlers::sys_connect(arg1 as i32, arg2 as *const u8, arg3 as u32),
        SYS_SEND => handlers::sys_send(arg1 as i32, arg2 as *const u8, arg3 as usize, arg4 as u32),
        SYS_RECV => handlers::sys_recv(arg1 as i32, arg2 as *mut u8, arg3 as usize, arg4 as u32),
        SYS_SHUTDOWN => handlers::sys_shutdown(arg1 as i32, arg2 as i32),
        SYS_GETSOCKNAME => handlers::sys_getsockname(arg1 as i32, arg2 as *mut u8, arg3 as *mut u32),
        SYS_GETPEERNAME => handlers::sys_getpeername(arg1 as i32, arg2 as *mut u8, arg3 as *mut u32),
        SYS_SETSOCKOPT => handlers::sys_setsockopt(arg1 as i32, arg2 as i32, arg3 as i32, arg4 as *const u8, arg5 as u32),
        SYS_GETSOCKOPT => handlers::sys_getsockopt(arg1 as i32, arg2 as i32, arg3 as i32, arg4 as *mut u8, arg5 as *mut u32),
        SYS_WAITPID => handlers::sys_waitpid(arg1 as i32, arg2 as *mut i32, arg3 as u32),
        SYS_PIPE2 => handlers::sys_pipe2(arg1 as *mut i32, arg2 as u32),
        SYS_UNAME => handlers::sys_uname(arg1 as *mut u8),
        SYS_MOUNT => handlers::sys_mount(arg1 as *const u8, arg2 as *const u8, arg3 as *const u8, arg4 as u64, arg5 as *const u8),
        SYS_UMOUNT => handlers::sys_umount(arg1 as *const u8),
        SYS_MPROTECT => handlers::sys_mprotect(arg1, arg2 as usize, arg3 as u32),
        SYS_FSYNC => handlers::sys_fsync(arg1 as i32),
        SYS_FTRUNCATE => handlers::sys_ftruncate(arg1 as i32, arg2),
        SYS_WRITEV => handlers::sys_writev(arg1 as i32, arg2 as *const u8, arg3 as i32),
        SYS_READV => handlers::sys_readv(arg1 as i32, arg2 as *const u8, arg3 as i32),
        SYS_SCHED_YIELD => handlers::sys_sched_yield(),
        SYS_REBOOT => handlers::sys_reboot(arg1 as u32),
        SYS_HOSTNAME => handlers::sys_hostname(arg1 as *mut u8, arg2 as usize, arg3 as u32),
        SYS_GETRANDOM => handlers::sys_getrandom(arg1 as *mut u8, arg2 as usize, arg3 as u32),
        SYS_CLONE => handlers::sys_clone(arg1 as u32, arg2 as *mut u8, arg3 as i32, arg4 as i32, arg5 as *mut u8),
        SYS_GETHOSTBYNAME => handlers::sys_gethostbyname(arg1 as *const u8, arg2 as usize, arg3 as *mut u8),
        SYS_PTHREAD_CREATE => handlers::sys_pthread_create(arg1, arg2),
        _ => Err(SyscallError::ENOSYS),
    };

    // Deliver any pending signals before returning to user space.
    handlers::deliver_pending_signals();

    result_to_raw(result)
}
