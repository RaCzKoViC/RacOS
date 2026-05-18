// RaCore — Task Context (CPU state saved/restored on context switch)
//
// On x86_64 System V ABI, the callee-saved registers are:
// RBX, RBP, R12, R13, R14, R15, RSP
// Plus RIP (instruction pointer) as the return address.
//
// We save these registers during context switch.
// Caller-saved registers (RAX, RCX, RDX, RSI, RDI, R8-R11) are
// already saved/restored by the interrupt frame or the caller.

/// CPU context saved during context switch.
/// Layout must match the assembly in `context_switch`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TaskContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub rip: u64,
}

impl TaskContext {
    pub const fn new() -> Self {
        TaskContext {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbx: 0,
            rbp: 0,
            rsp: 0,
            rip: 0,
        }
    }

    /// Create a new TaskContext for a user task.
    ///
    /// When switched to for the first time, this context will "return"
    /// to a trampoline that jumps to user space at `rip`.
    pub fn new_user(rip: u64, arg: u64, page_table: u64) -> Self {
        extern "C" {
            fn user_task_trampoline();
        }
        
        let mut context = Self::new();
        // The trampoline is written in assembly to secure ring transitions.
        // It resides in the kernel code but will switch to ring 3.
        context.rip = user_task_trampoline as u64;
        context.rbx = rip; // User RIP
        context.r12 = arg; // Thread argument (rdi in user side)
        context
    }
}

/// Switch from `old` context to `new` context.
///
/// Saves callee-saved registers into `old`, loads them from `new`.
/// After this function "returns", execution continues in the new task.
///
/// # Safety
/// - `old` must be a valid mutable pointer to the current task's context
/// - `new` must be a valid pointer to the target task's context
/// - The target task's stack and RIP must be valid
/// - Interrupts should be disabled by the caller
#[unsafe(naked)]
pub unsafe extern "C" fn context_switch(old: *mut TaskContext, new: *const TaskContext) {
    // SAFETY: This is the core context switch primitive.
    // We save the current cpu state to `old` and restore from `new`.
    // The `ret` at the end jumps to new.rip (the other task's saved return address).
    //
    // INVARIANT: old and new point to valid TaskContext structures.
    // FAILURE: If either pointer is invalid, undefined behavior (crash/corruption).
    // TESTED BY: scheduler integration tests (Sprint 4)
    core::arch::naked_asm!(
        // Save callee-saved registers to old context (RDI = old)
        "mov [rdi + 0x00], r15",
        "mov [rdi + 0x08], r14",
        "mov [rdi + 0x10], r13",
        "mov [rdi + 0x18], r12",
        "mov [rdi + 0x20], rbx",
        "mov [rdi + 0x28], rbp",
        "mov [rdi + 0x30], rsp",
        // Save return address as RIP
        "lea rax, [rip + 2f]",
        "mov [rdi + 0x38], rax",

        // Restore callee-saved registers from new context (RSI = new)
        "mov r15, [rsi + 0x00]",
        "mov r14, [rsi + 0x08]",
        "mov r13, [rsi + 0x10]",
        "mov r12, [rsi + 0x18]",
        "mov rbx, [rsi + 0x20]",
        "mov rbp, [rsi + 0x28]",
        "mov rsp, [rsi + 0x30]",

        // Jump to new task's saved RIP
        "jmp [rsi + 0x38]",

        // Label for the return point of the OLD task (when it's switched back to)
        "2:",
        "ret",
    );
}
