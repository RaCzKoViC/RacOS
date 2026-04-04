# RacOS — Kernel ABI Specification

> Version: 0.1.0 | Status: Draft | Component: RaCore

## 1. Overview

The RaCore kernel ABI defines the binary interface between user space processes and the kernel. All syscall numbers, calling conventions, error codes, and stability guarantees are specified here.

## 2. ABI Versioning Policy

- **ABI version**: Major.Minor (e.g., 1.0)
- **Stable syscalls**: guaranteed across minor versions; removal requires major version bump + 2-version deprecation period
- **Unstable syscalls**: may change across minor versions; marked explicitly
- **Experimental syscalls**: may change or be removed at any time
- Every ABI change requires an ADR

## 3. Calling Convention (x86_64)

| Register | Purpose |
|----------|---------|
| RAX | Syscall number (input) / return value (output) |
| RDI | Argument 1 |
| RSI | Argument 2 |
| RDX | Argument 3 |
| R10 | Argument 4 |
| R8 | Argument 5 |
| R9 | Argument 6 |
| RCX | Clobbered by `syscall` instruction (stores RIP) |
| R11 | Clobbered by `syscall` instruction (stores RFLAGS) |

**Instruction**: `syscall`

**Return**: RAX contains result on success (≥ 0) or negated error code on failure (< 0).

## 4. Error Codes

| Code | Name | Description |
|------|------|-------------|
| -1 | EPERM | Operation not permitted |
| -2 | ENOENT | No such file or directory |
| -3 | ESRCH | No such process |
| -4 | EINTR | Interrupted system call |
| -5 | EIO | I/O error |
| -6 | ENXIO | No such device or address |
| -9 | EBADF | Bad file descriptor |
| -11 | EAGAIN | Try again |
| -12 | ENOMEM | Out of memory |
| -13 | EACCES | Permission denied |
| -14 | EFAULT | Bad address |
| -17 | EEXIST | File exists |
| -20 | ENOTDIR | Not a directory |
| -21 | EISDIR | Is a directory |
| -22 | EINVAL | Invalid argument |
| -23 | ENFILE | File table overflow |
| -24 | EMFILE | Too many open files |
| -28 | ENOSPC | No space left on device |
| -36 | ENAMETOOLONG | Filename too long |
| -38 | ENOSYS | Function not implemented |

## 5. Syscall Table v1

### 5.1 Process Management

| Nr | Name | Args | Return | Stability |
|----|------|------|--------|-----------|
| 0 | sys_exit | status: i32 | — (noreturn) | Stable |
| 11 | sys_exec | path: *const u8, argv: *const *const u8, envp: *const *const u8 | 0 or error | Stable |
| 12 | sys_spawn | path: *const u8, argv: *const *const u8, envp: *const *const u8 | child_pid or error | Stable |
| 13 | sys_wait | pid: i32, status: *mut i32, options: u32 | pid or error | Stable |
| 14 | sys_getpid | — | pid | Stable |
| 17 | sys_kill | pid: i32, signal: i32 | 0 or error | Stable |

### 5.2 File Operations

| Nr | Name | Args | Return | Stability |
|----|------|------|--------|-----------|
| 1 | sys_read | fd: i32, buf: *mut u8, count: usize | bytes_read or error | Stable |
| 2 | sys_write | fd: i32, buf: *const u8, count: usize | bytes_written or error | Stable |
| 3 | sys_open | path: *const u8, flags: u32, mode: u32 | fd or error | Stable |
| 4 | sys_close | fd: i32 | 0 or error | Stable |
| 5 | sys_stat | path: *const u8, statbuf: *mut StatBuf | 0 or error | Stable |
| 9 | sys_dup | oldfd: i32 | newfd or error | Stable |
| 10 | sys_dup2 | oldfd: i32, newfd: i32 | newfd or error | Stable |
| 15 | sys_chdir | path: *const u8 | 0 or error | Stable |
| 18 | sys_getcwd | buf: *mut u8, size: usize | 0 or error | Stable |
| 16 | sys_ioctl | fd: i32, request: u64, arg: u64 | 0 or error | Unstable |

### 5.3 Memory Management

| Nr | Name | Args | Return | Stability |
|----|------|------|--------|-----------|
| 6 | sys_mmap | addr: u64, length: usize, prot: u32, flags: u32, fd: i32, offset: u64 | address or error | Stable |
| 7 | sys_munmap | addr: u64, length: usize | 0 or error | Stable |

### 5.4 IPC

| Nr | Name | Args | Return | Stability |
|----|------|------|--------|-----------|
| 8 | sys_pipe | fds: *mut [i32; 2] | 0 or error | Stable |

## 6. Data Structures

### 6.1 StatBuf

```rust
#[repr(C)]
pub struct StatBuf {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_mode: u32,
    pub st_nlink: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_size: u64,
    pub st_blksize: u32,
    pub st_blocks: u64,
    pub st_atime: u64,
    pub st_mtime: u64,
    pub st_ctime: u64,
}
```

### 6.2 Open Flags

| Flag | Value | Description |
|------|-------|-------------|
| O_RDONLY | 0x0000 | Read only |
| O_WRONLY | 0x0001 | Write only |
| O_RDWR | 0x0002 | Read and write |
| O_CREAT | 0x0040 | Create if not exists |
| O_TRUNC | 0x0200 | Truncate to zero |
| O_APPEND | 0x0400 | Append writes |

### 6.3 Mmap Prot/Flags

| Prot | Value |
|------|-------|
| PROT_NONE | 0x0 |
| PROT_READ | 0x1 |
| PROT_WRITE | 0x2 |
| PROT_EXEC | 0x4 |

| Flag | Value |
|------|-------|
| MAP_PRIVATE | 0x02 |
| MAP_ANONYMOUS | 0x20 |
| MAP_FIXED | 0x10 |

## 7. Pointer Validation

All user-space pointers passed to syscalls are validated:
1. Must be within user address space (below 0x0000_7FFF_FFFF_FFFF)
2. Must be properly aligned for the data type
3. Must be mapped in the process page tables
4. Kernel never trusts user pointers — always copies data via safe accessors

## 8. Future Syscalls (planned, not yet assigned)

- `sys_socket`, `sys_bind`, `sys_listen`, `sys_accept`, `sys_connect` (networking)
- `sys_select` / `sys_poll` (I/O multiplexing)
- `sys_clone` (thread creation)
- `sys_futex` (fast userspace mutex)
- `sys_getdents` (directory listing)
