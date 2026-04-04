# RacOS — Syscall Specification

> Version: 0.1.0 | Status: Draft | Component: RaCore

This document provides the detailed specification for each syscall in the RaCore ABI v1. For the ABI overview and calling convention, see [KERNEL_ABI.md](KERNEL_ABI.md).

## Syscall 0: sys_exit

**Purpose**: Terminate the calling process.

| Field | Value |
|-------|-------|
| Number | 0 |
| Stability | Stable |
| Args | RDI = exit_status (i32) |
| Return | Does not return |

**Behavior**:
1. Set process state to Zombie
2. Store exit_status for parent's `sys_wait`
3. Reparent children to PID 1 (RacInit)
4. Release address space and file descriptors
5. Wake parent if waiting
6. Schedule next task

**Errors**: None (always succeeds)

---

## Syscall 1: sys_read

**Purpose**: Read data from a file descriptor.

| Field | Value |
|-------|-------|
| Number | 1 |
| Stability | Stable |
| Args | RDI = fd, RSI = buf (*mut u8), RDX = count (usize) |
| Return | RAX = bytes_read (≥ 0) or error |

**Behavior**:
1. Validate fd is open and has read permission
2. Validate buf is in user space and writable
3. Read up to `count` bytes from the file/device
4. Update file offset
5. Return number of bytes read (0 = EOF)

**Errors**: EBADF (bad fd), EFAULT (bad buf), EINVAL (negative count), EIO (I/O error), EAGAIN (non-blocking, no data)

---

## Syscall 2: sys_write

**Purpose**: Write data to a file descriptor.

| Field | Value |
|-------|-------|
| Number | 2 |
| Stability | Stable |
| Args | RDI = fd, RSI = buf (*const u8), RDX = count (usize) |
| Return | RAX = bytes_written (≥ 0) or error |

**Behavior**:
1. Validate fd is open and has write permission
2. Validate buf is in user space and readable
3. Write up to `count` bytes to the file/device
4. Update file offset (or append if O_APPEND)
5. Return number of bytes written

**Errors**: EBADF, EFAULT, EINVAL, EIO, ENOSPC

---

## Syscall 3: sys_open

**Purpose**: Open a file or device.

| Field | Value |
|-------|-------|
| Number | 3 |
| Stability | Stable |
| Args | RDI = path (*const u8, null-terminated), RSI = flags (u32), RDX = mode (u32) |
| Return | RAX = fd (≥ 0) or error |

**Behavior**:
1. Validate path pointer and copy path from user space (max 4096 bytes)
2. Resolve path through VFS
3. Permission check against process uid/gid
4. If O_CREAT and file doesn't exist: create with given mode
5. If O_TRUNC: truncate to zero
6. Allocate file descriptor in process FD table
7. Return fd number

**Errors**: ENOENT, EACCES, ENAMETOOLONG, ENFILE, EMFILE, EEXIST (O_CREAT|O_EXCL), ENOTDIR, EFAULT

---

## Syscall 4: sys_close

**Purpose**: Close a file descriptor.

| Field | Value |
|-------|-------|
| Number | 4 |
| Stability | Stable |
| Args | RDI = fd (i32) |
| Return | RAX = 0 or error |

**Behavior**:
1. Validate fd is open
2. Flush pending writes if applicable
3. Release fd from process FD table
4. Decrement file refcount; free inode if last reference

**Errors**: EBADF

---

## Syscall 5: sys_stat

**Purpose**: Get file status.

| Field | Value |
|-------|-------|
| Number | 5 |
| Stability | Stable |
| Args | RDI = path (*const u8), RSI = statbuf (*mut StatBuf) |
| Return | RAX = 0 or error |

**Behavior**:
1. Copy path from user space
2. Resolve path through VFS
3. Fill StatBuf (see KERNEL_ABI.md §6.1)
4. Copy StatBuf to user space

**Errors**: ENOENT, EACCES, EFAULT, ENAMETOOLONG

---

## Syscall 6: sys_mmap

**Purpose**: Map memory into the process address space.

| Field | Value |
|-------|-------|
| Number | 6 |
| Stability | Stable |
| Args | RDI = addr, RSI = length, RDX = prot, R10 = flags, R8 = fd, R9 = offset |
| Return | RAX = mapped address or error |

**Behavior**:
1. If MAP_ANONYMOUS: allocate physical frames, ignore fd/offset
2. If MAP_FIXED: use specified address (fail if already mapped)
3. Otherwise: kernel chooses address
4. Set page table permissions per prot
5. Return base address of mapping

**Errors**: ENOMEM, EINVAL, EBADF (if not anonymous), EACCES

---

## Syscall 7: sys_munmap

**Purpose**: Unmap memory from the process address space.

| Field | Value |
|-------|-------|
| Number | 7 |
| Stability | Stable |
| Args | RDI = addr (u64, page-aligned), RSI = length (usize) |
| Return | RAX = 0 or error |

**Behavior**:
1. Validate addr is page-aligned
2. Unmap pages in range [addr, addr+length)
3. Free physical frames if not shared

**Errors**: EINVAL (not page-aligned, zero length)

---

## Syscall 8: sys_pipe

**Purpose**: Create a unidirectional pipe.

| Field | Value |
|-------|-------|
| Number | 8 |
| Stability | Stable |
| Args | RDI = fds (*mut [i32; 2]) |
| Return | RAX = 0 or error |

**Behavior**:
1. Allocate pipe buffer (default 64 KiB)
2. Create two file descriptors: fds[0] = read end, fds[1] = write end
3. Write to fds to user space

**Errors**: EFAULT, ENFILE, EMFILE

---

## Syscall 9: sys_dup

**Purpose**: Duplicate a file descriptor.

| Field | Value |
|-------|-------|
| Number | 9 |
| Stability | Stable |
| Args | RDI = oldfd (i32) |
| Return | RAX = newfd (lowest available) or error |

**Errors**: EBADF, EMFILE

---

## Syscall 10: sys_dup2

**Purpose**: Duplicate a file descriptor to a specific number.

| Field | Value |
|-------|-------|
| Number | 10 |
| Stability | Stable |
| Args | RDI = oldfd (i32), RSI = newfd (i32) |
| Return | RAX = newfd or error |

**Behavior**: If newfd is already open, close it first. Then duplicate oldfd to newfd.

**Errors**: EBADF, EINVAL

---

## Syscall 11: sys_exec

**Purpose**: Replace current process image with a new program.

| Field | Value |
|-------|-------|
| Number | 11 |
| Stability | Stable |
| Args | RDI = path, RSI = argv, RDX = envp |
| Return | Does not return on success; error on failure |

**Behavior**:
1. Copy path, argv, envp from user space
2. Open and validate ELF64 binary
3. Replace address space
4. Set up new stack with argv/envp
5. Jump to entry point

**Errors**: ENOENT, EACCES, ENOEXEC (not valid ELF), ENOMEM

---

## Syscall 12: sys_spawn

**Purpose**: Create a new child process running a specified program.

| Field | Value |
|-------|-------|
| Number | 12 |
| Stability | Stable |
| Args | RDI = path, RSI = argv, RDX = envp |
| Return | RAX = child_pid or error |

**Behavior**: Fork equivalent + exec in one syscall. Creates a new process with its own address space, loaded from the specified ELF binary.

**Errors**: ENOENT, EACCES, ENOEXEC, ENOMEM

---

## Syscall 13: sys_wait

**Purpose**: Wait for a child process to change state.

| Field | Value |
|-------|-------|
| Number | 13 |
| Stability | Stable |
| Args | RDI = pid (i32; -1 = any child), RSI = status (*mut i32), RDX = options (u32) |
| Return | RAX = pid of changed child or error |

**Options**: WNOHANG = 0x1 (return immediately if no child exited)

**Errors**: ESRCH (no such child), EINTR, EFAULT

---

## Syscall 14: sys_getpid

**Purpose**: Get current process ID.

| Field | Value |
|-------|-------|
| Number | 14 |
| Stability | Stable |
| Args | None |
| Return | RAX = current pid |

**Errors**: None

---

## Syscall 15: sys_chdir

**Purpose**: Change working directory.

| Field | Value |
|-------|-------|
| Number | 15 |
| Stability | Stable |
| Args | RDI = path (*const u8) |
| Return | RAX = 0 or error |

**Errors**: ENOENT, ENOTDIR, EACCES, EFAULT

---

## Syscall 16: sys_ioctl

**Purpose**: Device-specific control.

| Field | Value |
|-------|-------|
| Number | 16 |
| Stability | **Unstable** |
| Args | RDI = fd, RSI = request (u64), RDX = arg (u64) |
| Return | RAX = 0 or request-specific value, or error |

**Note**: ioctl requests are device-specific and may change. Each driver must document its ioctl interface.

**Errors**: EBADF, EINVAL, ENOTTY

---

## Syscall 17: sys_kill

**Purpose**: Send a signal to a process.

| Field | Value |
|-------|-------|
| Number | 17 |
| Stability | Stable |
| Args | RDI = pid (i32), RSI = signal (i32) |
| Return | RAX = 0 or error |

**Signals (v1)**: SIGTERM=15, SIGKILL=9, SIGINT=2, SIGSTOP=19, SIGCONT=18

**Errors**: ESRCH, EPERM, EINVAL (invalid signal)

---

## Syscall 18: sys_getcwd

**Purpose**: Get current working directory.

| Field | Value |
|-------|-------|
| Number | 18 |
| Stability | Stable |
| Args | RDI = buf (*mut u8), RSI = size (usize) |
| Return | RAX = 0 or error |

**Errors**: EFAULT, ENAMETOOLONG (buffer too small)
