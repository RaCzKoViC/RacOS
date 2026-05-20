// RaCore — Syscall entry/exit and MSR setup for x86_64
//
// The SYSCALL instruction transfers control from ring 3 to ring 0.
// It saves RIP in RCX, RFLAGS in R11, then loads CS/SS from STAR MSR
// and jumps to the address in LSTAR MSR.
//
// On entry:
//   RAX = syscall number
//   RDI, RSI, RDX, R10, R8, R9 = args (R10 replaces RCX which is clobbered)
//   RCX = user RIP (saved by CPU)
//   R11 = user RFLAGS (saved by CPU)
//   RSP = user stack (NOT switched by CPU — we must switch manually)
//
// On exit (SYSRET):
//   RAX = return value
//   RCX = restored to user RIP
//   R11 = restored to user RFLAGS
//   RSP = user stack (we restore it)

use crate::arch::gdt;

/// On-kernel-stack layout the syscall entry trampoline builds before
/// dispatch and pops via `sysretq`. Mirrors the exact push order in the
/// naked asm: lowest-address field (`saved_arg6`) corresponds to the
/// last value pushed, highest-address field (`user_rsp`) to the first.
///
/// `repr(C)` is required because the layout is consumed from assembly
/// (via offsets) and Rust code (via field access). Field order MUST stay
/// in lock-step with the push sequence in `syscall_entry` — adding or
/// removing a push without updating this struct will silently corrupt
/// user state on `sysretq`.
///
/// Signal delivery (Task 9) mutates `user_rip`, `user_rflags`, `user_rsp`
/// to redirect to a user signal handler.
#[derive(Debug)]
#[repr(C)]
pub struct SyscallFrame {
    /// Saved arg6 (R9 from the original userland register set). The
    /// dispatcher reads this from the stack as its 7th C ABI argument.
    pub saved_arg6: u64,
    /// Callee-saved + caller-saved GPRs we preserved across dispatch.
    /// Order matches the push sequence (last push first in memory).
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64,
    /// User RIP saved by the CPU into RCX on `syscall`.
    pub user_rip: u64,
    /// User RFLAGS saved by the CPU into R11 on `syscall`.
    pub user_rflags: u64,
    /// User RSP captured by the trampoline from `gs:[0x08]`.
    pub user_rsp: u64,
}

// MSR addresses
const MSR_STAR: u32 = 0xC000_0081;
const MSR_LSTAR: u32 = 0xC000_0082;
const MSR_SFMASK: u32 = 0xC000_0084;
const MSR_EFER: u32 = 0xC000_0080;

// EFER bits
const EFER_SCE: u64 = 1 << 0; // System Call Extensions (enable SYSCALL/SYSRET)

// SFMASK: flags to clear on SYSCALL entry
// Clear IF (bit 9) to disable interrupts on entry, clear DF (bit 10), clear TF (bit 8)
const SFMASK_VALUE: u64 = (1 << 9) | (1 << 10) | (1 << 8);

/// Read a Model-Specific Register.
#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nomem, nostack),
    );
    ((high as u64) << 32) | (low as u64)
}

/// Write a Model-Specific Register.
#[inline]
unsafe fn wrmsr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") low,
        in("edx") high,
        options(nomem, nostack),
    );
}

/// The raw syscall entry point (LSTAR target).
///
/// This is a naked function that:
/// 1. Swaps to the kernel stack (using per-task kernel stack from TSS.RSP0)
/// 2. Saves user state
/// 3. Calls the Rust syscall dispatcher
/// 4. Restores user state
/// 5. Executes SYSRETQ
#[unsafe(naked)]
unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        // At this point:
        //   RCX = user RIP, R11 = user RFLAGS
        //   RSP = user stack, RAX = syscall number
        //   RDI/RSI/RDX/R10/R8/R9 = args
        //   Interrupts are disabled (SFMASK cleared IF)

        // Save user RSP to a scratch register, load kernel RSP
        // We use GS:0 or a fixed location for the per-CPU kernel stack pointer.
        // For MVP (UP), we use a global variable.
        "swapgs",                           // Switch to kernel GS base
        "mov gs:[0x08], rsp",               // Save user RSP to per-CPU area
        "mov rsp, gs:[0x00]",               // Load kernel RSP from per-CPU area

        // Build a trap frame on the kernel stack
        "push gs:[0x08]",                   // User RSP
        "push r11",                         // User RFLAGS
        "push rcx",                         // User RIP

        // Save callee-saved + caller-saved registers we need to preserve
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Save syscall args — R10 is arg4 (replaces RCX which is clobbered)
        // Set up C calling convention for syscall_dispatch:
        //   RDI = syscall_nr (currently in RAX)
        //   RSI = arg1 (currently in RDI)
        //   RDX = arg2 (currently in RSI)
        //   RCX = arg3 (currently in RDX)
        //   R8  = arg4 (currently in R10)
        //   R9  = arg5 (currently in R8)
        //   [stack] = arg6 (currently in R9)
        "push r9",                          // arg6 on stack

        // Publish a pointer to the on-stack SyscallFrame so that
        // signal-delivery code in syscall_dispatch can mutate the
        // user RIP/RSP/RFLAGS the trampoline is about to pop on
        // sysretq. RSP now points at saved_arg6 (offset 0 of the
        // SyscallFrame).
        "mov gs:[0x10], rsp",

        "mov r9, r8",                       // arg5
        "mov r8, r10",                      // arg4
        "mov rcx, rdx",                     // arg3
        "mov rdx, rsi",                     // arg2
        "mov rsi, rdi",                     // arg1
        "mov rdi, rax",                     // syscall number

        // Call Rust dispatcher
        "call {dispatch}",

        // Clear the frame pointer now that dispatch has returned.
        // Use an immediate-to-memory store so RAX (return value) is
        // preserved.
        "mov qword ptr gs:[0x10], 0",

        // RAX now contains the return value
        "add rsp, 8",                       // Pop arg6

        // Restore callee-saved registers
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",

        // Disable interrupts before swapping back to the user stack. Otherwise
        // a timer (or any device) IRQ that fires between `pop rsp` and
        // `sysretq` would push its IRET frame onto whatever address we just
        // loaded into RSP — and that's the user RSP. Result: silent corruption
        // of user-space memory and a bogus RCX (user RIP) when sysret runs,
        // which manifests as a #PF at RIP=0 right after heavy syscalls (spawn,
        // fork) that leave interrupts re-enabled. SYSRETQ will restore the
        // user RFLAGS (with IF set) from R11, so this only blocks IRQs for the
        // ~5 instructions until we leave ring 0.
        "cli",

        // Restore user state
        "pop rcx",                          // User RIP
        "pop r11",                          // User RFLAGS
        "pop rsp",                          // User RSP

        // Swap back to user GS base, then SYSRETQ:
        // - loads CS from STAR[63:48]+16, SS from STAR[63:48]+8
        // - RCX → RIP, R11 → RFLAGS (re-enables IF in user mode)
        "swapgs",
        "sysretq",

        dispatch = sym crate::syscall::dispatch::syscall_dispatch,
    );
}

/// Per-CPU data structure (minimal, UP only for MVP).
///
/// Layout (offsets are fixed — referenced by the naked syscall asm via
/// `gs:[offset]` and by the inline asm in `task::process`):
///   - 0x00: `kernel_rsp`         — current task's kernel stack top
///   - 0x08: `user_rsp`           — scratch slot to save user RSP on entry
///   - 0x10: `current_syscall_frame` — pointer to the `SyscallFrame` the
///       entry trampoline built on the kernel stack for the in-progress
///       syscall. Set by the asm immediately before `call dispatch`,
///       cleared after the dispatch returns. Read by `syscall_dispatch`
///       to surface the frame to the rest of the kernel via the current
///       task struct.
#[repr(C, align(16))]
pub(crate) struct PerCpuData {
    kernel_rsp: u64,
    user_rsp: u64,
    current_syscall_frame: u64,
}

pub(crate) static mut PER_CPU: PerCpuData = PerCpuData {
    kernel_rsp: 0,
    user_rsp: 0,
    current_syscall_frame: 0,
};

/// Read the per-CPU current-syscall-frame pointer set by the entry asm.
///
/// Returns 0 when not in a syscall (e.g., from an IRQ handler that
/// preempted kernel-mode execution).
///
/// # Safety
/// Must be called with interrupts disabled — otherwise a timer IRQ could
/// context-switch between read and use, leaving the returned pointer
/// dangling relative to the new task's kernel stack.
#[inline]
pub unsafe fn current_syscall_frame_ptr() -> u64 {
    let per_cpu = &*core::ptr::addr_of!(PER_CPU);
    per_cpu.current_syscall_frame
}

/// MSR for kernel GS base (used by SWAPGS).
const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;
/// MSR for GS base.
const MSR_GS_BASE: u32 = 0xC000_0100;

/// Initialize the SYSCALL/SYSRET mechanism.
///
/// # Safety
/// Must be called once from kernel_main after GDT and TSS are set up,
/// with interrupts disabled.
pub unsafe fn init() {
    // Enable SYSCALL/SYSRET in EFER
    let efer = rdmsr(MSR_EFER);
    wrmsr(MSR_EFER, efer | EFER_SCE);

    // Set STAR: kernelCS/SS = 0x08, SYSRET CS/SS base = 0x18
    wrmsr(MSR_STAR, gdt::STAR_VALUE);

    // Set LSTAR: entry point for SYSCALL
    wrmsr(MSR_LSTAR, syscall_entry as u64);

    // Set SFMASK: clear IF, DF, TF on SYSCALL entry
    wrmsr(MSR_SFMASK, SFMASK_VALUE);

    // Set up per-CPU data for kernel/user RSP swapping
    // For now, use the current RSP as kernel RSP (will be updated per-task later)
    let current_rsp: u64;
    core::arch::asm!("mov {}, rsp", out(reg) current_rsp, options(nomem, nostack));
    let per_cpu = &mut *core::ptr::addr_of_mut!(PER_CPU);
    per_cpu.kernel_rsp = current_rsp;

    // Set kernel GS base to our per-CPU data
    wrmsr(MSR_KERNEL_GS_BASE, core::ptr::addr_of!(PER_CPU) as u64);

    crate::serial::serial_println!(
        "[  0.000200] RACORE: SYSCALL/SYSRET initialized (LSTAR=0x{:X})",
        syscall_entry as u64
    );
}

/// Update the kernel RSP in the per-CPU data.
/// Called during context switch to set the correct kernel stack for the current task.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn set_kernel_rsp(rsp: u64) {
    let per_cpu = &mut *core::ptr::addr_of_mut!(PER_CPU);
    per_cpu.kernel_rsp = rsp;
}
