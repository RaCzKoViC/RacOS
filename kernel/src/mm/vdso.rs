// VDSO: one user-mode page containing kernel-provided trampolines.
//
// Currently hosts a single trampoline used as the return address for user
// signal handlers:
//
//     mov rax, 28       ; SYS_sigreturn
//     syscall
//
// When a signal handler `ret`s, RIP lands on the first byte of this page
// and the syscall executes, which restores the saved user context via
// sys_sigreturn.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::mm::phys::{self, FRAME_SIZE};

/// Fixed user-virtual address where every process maps the VDSO page.
///
/// Chosen to sit just below the user/kernel split, well away from typical
/// ELF segments and stack placement.
pub const VDSO_VADDR: u64 = 0x0000_7FFF_FFFE_F000;

/// Physical frame backing the VDSO page. Set once at boot by `init()`.
static VDSO_PHYS: AtomicU64 = AtomicU64::new(0);

/// Trampoline bytes:
///   48 c7 c0 1c 00 00 00    mov rax, 0x1c    ; SYS_sigreturn (28)
///   0f 05                   syscall
///   f4                      hlt              ; safety net
/// (10 bytes; remainder of the page is zero.)
const TRAMPOLINE: [u8; 10] = [
    0x48, 0xC7, 0xC0, 0x1C, 0x00, 0x00, 0x00,
    0x0F, 0x05,
    0xF4,
];

/// Initialise the VDSO. Allocates one frame, writes the trampoline, stores
/// the physical address for later mapping into user page tables.
///
/// # Safety
/// Must be called once at boot, after `mm::phys::init_from_memory_map` but
/// before any user process is constructed.
pub unsafe fn init() -> Result<(), &'static str> {
    let frame = phys::alloc_frame().map_err(|_| "VDSO frame allocation failed")?;
    let phys_addr = frame.addr();

    // Zero the page then write the trampoline at offset 0.
    core::ptr::write_bytes(phys_addr as *mut u8, 0, FRAME_SIZE);
    core::ptr::copy_nonoverlapping(
        TRAMPOLINE.as_ptr(),
        phys_addr as *mut u8,
        TRAMPOLINE.len(),
    );

    VDSO_PHYS.store(phys_addr, Ordering::Release);
    crate::serial::serial_println!(
        "[  0.000180] RACORE: VDSO initialised at phys 0x{:X}, mapped @ 0x{:X}",
        phys_addr,
        VDSO_VADDR,
    );
    Ok(())
}

/// Physical frame address of the VDSO page. Zero before `init()`.
pub fn page_phys() -> u64 {
    VDSO_PHYS.load(Ordering::Acquire)
}
