// RaCore — User Threads (POSIX-style)
//
// Extends the task model to support shared address spaces and user-space contexts.

extern crate alloc;

use crate::mm::phys::{self, FRAME_SIZE};
use crate::task::context::TaskContext;
use crate::task::signal::SignalState;
use crate::task::task::{Credentials, Pid, Task, TaskState};
use crate::vfs::file::FdTable;
use alloc::sync::Arc;

pub struct UserThread {
    pub tid: Pid,
    pub pid: Pid, // Parent process ID
    pub state: TaskState,
    pub stack_addr: u64,
    pub entry_point: u64,
}

impl UserThread {
    /// Create a new thread for an existing process.
    pub fn new(pid: Pid, entry: u64, stack_addr: u64) -> Self {
        // TIDs and PIDs share the same pool in simple model
        UserThread {
            tid: crate::task::task::alloc_pid_internal(),
            pid,
            state: TaskState::Created,
            stack_addr,
            entry_point: entry,
        }
    }
}

/// Helper to allow task.rs to expose its PID allocator if needed.
pub fn next_id() -> Pid {
    crate::task::task::alloc_pid_internal()
}
