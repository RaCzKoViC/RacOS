// RaCore — Round-Robin Scheduler (MVP)
//
// Design decisions (ADR-007):
// - Round-robin with fixed time quantum (10ms = 10 ticks at 1000 Hz)
// - UP only (single CPU, no SMP)
// - Cooperative yield + preemptive via timer IRQ
// - Ready queue: simple ring buffer of task indices
//
// Invariants:
// - Exactly one task is Running at any time
// - The idle task (PID 0) is always runnable
// - Context switch happens with interrupts disabled
// - schedule() is called from the timer IRQ handler

extern crate alloc;

use alloc::vec::Vec;
use super::task::{Task, TaskState, Pid};
use super::context;
use super::process::UserProcess;
use crate::mm::virt;

/// Time quantum in ticks (10ms at 1000 Hz).
const TIME_QUANTUM: u64 = 10;

/// Maximum number of tasks.
const MAX_TASKS: usize = 64;

/// The scheduler state.
pub struct Scheduler {
    /// All tasks (index = task slot).
    tasks: Vec<Option<Task>>,
    /// Index of the currently running task.
    current: usize,
    /// Tick counter for preemption.
    ticks_remaining: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            tasks: Vec::new(),
            current: 0,
            ticks_remaining: TIME_QUANTUM,
        }
    }

    /// Initialize the scheduler with the idle task (PID 0).
    /// Called once from kernel_main.
    pub fn init(&mut self) {
        let idle = Task::idle();
        self.tasks.push(Some(idle));
        self.current = 0;
        self.ticks_remaining = TIME_QUANTUM;

        crate::serial::serial_println!(
            "[  0.000350] RACORE: Scheduler ready (round-robin, quantum={}ms)",
            TIME_QUANTUM
        );
    }

    /// Spawn a new kernel task.
    pub fn spawn(&mut self, name: &str, entry: fn() -> !) -> Result<Pid, &'static str> {
        if self.tasks.len() >= MAX_TASKS {
            return Err("Maximum task limit reached");
        }

        let mut task = Task::new_kernel(name, entry)?;
        let pid = task.pid;
        task.state = TaskState::Ready;

        // Find an empty slot or push new
        let mut slot = None;
        for (i, s) in self.tasks.iter().enumerate() {
            if s.is_none() {
                slot = Some(i);
                break;
            }
        }

        match slot {
            Some(idx) => self.tasks[idx] = Some(task),
            None => self.tasks.push(Some(task)),
        }

        crate::serial::serial_println!(
            "[  SCHED  ] Task '{}' spawned (PID {})",
            name,
            pid
        );

        Ok(pid)
    }

    /// Spawn a user-space process from a loaded ELF.
    pub fn spawn_user(&mut self, mut process: UserProcess) -> Result<Pid, &'static str> {
        if self.tasks.len() >= MAX_TASKS {
            return Err("Maximum task limit reached");
        }

        let pid = process.task.pid;
        let name_len = process.task.name_len;
        let name_bytes = process.task.name;
        let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("?");
        process.task.state = TaskState::Ready;

        let mut slot = None;
        for (i, s) in self.tasks.iter().enumerate() {
            if s.is_none() {
                slot = Some(i);
                break;
            }
        }

        match slot {
            Some(idx) => self.tasks[idx] = Some(process.task),
            None => self.tasks.push(Some(process.task)),
        }

        crate::serial::serial_println!(
            "[  SCHED  ] User process '{}' spawned (PID {})",
            name,
            pid
        );

        Ok(pid)
    }

    /// Spawn a new user thread within an existing process space.
    pub fn spawn_thread(&mut self, routine: u64, arg: u64, parent_task: &Task) -> Result<Pid, &'static str> {
        let _ = (routine, arg, parent_task);
        Err("pthread_create thread bootstrap not wired yet")
    }

    /// Called from the timer IRQ handler.
    /// Decrements the time quantum and preempts if expired.
    pub fn timer_tick(&mut self) {
        if self.ticks_remaining > 0 {
            self.ticks_remaining -= 1;
        }
        if self.ticks_remaining == 0 {
            self.schedule();
        }
    }

    /// Select the next task and perform context switch.
    pub fn schedule(&mut self) {
        // Defensive: validate every task's kernel-stack guard page before
        // picking the next task. Catches kernel stack overflow that would
        // otherwise silently corrupt adjacent allocations (this exact bug
        // bit us with the 4-page kernel stack + sys_spawn).
        self.check_kernel_stack_guards();

        let task_count = self.tasks.len();
        if task_count <= 1 {
            self.ticks_remaining = TIME_QUANTUM;
            return; // Only idle task, nothing to switch to
        }

        // Find next ready task (round-robin scan)
        let mut next = (self.current + 1) % task_count;
        let start = next;
        loop {
            if let Some(ref task) = self.tasks[next] {
                if task.state == TaskState::Ready {
                    break;
                }
            }
            next = (next + 1) % task_count;
            if next == start {
                // No ready tasks found — stay on current (or idle)
                self.ticks_remaining = TIME_QUANTUM;
                return;
            }
        }

        if next == self.current {
            self.ticks_remaining = TIME_QUANTUM;
            return;
        }

        // Perform context switch
        let old_idx = self.current;
        self.current = next;
        self.ticks_remaining = TIME_QUANTUM;

        // Update states
        if let Some(ref mut old_task) = self.tasks[old_idx] {
            if old_task.state == TaskState::Running {
                old_task.state = TaskState::Ready;
            }
        }
        if let Some(ref mut new_task) = self.tasks[next] {
            new_task.state = TaskState::Running;
        }

        // Get raw pointers to contexts
        let old_ctx = &mut self.tasks[old_idx].as_mut().unwrap().context as *mut _;
        let new_ctx = &self.tasks[next].as_ref().unwrap().context as *const _;

        // Switch page table if the incoming task has its own address space.
        // We switch before context_switch so the new task's virtual mappings
        // are live when its registers take effect.
        let new_pt = self.tasks[next].as_ref().unwrap().page_table_phys;
        let old_pt = self.tasks[old_idx].as_ref().unwrap().page_table_phys;
        if new_pt != 0 && new_pt != old_pt {
            // SAFETY: new_pt was created by virt::create_user_page_table() and
            // contains valid kernel mappings so the subsequent context_switch
            // (kernel code) remains reachable after the CR3 write.
            unsafe { virt::write_cr3(new_pt); }
        } else if new_pt == 0 && old_pt != 0 {
            // Switching back to a kernel task — keep current CR3. Kernel
            // mappings are shared in every process page table so this is safe.
        }

        // SAFETY: Both contexts are valid, interrupts are disabled (called from IRQ handler).
        // INVARIANT: old_ctx and new_ctx point to different tasks.
        // FAILURE: Invalid context → crash (caught by panic handler).
        // Program ring0 stack for the incoming task before switching.
        let incoming_kernel_stack_top = self.tasks[next]
            .as_ref()
            .map(|t| {
                if t.kernel_stack_base != 0 {
                    t.kernel_stack_base
                        + crate::mm::phys::FRAME_SIZE as u64 * super::task::KERNEL_STACK_PAGES as u64
                } else {
                    0
                }
            })
            .unwrap_or(0);
        if incoming_kernel_stack_top != 0 {
            unsafe {
                crate::syscall::entry::set_kernel_rsp(incoming_kernel_stack_top);
                crate::arch::gdt::set_kernel_stack(incoming_kernel_stack_top);
            }
        }

        unsafe {
            context::context_switch(old_ctx, new_ctx);
        }

        // NOTE: context_switch returns when THIS (old) task is scheduled back
        // in the future. The incoming task's kernel stack pointers are already
        // programmed before the switch above; touching them here would race
        // with later schedule decisions and can corrupt RSP0/kernel-RSP state.
    }

    /// Walk every task's kernel-stack guard page and verify it still holds
    /// the sentinel pattern. If it doesn't, that task overflowed its kernel
    /// stack — panic loudly with the task PID so we know what to blame.
    ///
    /// Cheap: 8 bytes read per task per schedule call. The guard page is
    /// `super::task::KERNEL_STACK_GUARD_PAGES` × `FRAME_SIZE` bytes
    /// immediately *below* `kernel_stack_base`.
    fn check_kernel_stack_guards(&self) {
        for slot in self.tasks.iter().flatten() {
            if slot.kernel_stack_base == 0 {
                continue; // idle task or similar — no allocated stack
            }
            let guard_addr = slot.kernel_stack_base
                - (super::task::KERNEL_STACK_GUARD_PAGES * crate::mm::phys::FRAME_SIZE) as u64;
            // Check the first 8 bytes of the guard page. If any of those is
            // not the sentinel, overflow happened.
            let first_qword = unsafe { core::ptr::read_volatile(guard_addr as *const u64) };
            let expected_byte = super::task::KERNEL_STACK_GUARD_BYTE as u64;
            let expected_qword = expected_byte
                | (expected_byte << 8)
                | (expected_byte << 16)
                | (expected_byte << 24)
                | (expected_byte << 32)
                | (expected_byte << 40)
                | (expected_byte << 48)
                | (expected_byte << 56);
            if first_qword != expected_qword {
                panic!(
                    "kernel stack overflow detected for PID {} (guard @ 0x{:X} = 0x{:X}, expected 0x{:X})",
                    slot.pid, guard_addr, first_qword, expected_qword,
                );
            }
        }
    }

    /// Yield the current task voluntarily.
    pub fn yield_current(&mut self) {
        self.ticks_remaining = 0;
        self.schedule();
    }

    /// Terminate the current task, setting it to Zombie, then wake its parent
    /// and post SIGCHLD. Closes user-visible file descriptors early so pipes
    /// and inodes see their refcount drop before the parent reaps.
    pub fn exit_current(&mut self, status: i32) {
        let idx = self.current;
        let (my_pid, parent_pid) = if let Some(ref mut task) = self.tasks[idx] {
            task.state = TaskState::Zombie;
            task.exit_status = status;
            // Drop every file descriptor right now. The Arc<OpenFile> refs
            // are released, which makes pipes/inodes observe the close.
            task.fd_table.close_all();
            (task.pid, task.parent_pid)
        } else { (0, 0) };

        // Reparent orphan children to init (PID 100) so they can be reaped.
        const INIT_PID: Pid = 100;
        if my_pid != 0 {
            for slot in self.tasks.iter_mut().flatten() {
                if slot.parent_pid == my_pid && slot.pid != my_pid {
                    slot.parent_pid = INIT_PID;
                }
            }
        }

        // Notify the parent: post SIGCHLD and wake it if it was blocked.
        if parent_pid != 0 {
            for slot in self.tasks.iter_mut().flatten() {
                if slot.pid == parent_pid {
                    slot.signals.send(super::signal::Signal::SIGCHLD);
                    if matches!(slot.state, TaskState::Blocked) {
                        slot.state = TaskState::Ready;
                    }
                    break;
                }
            }
        }

        // Schedule another task immediately.
        self.schedule();
    }

    fn child_matches_wait_filter(
        task: &Task,
        parent_pid: Pid,
        pid_filter: i32,
        parent_pgid: Pid,
    ) -> bool {
        if task.parent_pid != parent_pid {
            return false;
        }

        match pid_filter {
            -1 => true,
            0 => task.pgid == parent_pgid,
            p if p > 0 => task.pid == p as Pid,
            p if p < -1 => p.checked_neg().map(|g| task.pgid == g as Pid).unwrap_or(false),
            _ => false,
        }
    }

    /// Find the first zombie child matching a wait filter, clean up resources,
    /// remove it from the task list, and return (child_pid, exit_status).
    pub fn reap_zombie_child_filtered(
        &mut self,
        parent_pid: Pid,
        pid_filter: i32,
        parent_pgid: Pid,
    ) -> Option<(Pid, i32)> {
        for slot in self.tasks.iter_mut() {
            if let Some(ref task) = slot {
                if Self::child_matches_wait_filter(task, parent_pid, pid_filter, parent_pgid)
                    && matches!(task.state, TaskState::Zombie)
                {
                    let pid = task.pid;
                    let status = task.exit_status;
                    let page_table_phys = task.page_table_phys;
                    let kernel_stack_base = task.kernel_stack_base;

                    // Remove the task (drops FdTable, SignalState, etc.)
                    *slot = None;

                    // Free the user page table (and all mapped user frames)
                    if page_table_phys != 0 {
                        unsafe { crate::mm::virt::free_page_table(page_table_phys, true); }
                    }

                    // Free the kernel stack PLUS the guard page that lives
                    // immediately below it (same single contiguous alloc).
                    if kernel_stack_base != 0 {
                        let alloc_base = kernel_stack_base
                            - (super::task::KERNEL_STACK_GUARD_PAGES * crate::mm::phys::FRAME_SIZE) as u64;
                        let total_pages = super::task::KERNEL_STACK_PAGES
                            + super::task::KERNEL_STACK_GUARD_PAGES;
                        for i in 0..total_pages {
                            let addr = alloc_base + (i * crate::mm::phys::FRAME_SIZE) as u64;
                            let _ = crate::mm::phys::free_frame(
                                crate::mm::phys::PhysFrame::containing(addr),
                            );
                        }
                    }

                    return Some((pid, status));
                }
            }
        }
        None
    }

    /// Check whether `parent_pid` has a child matching the wait filter.
    pub fn has_children_filtered(&self, parent_pid: Pid, pid_filter: i32, parent_pgid: Pid) -> bool {
        self.tasks
            .iter()
            .flatten()
            .any(|t| Self::child_matches_wait_filter(t, parent_pid, pid_filter, parent_pgid))
    }

    /// Find the first zombie child of `parent_pid`, clean up its resources,
    /// remove it from the task list, and return (child_pid, exit_status).
    pub fn reap_zombie_child(&mut self, parent_pid: Pid) -> Option<(Pid, i32)> {
        self.reap_zombie_child_filtered(parent_pid, -1, 0)
    }

    /// Check whether `parent_pid` has ANY child task (any state).
    pub fn has_children(&self, parent_pid: Pid) -> bool {
        self.has_children_filtered(parent_pid, -1, 0)
    }

    /// Block the current task (caller must re-schedule immediately after).
    pub fn block_current(&mut self) {
        let idx = self.current;
        if let Some(ref mut task) = self.tasks[idx] {
            task.state = TaskState::Blocked;
        }
    }

    /// Get the PID of the current task.
    pub fn current_pid(&self) -> Pid {
        self.tasks[self.current].as_ref().map(|t| t.pid).unwrap_or(0)
    }

    /// Get the physical address of the current task's page table (0 = kernel task).
    pub fn current_page_table_phys(&self) -> u64 {
        self.tasks[self.current].as_ref().map(|t| t.page_table_phys).unwrap_or(0)
    }

    /// Get the kernel stack top of the current task (for TSS RSP0 updates).
    pub fn current_kernel_stack_top(&self) -> u64 {
        self.tasks[self.current].as_ref().map(|t| {
            if t.kernel_stack_base != 0 {
                t.kernel_stack_base + (super::task::KERNEL_STACK_PAGES * crate::mm::phys::FRAME_SIZE) as u64
            } else { 0 }
        }).unwrap_or(0)
    }

    /// Get the process group ID of the current task.
    pub fn current_pgid(&self) -> Pid {
        self.tasks[self.current].as_ref().map(|t| t.pgid).unwrap_or(0)
    }

    /// Get the session ID of the current task.
    pub fn current_session_id(&self) -> Pid {
        self.tasks[self.current].as_ref().map(|t| t.session_id).unwrap_or(0)
    }

    /// Set the process group ID of a task.
    /// Returns true if the target task was found.
    pub fn set_pgid(&mut self, pid: Pid, pgid: Pid) -> bool {
        for slot in self.tasks.iter_mut().flatten() {
            if slot.pid == pid {
                slot.pgid = pgid;
                return true;
            }
        }
        false
    }

    /// Get the process group ID of a task.
    pub fn get_pgid(&self, pid: Pid) -> Option<Pid> {
        self.tasks.iter().flatten().find(|t| t.pid == pid).map(|t| t.pgid)
    }

    /// Collect all PIDs in a given process group.
    pub fn pids_in_group(&self, pgid: Pid) -> alloc::vec::Vec<Pid> {
        self.tasks.iter().flatten()
            .filter(|t| t.pgid == pgid)
            .map(|t| t.pid)
            .collect()
    }

    /// Create a new session: set the current task's session_id and pgid to its own PID.
    /// Returns the new session ID (== current PID).
    pub fn create_session(&mut self) -> Pid {
        let idx = self.current;
        if let Some(ref mut task) = self.tasks[idx] {
            task.session_id = task.pid;
            task.pgid = task.pid;
            task.pid
        } else { 0 }
    }

    /// Send a signal to all tasks in a process group.
    pub fn send_signal_to_group(&mut self, pgid: Pid, sig: super::signal::Signal) {
        for slot in self.tasks.iter_mut().flatten() {
            if slot.pgid == pgid {
                slot.signals.send(sig);
                if matches!(slot.state, TaskState::Blocked) {
                    slot.state = TaskState::Ready;
                }
            }
        }
    }

    /// Replace the current task's execution image in-place (for sys_exec).
    /// Preserves: pid, parent_pid, pgid, session_id, fd_table.
    /// Replaces: context, kernel_stack, page_table, name, signals (reset).
    pub fn replace_current_image(&mut self, new_task: &Task) {
        let idx = self.current;
        if let Some(ref mut task) = self.tasks[idx] {
            // Free old page table if it exists
            let old_pt = task.page_table_phys;
            if old_pt != 0 {
                // SAFETY: old_pt was allocated by create_user_page_table and
                // is no longer in use after we load the new CR3.
                unsafe { crate::mm::virt::free_page_table(old_pt, true); }
            }
            // Free old kernel stack (and its guard page) if it exists.
            if task.kernel_stack_base != 0 {
                let alloc_base = task.kernel_stack_base
                    - (super::task::KERNEL_STACK_GUARD_PAGES * crate::mm::phys::FRAME_SIZE) as u64;
                let total_pages = super::task::KERNEL_STACK_PAGES
                    + super::task::KERNEL_STACK_GUARD_PAGES;
                for i in 0..total_pages {
                    let addr = alloc_base + (i * crate::mm::phys::FRAME_SIZE) as u64;
                    let _ = crate::mm::phys::free_frame(crate::mm::phys::PhysFrame::containing(addr));
                }
            }

            // Replace execution state, keep identity and fds
            task.context = new_task.context;
            task.kernel_stack_base = new_task.kernel_stack_base;
            task.page_table_phys = new_task.page_table_phys;
            task.signals = super::signal::SignalState::new();
            task.name = new_task.name;
            task.name_len = new_task.name_len;
        }
    }

    /// Insert a pre-built task (created by fork) into the scheduler.
    pub fn spawn_forked(&mut self, mut task: Task) -> Result<Pid, &'static str> {
        if self.tasks.len() >= MAX_TASKS {
            return Err("Maximum task limit reached");
        }

        let pid = task.pid;
        task.state = TaskState::Ready;

        let mut slot = None;
        for (i, s) in self.tasks.iter().enumerate() {
            if s.is_none() {
                slot = Some(i);
                break;
            }
        }

        match slot {
            Some(idx) => self.tasks[idx] = Some(task),
            None => self.tasks.push(Some(task)),
        }

        crate::serial::serial_println!(
            "[  SCHED  ] Forked process spawned (PID {})",
            pid,
        );

        Ok(pid)
    }
}

/// Global scheduler instance.
/// Protected by disabling interrupts during access.
static mut SCHEDULER: Option<Scheduler> = None;

/// Initialize the global scheduler.
///
/// # Safety
/// Must be called once from kernel_main with interrupts disabled.
pub unsafe fn init() {
    let sched = &mut *core::ptr::addr_of_mut!(SCHEDULER);
    let mut s = Scheduler::new();
    s.init();
    *sched = Some(s);
}

/// Spawn a kernel task.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn spawn(name: &str, entry: fn() -> !) -> Result<Pid, &'static str> {
    let sched = (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().unwrap();
    sched.spawn(name, entry)
}

/// Spawn a user-space process.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn spawn_user(process: UserProcess) -> Result<Pid, &'static str> {
    let sched = (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().unwrap();
    sched.spawn_user(process)
}

/// Called from timer IRQ to drive preemption.
pub fn timer_tick() {
    // SAFETY: Called from IRQ handler with interrupts disabled.
    unsafe {
        if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
            sched.timer_tick();
        }
    }
}

/// Yield the current task.
pub fn yield_now() {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
            sched.yield_current();
        }
        core::arch::asm!("sti", options(nomem, nostack));
    }
}

/// Terminate the current task.
///
/// # Safety
/// Must be called with interrupts disabled or from a context where
/// switching away is safe.
pub unsafe fn exit_current(status: i32) {
    core::arch::asm!("cli", options(nomem, nostack));
    if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
        sched.exit_current(status);
    }
    // Should not reach here
    loop {
        core::arch::asm!("cli; hlt", options(nomem, nostack));
    }
}

/// Get the current PID.
pub fn current_pid() -> Pid {
    unsafe {
        (*core::ptr::addr_of!(SCHEDULER)).as_ref().map(|s| s.current_pid()).unwrap_or(0)
    }
}

/// Get the physical address of the current task's page table (0 = kernel task).
pub fn current_page_table_phys() -> u64 {
    unsafe {
        (*core::ptr::addr_of!(SCHEDULER)).as_ref().map(|s| s.current_page_table_phys()).unwrap_or(0)
    }
}

/// Get the kernel stack top of the current task.
pub fn current_kernel_stack_top() -> u64 {
    unsafe {
        (*core::ptr::addr_of!(SCHEDULER)).as_ref().map(|s| s.current_kernel_stack_top()).unwrap_or(0)
    }
}

/// Reap a zombie child of `parent_pid`. Returns (child_pid, exit_status) or None.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn reap_zombie_child(parent_pid: Pid) -> Option<(Pid, i32)> {
    (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().and_then(|s| s.reap_zombie_child(parent_pid))
}

/// Reap a zombie child matching a waitpid-style filter.
///
/// `pid_filter` semantics:
/// - `-1`: any child
/// - `0`: any child in caller's process group (`parent_pgid`)
/// - `>0`: child with exact PID
/// - `<-1`: any child in process group `-pid_filter`
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn reap_zombie_child_filtered(
    parent_pid: Pid,
    pid_filter: i32,
    parent_pgid: Pid,
) -> Option<(Pid, i32)> {
    (*core::ptr::addr_of_mut!(SCHEDULER))
        .as_mut()
        .and_then(|s| s.reap_zombie_child_filtered(parent_pid, pid_filter, parent_pgid))
}

/// Check if `parent_pid` has any children (any state).
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn has_children(parent_pid: Pid) -> bool {
    (*core::ptr::addr_of!(SCHEDULER)).as_ref().map(|s| s.has_children(parent_pid)).unwrap_or(false)
}

/// Check if `parent_pid` has any child matching a waitpid-style filter.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn has_children_filtered(parent_pid: Pid, pid_filter: i32, parent_pgid: Pid) -> bool {
    (*core::ptr::addr_of!(SCHEDULER))
        .as_ref()
        .map(|s| s.has_children_filtered(parent_pid, pid_filter, parent_pgid))
        .unwrap_or(false)
}

/// Block the current task and reschedule.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn block_and_reschedule() {
    if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
        sched.block_current();
        sched.schedule();
    }
}

/// Take one pending (non-blocked) signal from the current task.
/// Returns None if no signal is pending.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn take_pending_signal() -> Option<super::signal::Signal> {
    (*core::ptr::addr_of_mut!(SCHEDULER))
        .as_mut()
        .and_then(|s| {
            let idx = s.current;
            s.tasks[idx].as_mut().and_then(|t| t.signals.take_pending())
        })
}

/// Deliver a signal to the task with the given PID.
/// Returns true if the target task was found.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn send_signal_to(pid: Pid, sig: super::signal::Signal) -> bool {
    if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
        for slot in sched.tasks.iter_mut().flatten() {
            if slot.pid == pid {
                slot.signals.send(sig);
                // If the task is blocked, wake it so it can handle the signal.
                if matches!(slot.state, TaskState::Blocked) {
                    slot.state = TaskState::Ready;
                }
                return true;
            }
        }
    }
    false
}

/// Deliver a signal to the current task.
/// Used for exception recovery (e.g., SIGSEGV on page fault in user space).
///
/// # Safety
/// Must be called with interrupts disabled.
pub fn current_deliver_signal(sig_num: u8) {
    if let Some(sig) = super::signal::Signal::from_u8(sig_num) {
        unsafe {
            if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
                let idx = sched.current;
                if let Some(ref mut task) = sched.tasks[idx] {
                    task.signals.send(sig);
                }
            }
        }
    }
}

/// Run a closure with a mutable reference to the current task's FdTable.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn with_current_fd_table<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut crate::vfs::file::FdTable) -> R,
{
    (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().and_then(|s| {
        let idx = s.current;
        s.tasks[idx].as_mut().map(|t| f(&mut t.fd_table))
    })
}

/// Get the process group ID of a task. Returns None if not found.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn get_pgid(pid: Pid) -> Option<Pid> {
    (*core::ptr::addr_of!(SCHEDULER)).as_ref().and_then(|s| s.get_pgid(pid))
}

/// Set the process group ID of a task. Returns false if not found.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn set_pgid(pid: Pid, pgid: Pid) -> bool {
    (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().map(|s| s.set_pgid(pid, pgid)).unwrap_or(false)
}

/// Create a new session for the current task. Returns the new session ID.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn create_session() -> Pid {
    (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().map(|s| s.create_session()).unwrap_or(0)
}

/// Get the current task's process group ID.
pub fn current_pgid() -> Pid {
    unsafe {
        (*core::ptr::addr_of!(SCHEDULER)).as_ref().map(|s| s.current_pgid()).unwrap_or(0)
    }
}

/// Send a signal to all tasks in a process group.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn send_signal_to_group(pgid: Pid, sig: super::signal::Signal) {
    if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
        sched.send_signal_to_group(pgid, sig);
    }
}

/// Get all PIDs in a process group.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn pids_in_group(pgid: Pid) -> alloc::vec::Vec<Pid> {
    (*core::ptr::addr_of!(SCHEDULER)).as_ref().map(|s| s.pids_in_group(pgid)).unwrap_or_default()
}

/// Replace the current task's execution image (for sys_exec).
///
/// # Safety
/// Must be called with interrupts disabled. The caller must switch CR3 and
/// update the kernel stack pointer after this function returns.
pub unsafe fn replace_current_task(new_task: super::task::Task) {
    if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
        sched.replace_current_image(&new_task);
    }
}

/// Access the global scheduler instance.
///
/// # Safety
/// Must be called with interrupts disabled to avoid race conditions.
pub unsafe fn get_instance() -> &'static mut Scheduler {
    (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().expect("Scheduler not initialized")
}

/// jump to the new entry point after this returns.
pub unsafe fn replace_current_image(new_task: &super::task::Task) {
    if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
        sched.replace_current_image(new_task);
    }
}

/// Insert a pre-built forked task into the scheduler.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn spawn_forked(task: super::task::Task) -> Result<Pid, &'static str> {
    (*core::ptr::addr_of_mut!(SCHEDULER))
        .as_mut()
        .ok_or("Scheduler not initialized")?
        .spawn_forked(task)
}

/// Get the parent PID of the current task.
pub fn current_parent_pid() -> Pid {
    unsafe {
        (*core::ptr::addr_of!(SCHEDULER))
            .as_ref()
            .and_then(|s| s.tasks[s.current].as_ref().map(|t| t.parent_pid))
            .unwrap_or(0)
    }
}

/// Run a closure with read access to the current task.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn with_current_task<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&super::task::Task) -> R,
{
    (*core::ptr::addr_of!(SCHEDULER)).as_ref().and_then(|s| {
        let idx = s.current;
        s.tasks[idx].as_ref().map(|t| f(t))
    })
}

/// Run a closure with mutable access to the current task.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn with_current_task_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut super::task::Task) -> R,
{
    (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().and_then(|s| {
        let idx = s.current;
        s.tasks[idx].as_mut().map(|t| f(t))
    })
}

/// Run a closure with read access to a task identified by PID.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn with_task_by_pid<F, R>(pid: Pid, f: F) -> Option<R>
where
    F: FnOnce(&super::task::Task) -> R,
{
    (*core::ptr::addr_of!(SCHEDULER)).as_ref().and_then(|s| {
        s.tasks.iter().flatten().find(|t| t.pid == pid).map(|t| f(t))
    })
}

/// Copy the current task's working directory into `buf`.
/// Returns the length of the path.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn get_cwd(buf: &mut [u8]) -> usize {
    (*core::ptr::addr_of!(SCHEDULER)).as_ref().map(|s| {
        let idx = s.current;
        s.tasks[idx].as_ref().map(|t| {
            let len = t.cwd_len.min(buf.len());
            buf[..len].copy_from_slice(&t.cwd[..len]);
            len
        }).unwrap_or(0)
    }).unwrap_or(0)
}

/// Set the current task's working directory.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn set_cwd(path: &[u8]) -> bool {
    (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut().map(|s| {
        let idx = s.current;
        if let Some(ref mut t) = s.tasks[idx] {
            let len = path.len().min(255);
            t.cwd[..len].copy_from_slice(&path[..len]);
            t.cwd_len = len;
            true
        } else {
            false
        }
    }).unwrap_or(false)
}

/// Send a signal to the foreground process (most recently spawned non-idle
/// user process). Called from IRQ context (e.g., serial Ctrl-C).
///
/// In the current simple model without sessions/controlling terminals,
/// we send the signal to the currently running user process. If it's a
/// kernel task (PID < 100), we skip.
pub fn signal_foreground(sig: super::signal::Signal) {
    unsafe {
        if let Some(ref mut sched) = *core::ptr::addr_of_mut!(SCHEDULER) {
            // Find the running user process (or any ready user process)
            // Prefer the currently running task if it's a user process
            let current_pid = sched.tasks[sched.current]
                .as_ref()
                .map(|t| t.pid)
                .unwrap_or(0);

            if current_pid >= 100 {
                // Send to current user process
                if let Some(ref mut task) = sched.tasks[sched.current] {
                    task.signals.send(sig);
                    if matches!(task.state, TaskState::Blocked) {
                        task.state = TaskState::Ready;
                    }
                }
            } else {
                // Current is kernel task — find any running/ready user task
                for slot in sched.tasks.iter_mut().flatten() {
                    if slot.pid >= 100 && matches!(slot.state, TaskState::Running | TaskState::Ready | TaskState::Blocked) {
                        slot.signals.send(sig);
                        if matches!(slot.state, TaskState::Blocked) {
                            slot.state = TaskState::Ready;
                        }
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::task::Task;
    use super::super::context::TaskContext;

    fn create_test_task(pid: Pid, name: &str) -> Task {
        let mut task = Task::new_kernel(name, || loop {}).unwrap();
        task.pid = pid;
        task.state = TaskState::Ready;
        task
    }

    #[test]
    fn test_scheduler_basic() {
        let mut scheduler = Scheduler::new();
        
        // Add idle task (PID 0)
        let idle_task = create_test_task(0, "idle");
        scheduler.tasks.push(Some(idle_task));
        scheduler.current = 0;

        // Add a user task
        let user_task = create_test_task(100, "test");
        scheduler.tasks.push(Some(user_task));

        // Test spawning
        assert_eq!(scheduler.tasks.len(), 2);
        assert!(scheduler.tasks[0].is_some());
        assert!(scheduler.tasks[1].is_some());
    }

    #[test]
    fn test_scheduler_timer_tick() {
        let mut scheduler = Scheduler::new();
        
        // Add idle task
        let idle_task = create_test_task(0, "idle");
        scheduler.tasks.push(Some(idle_task));
        scheduler.current = 0;
        scheduler.ticks_remaining = 5;

        // Tick down
        scheduler.timer_tick();
        assert_eq!(scheduler.ticks_remaining, 4);

        // Tick to zero should trigger schedule (but we can't test context switch easily)
        for _ in 0..4 {
            scheduler.timer_tick();
        }
        assert_eq!(scheduler.ticks_remaining, TIME_QUANTUM); // Reset after schedule
    }

    #[test]
    fn test_scheduler_find_slot() {
        let mut scheduler = Scheduler::new();
        
        // Add some tasks with gaps
        scheduler.tasks.push(Some(create_test_task(0, "idle")));
        scheduler.tasks.push(None); // Gap
        scheduler.tasks.push(Some(create_test_task(101, "test")));

        // Test that we can find a free slot by checking length
        assert_eq!(scheduler.tasks.len(), 3);
        assert!(scheduler.tasks[0].is_some());
        assert!(scheduler.tasks[1].is_none());
        assert!(scheduler.tasks[2].is_some());
    }
}
