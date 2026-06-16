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

use super::context::TaskContext;
use super::signal::SignalState;
use super::task::{Task, TaskState, KERNEL_STACK_PAGES, KERNEL_STACK_SIZE};
use crate::arch::gdt;
use crate::elf::LoadedElf;
use crate::mm::virt::flags as vflags;
use crate::mm::{phys, virt};

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
    pub fn from_elf(
        name: &str,
        loaded: &LoadedElf,
        argv: &[&[u8]],
        envp: &[&[u8]],
    ) -> Result<Self, &'static str> {
        let pid = alloc_user_pid();
        crate::serial::serial_println!("[ USERPROC ] from_elf('{}') pid={} start", name, pid);

        // Allocate kernel stack + guard page (see KERNEL_STACK_GUARD_PAGES).
        // Layout: [guard page] [usable stack ...]. Overflow off the bottom of
        // the usable stack walks into the guard page and is caught by
        // scheduler.check_kernel_stack_guard on the next context switch.
        let total_pages = KERNEL_STACK_PAGES + super::task::KERNEL_STACK_GUARD_PAGES;
        let alloc_frame = phys::alloc_contiguous(total_pages)
            .map_err(|_| "Failed to allocate kernel stack + guard")?;
        let alloc_base = alloc_frame.addr();
        let kernel_stack_base =
            alloc_base + (super::task::KERNEL_STACK_GUARD_PAGES * phys::FRAME_SIZE) as u64;
        let kernel_stack_top = kernel_stack_base + KERNEL_STACK_SIZE as u64;

        // SAFETY: zero/pattern-fill the freshly-allocated kernel stack +
        // guard frames.
        // WHY: write_bytes is the only no-alloc way to clear a raw frame range
        //   that doesn't have a typed reference yet.
        // INVARIANT: alloc_contiguous(total_pages) above just returned this
        //   range, so [alloc_base, alloc_base + total_pages * FRAME_SIZE) is
        //   the exclusive owner of these physical bytes and identity-mapped
        //   into the kernel address space.
        // FAILURE: a buggy phys allocator returning an already-owned frame
        //   would have us scribbling over someone else's stack. Mitigated by
        //   alloc_contiguous tracking and the guard page byte pattern that
        //   surfaces overflows on the next context switch.
        unsafe {
            core::ptr::write_bytes(
                alloc_base as *mut u8,
                super::task::KERNEL_STACK_GUARD_BYTE,
                super::task::KERNEL_STACK_GUARD_PAGES * phys::FRAME_SIZE,
            );
            core::ptr::write_bytes(kernel_stack_base as *mut u8, 0, KERNEL_STACK_SIZE);
        }
        crate::serial::serial_println!(
            "[ USERPROC ] kernel stack @ 0x{:X} (guard @ 0x{:X})",
            kernel_stack_base,
            alloc_base
        );

        // ── Push argv + envp onto the user stack ──────────────────────────
        // System V AMD64 ABI on entry:
        //   [rsp+0]                = argc
        //   [rsp+8 .. +8N]         = argv[0..N-1]
        //   [rsp+8(N+1)]           = NULL          (argv terminator)
        //   [rsp+8(N+2) .. +8(N+2+M-1)] = envp[0..M-1]
        //   [rsp+8(N+2+M)]         = NULL          (envp terminator)
        //   rsp                    must be 16-byte aligned
        //
        // Earlier history: this routine aligned to 16 AFTER writing argc,
        // which could shift sp downward by 8 — argc would end up at
        // [user_rsp+8] instead of [user_rsp+0] and _start would read 0
        // (alignment pad) as argc. Now we compute the exact block size up
        // front and align sp down to it.
        //
        // Layout (growing downward from stack_virt_top):
        //   [argv + envp string data ...]  ← null-terminated bytes
        //   [padding to 16-byte align]
        //   NULL                            ← envp terminator
        //   envp[M-1] ptr
        //   ...
        //   envp[0] ptr
        //   NULL                            ← argv terminator
        //   argv[N-1] ptr
        //   ...
        //   argv[0] ptr
        //   argc                             ← user_rsp, 16-byte aligned

        let stack_virt_base = loaded.stack_virt_top - loaded.stack_size as u64;
        let virt_to_phys =
            |vaddr: u64| -> u64 { loaded.stack_phys_base + (vaddr - stack_virt_base) };

        let mut sp = loaded.stack_virt_top;
        let argc = argv.len();
        let envc = envp.len();

        // 1. Write argv + envp string data at the top of the stack. envp
        //    strings come "below" argv in virtual address order, but that's
        //    fine — we just need to remember each one's virtual address.
        let mut argv_vaddrs = alloc::vec::Vec::with_capacity(argc);
        for arg in argv.iter().rev() {
            sp -= 1;
            // SAFETY: write the NUL terminator for this argv string.
            // WHY: raw write into a freshly-allocated user-stack frame —
            //   no &mut [u8] exists for the mapping yet because nothing has
            //   set it up as a typed Rust slice.
            // INVARIANT: virt_to_phys maps sp into [stack_phys_base,
            //   stack_phys_base + stack_size). Each loop iteration only
            //   decrements sp, never increments, and the running guards
            //   (block_bytes_aligned + string lengths) leave headroom.
            // FAILURE: a too-large argv array would push sp below
            //   stack_phys_base and corrupt whatever sits there. Mitigated
            //   by the user-side path computing block_bytes_aligned before
            //   this loop and the caller (sys_spawn) capping argv at
            //   MAX_ARGS = 64.
            unsafe {
                *(virt_to_phys(sp) as *mut u8) = 0;
            }
            sp -= arg.len() as u64;
            // SAFETY: copy the argv string bytes into user stack at the
            // address we just reserved.
            // INVARIANT: arg lives in the kernel heap (alloc::Vec<u8>) and
            //   the destination range [sp, sp + arg.len()) is fresh user
            //   stack memory carved out by the just-previous decrements.
            //   The two ranges cannot overlap because the destination is
            //   user-virtual + identity-mapped, the source is kernel-heap.
            // FAILURE: same overflow story as above; mitigated the same way.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    arg.as_ptr(),
                    virt_to_phys(sp) as *mut u8,
                    arg.len(),
                );
            }
            argv_vaddrs.push(sp);
        }
        argv_vaddrs.reverse();

        let mut envp_vaddrs = alloc::vec::Vec::with_capacity(envc);
        for var in envp.iter().rev() {
            sp -= 1;
            // SAFETY: NUL terminator for this envp entry. Same justification
            // as the argv NUL write above; envp entries are sized + capped
            // by sys_spawn's collect_user_envp (MAX_ENV_VARS = 256).
            unsafe {
                *(virt_to_phys(sp) as *mut u8) = 0;
            }
            sp -= var.len() as u64;
            // SAFETY: copy envp entry bytes into user stack — same argument
            // as the argv copy above. Source (kernel heap) and destination
            // (user stack identity map) cannot alias.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    var.as_ptr(),
                    virt_to_phys(sp) as *mut u8,
                    var.len(),
                );
            }
            envp_vaddrs.push(sp);
        }
        envp_vaddrs.reverse();

        // 2. Drop sp to a 16-byte boundary so the pointer block stays aligned.
        sp &= !15u64;

        // 3. Reserve the argc/argv/envp block, rounded up to 16 bytes.
        //    Layout inside the block (low → high addresses):
        //      argc, argv[0], ..., argv[N-1], NULL,
        //      envp[0], ..., envp[M-1], NULL,
        //      [optional padding].
        let block_bytes = 8 + 8 * (argc as u64 + 1) + 8 * (envc as u64 + 1);
        let block_bytes_aligned = (block_bytes + 15) & !15;
        sp -= block_bytes_aligned;

        let user_rsp = sp; // 16-byte aligned

        // 4. Populate the block at fixed offsets from user_rsp.
        //
        // SAFETY: write argc + argv pointer array + envp pointer array into
        // the user stack at known offsets.
        // WHY: at this point no &mut [u64] exists over the freshly-allocated
        //   user stack — these are raw u64 writes through identity-mapped
        //   physical addresses.
        // INVARIANT: sp -= block_bytes_aligned above carved the exact byte
        //   count needed for `argc, argv[0..N-1], NULL, envp[0..M-1], NULL`
        //   plus 16-byte alignment padding. Every offset computed below
        //   (`user_rsp + 8 + 8 * k`) falls within that carved region for
        //   k in 0..=argc + 1 + envc + 1, which is the loop range used.
        // FAILURE: a wrong offset arithmetic here would either corrupt the
        //   string data sitting just above the block (off-by-too-much) or
        //   leak unrelated stack content into the argc/argv view
        //   _start sees. Mitigated by the block_bytes formula matching the
        //   index arithmetic line-for-line.
        unsafe {
            // argc at [rsp+0]
            *(virt_to_phys(user_rsp) as *mut u64) = argc as u64;
            // argv[i] at [rsp + 8*(i+1)]
            for (i, vaddr) in argv_vaddrs.iter().enumerate() {
                let slot = user_rsp + 8 + 8 * i as u64;
                *(virt_to_phys(slot) as *mut u64) = *vaddr;
            }
            // argv NULL terminator at [rsp + 8*(argc+1)]
            let argv_null = user_rsp + 8 + 8 * argc as u64;
            *(virt_to_phys(argv_null) as *mut u64) = 0;
            // envp[i] at [rsp + 8*(argc+2+i)]
            for (i, vaddr) in envp_vaddrs.iter().enumerate() {
                let slot = user_rsp + 8 + 8 * (argc as u64 + 1 + i as u64 + 1);
                *(virt_to_phys(slot) as *mut u64) = *vaddr;
            }
            // envp NULL terminator at [rsp + 8*(argc+2+M)]
            let envp_null = user_rsp + 8 + 8 * (argc as u64 + 1 + envc as u64 + 1);
            *(virt_to_phys(envp_null) as *mut u64) = 0;
        }

        crate::serial::serial_println!(
            "[ USERPROC ] argv+envp/user stack prepared rsp=0x{:X} argc={} envc={} block_bytes={}",
            user_rsp,
            argc,
            envc,
            block_bytes_aligned,
        );

        // Set up the IRETQ frame at the top of the kernel stack.
        let iret_frame_size = 5 * 8; // 5 u64 values for IRETQ
        let iret_frame_start = kernel_stack_top - iret_frame_size;

        // RFLAGS: IF set (interrupts enabled), IOPL=0
        let user_rflags: u64 = 0x200; // IF bit

        // SAFETY: write the 5-slot IRETQ frame at the top of the freshly-
        // allocated kernel stack.
        // WHY: the trampoline will execute IRETQ over this frame to jump
        //   into ring-3 — there's no safe Rust wrapper for "transition to
        //   user mode".
        // INVARIANT: kernel_stack_top was computed from the contiguous
        //   allocation above (lines 75-77) and iret_frame_start sits 40
        //   bytes below it, inside the usable stack region. The frame is
        //   contiguous and we're the only writer.
        // FAILURE: a wrong frame layout (re-ordering RIP/CS/RFLAGS/RSP/SS)
        //   would IRETQ to a garbage RIP and triple-fault. Mitigated by
        //   matching the Intel SDM Vol 1 §6.14 order documented inline.
        unsafe {
            let frame = iret_frame_start as *mut u64;
            // IRETQ pops: RIP, CS, RFLAGS, RSP, SS (in that order)
            *frame.add(0) = loaded.entry_point; // RIP
            *frame.add(1) = gdt::USER_CS as u64; // CS
            *frame.add(2) = user_rflags; // RFLAGS
            *frame.add(3) = user_rsp; // RSP (adjusted for argv)
            *frame.add(4) = gdt::USER_DS as u64; // SS
        }
        crate::serial::serial_println!(
            "[ USERPROC ] iret frame prepared @ 0x{:X}",
            iret_frame_start
        );

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
        crate::serial::serial_println!(
            "[ USERPROC ] segment metadata captured (count={})",
            loaded.segment_count
        );

        // ── Create user page table ─────────────────────────────────────────
        // Clone the kernel's current PML4 so the process inherits kernel
        // mappings needed for syscall entry/exit code.
        let pml4_phys =
            virt::create_user_page_table().map_err(|_| "Failed to create user page table")?;
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

            // SAFETY: install per-segment user-mode mappings in the
            // freshly-allocated PML4.
            // WHY: virt::map_range walks raw paging structures and writes
            //   table entries — no safe wrapper exists for "edit page tables
            //   in place".
            // INVARIANT: pml4_phys was just allocated by
            //   virt::alloc_page_table above and is exclusively owned by
            //   this UserProcess until we hand it off to the scheduler;
            //   seg.paddr came out of elf::load_elf which sized the segment
            //   to `pages * FRAME_SIZE`.
            // FAILURE: a mistaken vaddr would shadow kernel memory and let
            //   user code escalate. Mitigated by USER_CODE/USER_DATA flag
            //   selection above and by validate_user_ptr checks in the
            //   syscall path.
            unsafe {
                virt::map_range(
                    pml4_phys,
                    seg.vaddr,
                    seg.paddr,
                    (pages * phys::FRAME_SIZE) as u64,
                    page_flags,
                )
                .map_err(|_| "Failed to map ELF segment")?;
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
        // SAFETY: install the user-stack mapping in the same PML4 we just
        // populated with ELF segments above.
        // WHY/INVARIANT/FAILURE: same as the per-segment map_range above.
        //   Mapping is USER_DATA (writable, NX) because the stack is
        //   non-executable.
        unsafe {
            virt::map_range(
                pml4_phys,
                stack_virt_base,
                loaded.stack_phys_base,
                loaded.stack_size as u64,
                vflags::USER_DATA,
            )
            .map_err(|_| "Failed to map user stack")?;
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

        // Unconditionally normalise the GS/KERNEL_GS_BASE pair before
        // entering ring 3. The trampoline can be reached from two very
        // different kernel-mode contexts:
        //   (A) directly from the boot/idle path, where GS_BASE is the
        //       initial 0 and KERNEL_GS_BASE is the per-CPU pointer
        //       (already set during syscall::entry::init).
        //   (B) mid-syscall, after some other task hit swapgs in
        //       syscall_entry and block_and_reschedule'd into us — here
        //       GS_BASE points at PER_CPU and KERNEL_GS_BASE is the
        //       previous task's user GS (typically 0).
        // A blanket `swapgs` fixes (B) but breaks (A); avoiding swapgs
        // does the opposite. Setting both MSRs explicitly works for both:
        // user gets GS_BASE = 0, the next syscall's swapgs cleanly flips
        // it to the per-CPU pointer.
        //
        // wrmsr clobbers ECX/EAX/EDX; nothing in this trampoline cares
        // about their values at IRETQ time (the IRET frame restores
        // user-visible registers).

        // GS_BASE = 0
        "xor eax, eax",
        "xor edx, edx",
        "mov ecx, 0xC0000100",      // MSR_GS_BASE
        "wrmsr",

        // KERNEL_GS_BASE = &PER_CPU
        "lea rax, [rip + {per_cpu}]",
        "mov rdx, rax",
        "shr rdx, 32",
        "mov ecx, 0xC0000102",      // MSR_KERNEL_GS_BASE
        "wrmsr",

        // IRETQ pops: RIP, CS, RFLAGS, RSP, SS from current RSP. The popped
        // RFLAGS already has IF set so interrupts are enabled in user mode.
        "iretq",
        per_cpu = sym crate::syscall::entry::PER_CPU,
    );
}
