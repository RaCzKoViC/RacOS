// RaCore — Syscall error codes (KERNEL_ABI.md §4)
//
// Error codes are returned as negative values in RAX.
// These match the POSIX error numbers used in KERNEL_ABI.md.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i64)]
pub enum SyscallError {
    EPERM        = -1,
    ENOENT       = -2,
    ESRCH        = -3,
    EINTR        = -4,
    EIO          = -5,
    ENXIO        = -6,
    EBADF        = -9,
    EAGAIN       = -11,
    ENOMEM       = -12,
    EACCES       = -13,
    EFAULT       = -14,
    EEXIST       = -17,
    ENOTDIR      = -20,
    EISDIR       = -21,
    EINVAL       = -22,
    ENFILE       = -23,
    EMFILE       = -24,
    ENOSPC       = -28,
    ERANGE       = -34,
    ENAMETOOLONG = -36,
    ENOSYS       = -38,
    ENOEXEC      = -39,
    ECHILD       = -10,
    EPIPE        = -32,
    ENOTTY       = -25,
    ECONNREFUSED = -111,
    EADDRINUSE   = -98,
    ENOTCONN     = -107,
    ETIMEDOUT    = -110,
}

impl SyscallError {
    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

/// Syscall result type.
pub type SyscallResult = Result<i64, SyscallError>;

/// Convert a SyscallResult to the raw RAX return value.
/// Success: non-negative value. Error: negative error code.
pub fn result_to_raw(result: SyscallResult) -> i64 {
    match result {
        Ok(val) => val,
        Err(e) => e as i64,
    }
}
