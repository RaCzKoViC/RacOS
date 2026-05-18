// RaCore — Task (Process/Thread) model
//
// Design decisions (ADR-006):
// - PID 0 = idle task (kernel), PID 1 = init
// - MVP: kernel tasks only (not user processes yet — that's Sprint 5)
// - Each task has its own kernel stack
// - Tasks are created with a function pointer (kernel thread entry)
// - State machine: Created → Ready → Running → Blocked → Zombie
//
// Invariants:
// - PID is unique and monotonically increasing
// - Only one task is Running at a time (UP scheduler)
// - A task's kernel stack must remain valid for its entire lifetime

use core::sync::atomic::{AtomicU32, Ordering};

extern crate alloc;

use super::context::TaskContext;
use super::signal::SignalState;
use crate::mm::phys::{self, FRAME_SIZE};
use crate::vfs::file::FdTable;

/// Task ID type.
pub type Pid = u32;

/// Next PID counter.
static NEXT_PID: AtomicU32 = AtomicU32::new(0);

pub(crate) fn alloc_pid_internal() -> Pid {
    NEXT_PID.fetch_add(1, Ordering::Relaxed)
}

fn alloc_pid() -> Pid {
    alloc_pid_internal()
}

/// Task state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Created,
    Ready,
    Running,
    Blocked,
    Zombie,
}

/// Kernel stack size: 16 KiB (4 frames).
pub const KERNEL_STACK_PAGES: usize = 16;
pub const KERNEL_STACK_SIZE: usize = KERNEL_STACK_PAGES * FRAME_SIZE;

/// Per-task credentials (Phase C MVP).
#[derive(Debug, Clone, Copy)]
pub struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    /// Capability masks (prepared for C2).
    pub cap_permitted: u64,
    pub cap_effective: u64,
    pub cap_inheritable: u64,
}

impl Credentials {
    pub const fn root() -> Self {
        Credentials {
            uid: 0,
            gid: 0,
            euid: 0,
            egid: 0,
            cap_permitted: u64::MAX,
            cap_effective: u64::MAX,
            cap_inheritable: u64::MAX,
        }
    }
}

/// A kernel task.
pub struct Task {
    pub pid: Pid,
    /// Parent PID (0 for kernel tasks).
    pub parent_pid: Pid,
    pub state: TaskState,
    pub context: TaskContext,
    /// Base address of the kernel stack (lowest address).
    pub kernel_stack_base: u64,
    /// Physical address of this task's PML4 page table.
    /// 0 = kernel task (no CR3 switch on context switch).
    pub page_table_phys: u64,
    /// Exit status (valid when state == Zombie).
    pub exit_status: i32,
    /// Signal state (pending + blocked bitmasks).
    pub signals: SignalState,
    /// Per-process file descriptor table.
    pub fd_table: FdTable,
    /// Process group ID (defaults to own PID).
    pub pgid: Pid,
    /// Session ID (defaults to own PID).
    pub session_id: Pid,
    /// Security credentials.
    pub creds: Credentials,
    /// Per-process file mode creation mask.
    pub umask: u32,
    /// Name for debugging.
    pub name: [u8; 32],
    pub name_len: usize,
    /// Current working directory (absolute path, no trailing slash except root).
    pub cwd: [u8; 256],
    pub cwd_len: usize,
}

impl Task {
    /// Create a new kernel task that will start executing `entry_fn`.
    ///
    /// Allocates a kernel stack and sets up the initial context so that
    /// when the scheduler switches to this task for the first time,
    /// it will begin at `entry_fn`.
    pub fn new_kernel(name: &str, entry_fn: fn() -> !) -> Result<Self, &'static str> {
        let pid = alloc_pid();

        // Allocate kernel stack
        let stack_frame = phys::alloc_contiguous(KERNEL_STACK_PAGES)
            .map_err(|_| "Failed to allocate kernel stack")?;
        let stack_base = stack_frame.addr();
        let stack_top = stack_base + KERNEL_STACK_SIZE as u64;

        // Zero the stack
        unsafe {
            core::ptr::write_bytes(stack_base as *mut u8, 0, KERNEL_STACK_SIZE);
        }

        // Set up initial context
        // When context_switch restores this context, it will pop callee-saved
        // registers and `ret` to the instruction pointer we set here.
        let mut context = TaskContext::new();
        // The entry point — context_switch will `ret` to this address
        context.rip = task_entry_trampoline as u64;
        // RSP points to where we've set up our fake stack frame
        // We push a return address (entry_fn) onto the stack
        let initial_rsp = stack_top - 8; // Space for the "return address"
        unsafe {
            // The trampoline will read RBX as the real entry function
            *(initial_rsp as *mut u64) = 0; // Dummy return address (task_entry_trampoline never returns)
        }
        context.rsp = initial_rsp;
        context.rbx = entry_fn as u64; // Trampoline reads RBX to call the real entry

        let mut name_buf = [0u8; 32];
        let len = name.len().min(31);
        name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);

        let mut cwd_buf = [0u8; 256];
        cwd_buf[0] = b'/';

        Ok(Task {
            pid,
            parent_pid: 0,
            state: TaskState::Created,
            context,
            kernel_stack_base: stack_base,
            page_table_phys: 0, // kernel task — no separate page table
            exit_status: 0,
            signals: SignalState::new(),
            fd_table: FdTable::new(),
            pgid: pid,
            session_id: pid,
            creds: Credentials::root(),
            umask: 0o022,
            name: name_buf,
            name_len: len,
            cwd: cwd_buf,
            cwd_len: 1,
        })
    }

    /// Create the idle task (PID 0) representing the boot context.
    /// Context will be filled in by the scheduler on first switch.
    pub fn idle() -> Self {
        let pid = alloc_pid(); // Should be 0
        let mut name_buf = [0u8; 32];
        name_buf[..4].copy_from_slice(b"idle");
        let mut cwd_buf = [0u8; 256];
        cwd_buf[0] = b'/';
        Task {
            pid,
            parent_pid: 0,
            state: TaskState::Running,
            context: TaskContext::new(),
            kernel_stack_base: 0, // Uses the boot stack
            page_table_phys: 0,   // kernel task — no separate page table
            exit_status: 0,
            signals: SignalState::new(),
            fd_table: FdTable::new(),
            pgid: pid,
            session_id: pid,
            creds: Credentials::root(),
            umask: 0o022,
            name: name_buf,
            name_len: 4,
            cwd: cwd_buf,
            cwd_len: 1,
        }
    }
}

/// Trampoline for new kernel tasks.
///
/// The context switch `ret`s here. RBX holds the actual entry function pointer.
/// We enable interrupts (they're disabled during context switch) and call the entry.
#[unsafe(naked)]
unsafe extern "C" fn task_entry_trampoline() -> ! {
    // SAFETY: This is a naked function that serves as the entry point
    // for new tasks after their first context switch.
    // RBX was set to the entry function in Task::new_kernel.
    core::arch::naked_asm!(
        "sti",       // Re-enable interrupts
        "call rbx",  // Call the actual entry function (fn() -> !)
        "ud2",       // Should never reach here
    );
}
