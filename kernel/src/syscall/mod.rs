// RaCore — Syscall subsystem
//
// Implements the SYSCALL/SYSRET mechanism for x86_64.
// See KERNEL_ABI.md and SYSCALL_SPEC.md for the full specification.
//
// Calling convention:
//   RAX = syscall number
//   RDI, RSI, RDX, R10, R8, R9 = arguments
//   Return in RAX (>= 0 success, < 0 negated error code)
//   RCX and R11 are clobbered by the SYSCALL instruction itself.

pub mod dispatch;
pub mod entry;
pub mod error;
pub mod handlers;
