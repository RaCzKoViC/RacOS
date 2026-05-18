// RaCore — User Process model
//
// Extends the kernel task model to support user-space processes.
// A user process has:
// - Its own address space (page tables)
// - User-mode code/data segments
// - A kernel stack for handling syscalls/interrupts
// - A user stack in the user address space
//
// Ring 3 entry is done via IRETQ (initial entry) or SYSRETQ (return from syscall).

extern crate alloc;

use crate::arch::gdt;
use crate::elf::LoadedElf;
use crate::mm::{phys, virt};
use crate::mm::virt::flags as vflags;
use super::task::{Task, TaskState, KERNEL_STACK_PAGES, KERNEL_STACK_SIZE};
use super::context::TaskContext;
use super::signal::SignalState;

use core::sync::atomic::{AtomicU32, Ordering};

/// Process ID counter (shared with kernel tasks).
static NEXT_PID: AtomicU32 = AtomicU32::new(100); // User PIDs start at 100

pub fn alloc_user_pid() -> u32 {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

/// Saved user-space register state for IRETQ.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UserRegs {
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// A user-space process.
pub struct UserProcess {
    /// The underlying kernel task (for scheduler integration).
    pub task: Task,
    /// User-space entry point (for initial jump to ring 3).
    pub user_entry: u64,
    /// User stack top virtual address.
    pub user_stack_top: u64,
    /// Physical base of user stack (for cleanup).
    pub user_stack_phys: u64,
    /// Loaded ELF segments info (for cleanup).
    pub segment_bases: [u64; 8],
    pub segment_sizes: [usize; 8],
    pub segment_count: usize,
}

impl UserProcess {
    /// Create a new user process from a loaded ELF image.
    ///
    /// Allocates a kernel stack, sets up the initial context to jump to
    /// `user_entry_trampoline` which will IRETQ into user mode.
    pub fn from_elf(name: &str, loaded: &LoadedElf, argv: &[&[u8]]) -> Result<Self, &'static str> {
        let pid = alloc_user_pid();
        crate::serial::serial_println!("[ USERPROC ] from_elf('{}') pid={} start", name, pid);

        // Allocate kernel stack for this process
        let kernel_stack = phys::alloc_contiguous(KERNEL_STACK_PAGES)
            .map_err(|_| "Failed to allocate kernel stack")?;
        let kernel_stack_base = kernel_stack.addr();
        let kernel_stack_top = kernel_stack_base + KERNEL_STACK_SIZE as u64;

        // Zero kernel stack
        unsafe {
            core::ptr::write_bytes(kernel_stack_base as *mut u8, 0, KERNEL_STACK_SIZE);
        }
        crate::serial::serial_println!("[ USERPROC ] kernel stack allocated @ 0x{:X}", kernel_stack_base);

        // ── Push argv onto the user stack ─────────────────────────────────
        // Layout (growing downward from stack_virt_top):
        //   [argv string data]   ← null-terminated strings at top of stack
        //   [alignment padding]
        //   NULL (u64)           ← end of argv array
        //   argv[N-1] ptr (u64)  ← virtual address of argv[N-1] string
        //   ...
        //   argv[0] ptr (u64)    ← virtual address of argv[0] string
        //   argc (u64)           ← number of arguments
        //   ← RSP points here

        let stack_virt_base = loaded.stack_virt_top - loaded.stack_size as u64;
        // Offset in physical memory that corresponds to a virtual address
        let virt_to_phys = |vaddr: u64| -> u64 {
            loaded.stack_phys_base + (vaddr - stack_virt_base)
        };

        let mut sp = loaded.stack_virt_top; // current position (grows down)
        let argc = argv.len();

        // 1. Write string data at the top of the stack
        let mut string_vaddrs = alloc::vec::Vec::with_capacity(argc);
        for arg in argv.iter().rev() {
            sp -= 1; // null terminator
            unsafe { *(virt_to_phys(sp) as *mut u8) = 0; }
            sp -= arg.len() as u64;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    arg.as_ptr(),
                    virt_to_phys(sp) as *mut u8,
                    arg.len(),
                );
            }
            string_vaddrs.push(sp); // virtual address of this string
        }
        string_vaddrs.reverse(); // now in correct order (argv[0] first)

        // 2. Align SP to 8 bytes
        sp &= !7u64;

        // 3. Write NULL terminator for argv array
        sp -= 8;
        unsafe { *(virt_to_phys(sp) as *mut u64) = 0; }

        // 4. Write argv pointers (in reverse so argv[0] is at lowest address)
        for vaddr in string_vaddrs.iter().rev() {
            sp -= 8;
            unsafe { *(virt_to_phys(sp) as *mut u64) = *vaddr; }
        }
        let argv_ptr_vaddr = sp; // virtual address of argv[0] pointer

        // 5. Write argc
        sp -= 8;
        unsafe { *(virt_to_phys(sp) as *mut u64) = argc as u64; }

        // 6. Align SP to 16 bytes (System V ABI requires 16-byte aligned RSP at entry)
        sp &= !15u64;

        let user_rsp = sp;
        let _ = argv_ptr_vaddr; // used by _start to compute argv
        crate::serial::serial_println!("[ USERPROC ] argv/user stack prepared rsp=0x{:X}", user_rsp);

        // Set up the IRETQ frame at the top of the kernel stack.
        let iret_frame_size = 5 * 8; // 5 u64 values for IRETQ
        let iret_frame_start = kernel_stack_top - iret_frame_size;

        // RFLAGS: IF set (interrupts enabled), IOPL=0
        let user_rflags: u64 = 0x200; // IF bit

        unsafe {
            let frame = iret_frame_start as *mut u64;
            // IRETQ pops: RIP, CS, RFLAGS, RSP, SS (in that order)
            *frame.add(0) = loaded.entry_point;            // RIP
            *frame.add(1) = gdt::USER_CS as u64;           // CS
            *frame.add(2) = user_rflags;                   // RFLAGS
            *frame.add(3) = user_rsp;                      // RSP (adjusted for argv)
            *frame.add(4) = gdt::USER_DS as u64;            // SS
        }
        crate::serial::serial_println!("[ USERPROC ] iret frame prepared @ 0x{:X}", iret_frame_start);

        // Set up the task context so context_switch will jump to our trampoline.
        // The trampoline will set up segments and execute IRETQ.
        let mut context = TaskContext::new();
        context.rip = user_entry_trampoline as u64;
        // RSP points below the IRETQ frame — the trampoline will set data segments
        // and then the IRETQ frame is at RSP
        context.rsp = iret_frame_start;
        // RBX = pointer to TSS (so trampoline can update RSP0)
        context.rbx = kernel_stack_top;

        // Copy segment info for cleanup
        let mut seg_bases = [0u64; 8];
        let mut seg_sizes = [0usize; 8];
        for i in 0..loaded.segment_count {
            seg_bases[i] = loaded.segments[i].paddr;
            seg_sizes[i] = loaded.segments[i].memsz;
        }
        crate::serial::serial_println!("[ USERPROC ] segment metadata captured (count={})", loaded.segment_count);

        // ── Create user page table ─────────────────────────────────────────
        // Clone the kernel's current PML4 so the process inherits kernel
        // mappings needed for syscall entry/exit code.
        let pml4_phys = virt::create_user_page_table()
            .map_err(|_| "Failed to create user page table")?;
        crate::serial::serial_println!("[ USERPROC ] user page table created @ 0x{:X}", pml4_phys);

        // ── Map ELF segments into the user page table ──────────────────────
        for i in 0..loaded.segment_count {
            let seg = &loaded.segments[i];
            let pages = (seg.memsz + phys::FRAME_SIZE - 1) / phys::FRAME_SIZE;
            let page_flags = if seg.flags & 0x1 != 0 {
                // Executable segment: present, user, no NX
                vflags::USER_CODE
            } else if seg.flags & 0x2 != 0 {
                // Writable data/BSS: present, writable, user, NX
                vflags::USER_DATA
            } else {
                // Read-only data: present, user, NX
                vflags::USER_DATA & !vflags::WRITABLE
            };

            unsafe {
                virt::map_range(
                    pml4_phys,
                    seg.vaddr,
                    seg.paddr,
                    (pages * phys::FRAME_SIZE) as u64,
                    page_flags,
                ).map_err(|_| "Failed to map ELF segment")?;
            }
            crate::serial::serial_println!(
                "[ USERPROC ] mapped seg {} v=0x{:X} p=0x{:X} size=0x{:X}",
                i,
                seg.vaddr,
                seg.paddr,
                pages * phys::FRAME_SIZE
            );
        }

        // ── Map user stack ─────────────────────────────────────────────────
        let stack_pages = loaded.stack_size / phys::FRAME_SIZE;
        let stack_virt_base = loaded.stack_virt_top - loaded.stack_size as u64;
        unsafe {
            virt::map_range(
                pml4_phys,
                stack_virt_base,
                loaded.stack_phys_base,
                loaded.stack_size as u64,
                vflags::USER_DATA,
            ).map_err(|_| "Failed to map user stack")?;
        }
        crate::serial::serial_println!(
            "[ USERPROC ] mapped user stack v=0x{:X} p=0x{:X} size=0x{:X}",
            stack_virt_base,
            loaded.stack_phys_base,
            loaded.stack_size
        );

        let _ = stack_pages; // suppress unused warning

        let mut name_buf = [0u8; 32];
        let len = name.len().min(31);
        name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);

        let mut cwd_buf = [0u8; 256];
        cwd_buf[0] = b'/';

        Ok(UserProcess {
            task: Task {
                pid,
                parent_pid: crate::task::scheduler::current_pid(), // inherit caller's PID
                state: TaskState::Created,
                context,
                kernel_stack_base,
                page_table_phys: pml4_phys,
                exit_status: 0,
                signals: SignalState::new(),
                fd_table: crate::vfs::file::FdTable::new(),
                pgid: pid,
                session_id: crate::task::scheduler::current_pid(), // inherit parent's session
                creds: super::task::Credentials::root(),
                umask: 0o022,
                name: name_buf,
                name_len: len,
                cwd: cwd_buf,
                cwd_len: 1,
            },
            user_entry: loaded.entry_point,
            user_stack_top: loaded.stack_virt_top,
            user_stack_phys: loaded.stack_phys_base,
            segment_bases: seg_bases,
            segment_sizes: seg_sizes,
            segment_count: loaded.segment_count,
        })
    }
}

/// Trampoline for entering user mode for the first time.
///
/// Called via context_switch. Sets up user-mode segment registers
/// and executes IRETQ to jump to user space.
///
/// On entry:
///   RSP = points to the IRETQ frame (RIP, CS, RFLAGS, RSP, SS)
///   RBX = kernel stack top (for TSS RSP0)
#[unsafe(naked)]
unsafe extern "C" fn user_entry_trampoline() {
    core::arch::naked_asm!(
        // Set user data segment selectors (CS/SS come from the IRETQ frame).
        "mov ax, 0x1B",       // USER_DS = 0x18 | 3
        "mov ds, ax",
        "mov es, ax",
        // IRETQ pops: RIP, CS, RFLAGS, RSP, SS from current RSP, and the
        // popped RFLAGS already has IF set so interrupts come back enabled
        // in user mode.
        "iretq",
    );
}
