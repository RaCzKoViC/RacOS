//! Access to user memory from syscall handlers.
//!
//! A pointer a process hands the kernel is only as good as the process's
//! own right to use it. Until 2026-09 `validate_user_ptr` checked null,
//! `ptr <= USER_SPACE_MAX` and overflow and nothing else; the kernel is
//! linked at 0x100000 and identity-mapped as supervisor pages, so a kernel
//! address passed every check and `write(1, 0x100000, 64)` printed kernel
//! bytes, `read` into a kernel address wrote kernel memory in ring 0, and
//! an unmapped address page-faulted in the kernel.
//!
//! Every function here asks the process's page table the question the CPU
//! would ask for a ring-3 access - present, USER at every level, WRITABLE
//! at every level for a write - for every page of the range, and answers
//! EFAULT instead of touching what the process could not. The copying
//! functions check and copy with interrupts off, so a `munmap` from
//! another thread of the process cannot land between the check and the
//! access.
//!
//! What this does not do yet: recover from a page fault taken in ring 0
//! (an exception table). Handlers that still dereference a checked range
//! directly say so in their SAFETY comment; the window is the same thread,
//! the same page table, between the check and the access.

use super::error::SyscallError;
use crate::mm::virt::{flags, read_cr3, user_access_flags};

/// Highest address a user pointer may name (canonical lower half).
pub const USER_SPACE_MAX: u64 = 0x0000_7FFF_FFFF_FFFF;

const PAGE_SIZE: u64 = 4096;

/// The direction of the access the kernel is about to make.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    /// The kernel reads the range (a write(2) buffer, a path, argv).
    Read,
    /// The kernel writes the range (a read(2) buffer, a stat buffer). A
    /// page the process can write it can also read, so Write implies Read.
    Write,
}

/// Run `f` with interrupts disabled, restoring IF afterwards.
///
/// Nesting-safe: handlers already open cli/sti windows of their own, so
/// this must not enable interrupts a caller had disabled.
fn with_irqs_off<T>(f: impl FnOnce() -> T) -> T {
    let rflags: u64;
    // SAFETY: reading RFLAGS and clearing IF; IF is restored below from the
    // saved copy. No memory is touched by these instructions.
    unsafe {
        core::arch::asm!("pushfq", "pop {}", out(reg) rflags, options(nomem));
        core::arch::asm!("cli", options(nomem, nostack));
    }
    let result = f();
    if rflags & (1 << 9) != 0 {
        // SAFETY: IF was set on entry; restoring it re-enables what we took.
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack));
        }
    }
    result
}

/// Is every page of `[ptr, ptr + len)` one the current process may access
/// the way `access` says? EFAULT otherwise. A zero-length range is fine
/// as long as the pointer itself is sane.
pub fn check_user_range(ptr: u64, len: usize, access: Access) -> Result<(), SyscallError> {
    if ptr == 0 || ptr > USER_SPACE_MAX {
        return Err(SyscallError::EFAULT);
    }
    let end = ptr.checked_add(len as u64).ok_or(SyscallError::EFAULT)?;
    if end > USER_SPACE_MAX {
        return Err(SyscallError::EFAULT);
    }
    if len == 0 {
        return Ok(());
    }
    let cr3 = read_cr3();
    let mut page = ptr & !(PAGE_SIZE - 1);
    let last = (end - 1) & !(PAGE_SIZE - 1);
    loop {
        // SAFETY: CR3 names the current process's page table; this kernel
        // identity-maps every page-table frame it allocates.
        let f = unsafe { user_access_flags(cr3, page) }.ok_or(SyscallError::EFAULT)?;
        if f & flags::USER == 0 {
            return Err(SyscallError::EFAULT);
        }
        if access == Access::Write && f & flags::WRITABLE == 0 {
            return Err(SyscallError::EFAULT);
        }
        if page == last {
            break;
        }
        page += PAGE_SIZE;
    }
    Ok(())
}

/// Copy `dst.len()` bytes from user address `src` into `dst`.
pub fn copy_from_user(dst: &mut [u8], src: u64) -> Result<(), SyscallError> {
    with_irqs_off(|| {
        check_user_range(src, dst.len(), Access::Read)?;
        // SAFETY: every page of [src, src+len) is present and user-readable
        // in the current page table (checked just above, interrupts off, so
        // no other thread can unmap it before the copy); dst is a kernel
        // slice of exactly len bytes and cannot overlap user memory.
        unsafe {
            core::ptr::copy_nonoverlapping(src as *const u8, dst.as_mut_ptr(), dst.len());
        }
        Ok(())
    })
}

/// Copy `src` to user address `dst`.
pub fn copy_to_user(dst: u64, src: &[u8]) -> Result<(), SyscallError> {
    with_irqs_off(|| {
        check_user_range(dst, src.len(), Access::Write)?;
        // SAFETY: every page of [dst, dst+len) is present, user-accessible
        // and writable in the current page table (checked above with
        // interrupts off); src is a kernel slice of exactly len bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len());
        }
        Ok(())
    })
}

/// Read one plain value from user memory. `T` must be a plain-old-data
/// type for which any bit pattern is valid (integers, `#[repr(C)]` structs
/// of integers); the read is unaligned, so the user need not align it.
pub fn get_user<T: Copy>(ptr: u64) -> Result<T, SyscallError> {
    with_irqs_off(|| {
        check_user_range(ptr, core::mem::size_of::<T>(), Access::Read)?;
        // SAFETY: the range is present and user-readable (checked above,
        // interrupts off); read_unaligned makes no alignment assumption.
        Ok(unsafe { core::ptr::read_unaligned(ptr as *const T) })
    })
}

/// Write one plain value to user memory (see `get_user` for `T`).
pub fn put_user<T: Copy>(ptr: u64, value: T) -> Result<(), SyscallError> {
    with_irqs_off(|| {
        check_user_range(ptr, core::mem::size_of::<T>(), Access::Write)?;
        // SAFETY: the range is present, user-accessible and writable
        // (checked above, interrupts off); write_unaligned makes no
        // alignment assumption.
        unsafe {
            core::ptr::write_unaligned(ptr as *mut T, value);
        }
        Ok(())
    })
}

/// Length of the NUL-terminated string at `ptr`, not counting the NUL, at
/// most `max` bytes (ENAMETOOLONG past that). Each page is checked before
/// a byte of it is looked at, so a string that runs off its mapping is
/// EFAULT, not a kernel page fault.
pub fn strlen_user(ptr: u64, max: usize) -> Result<usize, SyscallError> {
    if ptr == 0 || ptr > USER_SPACE_MAX {
        return Err(SyscallError::EFAULT);
    }
    with_irqs_off(|| strlen_user_locked(ptr, max))
}

fn strlen_user_locked(ptr: u64, max: usize) -> Result<usize, SyscallError> {
    let cr3 = read_cr3();
    let mut len = 0usize;
    let mut addr = ptr;
    loop {
        if addr > USER_SPACE_MAX {
            return Err(SyscallError::EFAULT);
        }
        let page = addr & !(PAGE_SIZE - 1);
        // SAFETY: as in check_user_range.
        let f = unsafe { user_access_flags(cr3, page) }.ok_or(SyscallError::EFAULT)?;
        if f & flags::USER == 0 {
            return Err(SyscallError::EFAULT);
        }
        let page_end = page + PAGE_SIZE;
        while addr < page_end {
            if len >= max {
                return Err(SyscallError::ENAMETOOLONG);
            }
            // SAFETY: addr lies in a page just found present and
            // user-readable; interrupts are off for the whole scan.
            let byte = unsafe { *(addr as *const u8) };
            if byte == 0 {
                return Ok(len);
            }
            len += 1;
            addr += 1;
        }
    }
}

/// Copy the NUL-terminated byte string at `ptr` (at most `max` bytes, NUL
/// not included) into kernel memory.
pub fn read_user_bytes(ptr: u64, max: usize) -> Result<alloc::vec::Vec<u8>, SyscallError> {
    if ptr == 0 || ptr > USER_SPACE_MAX {
        return Err(SyscallError::EFAULT);
    }
    with_irqs_off(|| {
        let len = strlen_user_locked(ptr, max)?;
        let mut out = alloc::vec![0u8; len];
        // SAFETY: strlen_user_locked just walked [ptr, ptr+len) page by page
        // and found every page present and user-readable; interrupts are
        // still off, so the mapping cannot have changed.
        unsafe {
            core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), len);
        }
        Ok(out)
    })
}

/// Copy the NUL-terminated string at `ptr` (at most `max` bytes) into
/// kernel memory. EINVAL if it is not UTF-8.
pub fn read_user_string(ptr: u64, max: usize) -> Result<alloc::string::String, SyscallError> {
    let bytes = read_user_bytes(ptr, max)?;
    alloc::string::String::from_utf8(bytes).map_err(|_| SyscallError::EINVAL)
}
