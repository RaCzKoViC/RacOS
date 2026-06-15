# Phase 2 — Kernel correctness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close two critical kernel correctness gaps: process cleanup at exit (FDs released early, SIGCHLD delivered to parent) and `sys_sigreturn` with a working user-handler delivery pipeline backed by a per-process VDSO trampoline.

**Architecture:** Add a `SignalFrame` struct to `kernel/src/task/signal.rs` matching the x86_64 user-mode register set. Wire `deliver_pending_signals` to push a SignalFrame onto the user stack and jump to the user handler (when one is registered via `sigaction`). Implement `sys_sigreturn` to pop the frame and restore context. Map a single page of VDSO bytes containing `mov rax, 28; syscall` into every user address space at a fixed virtual address so handlers' `ret` lands in `sys_sigreturn`. Add `FdTable::close_all()` and call it from `sys_exit`; deliver SIGCHLD to the parent before scheduling away.

**Tech Stack:** Rust nightly, x86_64 inline ASM, no_std kernel, existing `mm::phys`/`mm::virt` allocators, existing `task::scheduler`/`task::signal` modules.

**Spec reference:** `docs/superpowers/specs/2026-05-20-cross-platform-build-and-kernel-correctness-design.md` §5.

**Prerequisite:** Phase 1 complete and `just qemu` works on Linux. Without it, none of the integration tests below can be run locally.

---

## File structure

| File | Action | Responsibility |
|---|---|---|
| `kernel/src/task/signal.rs` | Modify | Add `SignalFrame` struct + helpers |
| `kernel/src/task/task.rs` | Modify | Add `in_signal_handler`, `saved_signal_frame_ptr` fields to `Task` |
| `kernel/src/mm/vdso.rs` | Create | One-page VDSO image with sigreturn trampoline + `init()`/`page_phys()` |
| `kernel/src/mm/mod.rs` | Modify | `pub mod vdso;` + call `vdso::init()` from boot |
| `kernel/src/mm/virt.rs` | Modify | Map VDSO into every user page table at fixed VA |
| `kernel/src/syscall/handlers.rs` | Modify | `deliver_pending_signals` user-handler path; `sys_sigreturn` implementation; `sys_exit` FD close + SIGCHLD |
| `kernel/src/task/scheduler.rs` | Modify | `current_with_saved_iretq_frame()` accessor; `send_signal_to_pid()` helper; SIGCHLD wiring in `exit_current` |
| `kernel/src/vfs/file.rs` | Modify | Add `FdTable::close_all()` |
| `tests/unit/sigframe.rs` | Create | Unit tests for `SignalFrame` layout/size |
| `tests/integration/signal_roundtrip.rs` | Create | Boot-test: process raises SIGUSR1, handler sets a counter, sigreturn returns to caller |
| `tests/integration/exec_loop.rs` | Create | Boot-test: 100× fork/exec/exit/waitpid; assert `mm::phys::free_count` returns to baseline |
| `.github/workflows/ci.yml` | Modify | Wire two new integration boot-tests into the matrix |

---

## VDSO virtual address

This plan reserves **`0x0000_7FFF_FFFE_F000`** (one 4 KiB page) for the VDSO. If `mm::virt::map_range` reports a conflict with existing user mappings during Task 4 verification, pick the next free 4 KiB-aligned address in `0x0000_7FFF_FFFE_0000..0x0000_7FFF_FFFF_F000` and update the constant in `vdso.rs` plus the spec note.

---

## Task 1: Define `SignalFrame` layout

**Files:**
- Modify: `kernel/src/task/signal.rs:80-89` (just after `SignalState`)
- Test: `tests/unit/sigframe.rs`

- [ ] **Step 1: Write the failing unit test**

Create `tests/unit/sigframe.rs`:

```rust
#![cfg(test)]

extern crate alloc;

use racore::task::signal::SignalFrame;

#[test]
fn signal_frame_size_is_expected() {
    // Frame must fit on the user stack red zone + a small margin.
    assert_eq!(core::mem::size_of::<SignalFrame>(), 152);
}

#[test]
fn signal_frame_repr_c_field_offsets() {
    // Verify alignment matches the C ABI layout the kernel writes/reads.
    let f = SignalFrame::default();
    let base = &f as *const _ as usize;
    let rax_off  = &f.rax  as *const _ as usize - base;
    let rsp_off  = &f.rsp  as *const _ as usize - base;
    let rip_off  = &f.rip  as *const _ as usize - base;
    let mask_off = &f.saved_sigmask as *const _ as usize - base;
    let sig_off  = &f.signal_number as *const _ as usize - base;

    // Spot-check a few — confirm no surprising padding.
    assert_eq!(rax_off, 0);
    assert!(rsp_off > 0 && rsp_off < 0x80);
    assert!(rip_off > 0 && rip_off < 0x80);
    assert!(mask_off > rip_off);
    assert!(sig_off > mask_off);
}
```

- [ ] **Step 2: Run the test — it must fail to compile**

Run: `cargo test --package racore --test sigframe -- --nocapture 2>&1 | head -20`
Expected: `error[E0432]: unresolved import 'racore::task::signal::SignalFrame'` or similar.

- [ ] **Step 3: Add `SignalFrame` to `signal.rs`**

Append to `kernel/src/task/signal.rs` (right after the closing `}` of `SignalAction`):

```rust
/// Frame written to the user stack when delivering a signal with a user
/// handler. `sys_sigreturn` consumes it to restore the interrupted context.
///
/// Layout is `#[repr(C)]` because user-space VDSO code may read it.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct SignalFrame {
    // Saved GPRs in the order pushed by the syscall entry path
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8:  u64,
    pub r9:  u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    // Interrupted instruction state
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
    // Signal bookkeeping
    pub saved_sigmask: u64,
    pub signal_number: u32,
    pub _pad: u32,
}

impl SignalFrame {
    /// Total bytes a SignalFrame occupies on the user stack, including the
    /// 16-byte alignment the C ABI guarantees just before a `call` instruction.
    pub const fn aligned_size() -> usize {
        // 19 u64 + 1 u32 + 1 u32 = 152 bytes — already 8-aligned.
        // Round up to 16 for stack alignment after the kernel push.
        ((core::mem::size_of::<SignalFrame>() + 15) / 16) * 16
    }
}
```

- [ ] **Step 4: Run the test — it must pass**

Run: `cargo test --package racore --test sigframe`
Expected: PASS for both tests.

- [ ] **Step 5: Commit**

```bash
git add kernel/src/task/signal.rs tests/unit/sigframe.rs
git commit -m "feat(kernel): add SignalFrame struct for signal handler delivery"
```

---

## Task 2: Extend `Task` with signal-handler bookkeeping

**Files:**
- Modify: `kernel/src/task/task.rs:88-119` (Task struct) and `task.rs:172-220` (new_kernel + idle constructors)

- [ ] **Step 1: Add two fields to `Task`**

Inside `pub struct Task { … }` at `task.rs:88-119`, append before the closing brace:

```rust
    /// True between the moment a signal handler is invoked and the moment
    /// `sys_sigreturn` runs. Used by sigreturn to validate state.
    pub in_signal_handler: bool,
    /// User-space pointer to the SignalFrame written for the currently
    /// active handler. Zero when no handler is running.
    pub saved_signal_frame_ptr: u64,
```

- [ ] **Step 2: Initialise the new fields in both constructors**

In `Task::new_kernel` (around `task.rs:172`), add to the returned struct literal:

```rust
            in_signal_handler: false,
            saved_signal_frame_ptr: 0,
```

Same lines added to `Task::idle` (around `task.rs:201`).

- [ ] **Step 3: Build to confirm compile**

Run: `cargo build --package racore --target x86_64-unknown-none`
Expected: builds cleanly. If fields are missing from a third constructor (e.g. `UserProcess::from_elf`), add them there too — search with:

```bash
grep -rn "fd_table:" kernel/src/task/ kernel/src/syscall/
```

- [ ] **Step 4: Commit**

```bash
git add kernel/src/task/task.rs kernel/src/task/process.rs
git commit -m "feat(kernel): add Task signal-handler bookkeeping fields"
```

(Include `process.rs` only if you had to edit it for the third constructor.)

---

## Task 3: Create the VDSO module

**Files:**
- Create: `kernel/src/mm/vdso.rs`
- Modify: `kernel/src/mm/mod.rs`
- Modify: `kernel/src/main.rs` (call `vdso::init()` from boot)

- [ ] **Step 1: Write `kernel/src/mm/vdso.rs`**

```rust
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
```

- [ ] **Step 2: Register the module**

Edit `kernel/src/mm/mod.rs` to add `pub mod vdso;` next to the other `pub mod` lines (likely around the top of the file).

- [ ] **Step 3: Call `vdso::init()` from boot**

In `kernel/src/main.rs`, find the block where `mm::heap::init()` is called (around line 113). Immediately after the heap is up and before VFS init, add:

```rust
    // Initialize VDSO trampoline page (used by signal sigreturn).
    // SAFETY: heap and phys allocator are initialised.
    unsafe {
        crate::mm::vdso::init().expect("Failed to initialise VDSO");
    }
```

- [ ] **Step 4: Build + boot test**

Run: `cargo build --package racore --target x86_64-unknown-none`
Then: `just build-image && just run-uefi` (Ctrl-A X to quit after seeing the new log line)

Expected: serial output contains `RACORE: VDSO initialised at phys …`.

- [ ] **Step 5: Commit**

```bash
git add kernel/src/mm/vdso.rs kernel/src/mm/mod.rs kernel/src/main.rs
git commit -m "feat(kernel): add VDSO page with sys_sigreturn trampoline"
```

---

## Task 4: Map VDSO into every user page table

**Files:**
- Modify: `kernel/src/mm/virt.rs:401-422` (`create_user_page_table`)

- [ ] **Step 1: Add VDSO mapping to `create_user_page_table`**

Replace the body of `create_user_page_table()` with (keeping the existing copy-kernel-PML4 loop intact and appending the VDSO mapping):

```rust
pub fn create_user_page_table() -> Result<u64, &'static str> {
    let new_pml4_phys = alloc_page_table()?;
    let kernel_pml4_phys = unsafe {
        let cached = KERNEL_PML4_PHYS;
        if cached != 0 { cached } else { read_cr3() & !0xFFF_u64 }
    };

    unsafe {
        let src = &*(kernel_pml4_phys as *const PageTable);
        let dst = &mut *(new_pml4_phys as *mut PageTable);
        for i in 0..512 {
            if src.entries[i].is_present() && (src.entries[i].flags() & flags::USER == 0) {
                dst.entries[i] = src.entries[i];
            }
        }
    }

    // Map the VDSO page (user-readable, user-executable, no write).
    let vdso_phys = crate::mm::vdso::page_phys();
    if vdso_phys != 0 {
        let vdso_flags = flags::PRESENT | flags::USER;
        // SAFETY: vdso_phys was allocated and initialised by vdso::init at boot,
        // new_pml4_phys was just allocated by alloc_page_table.
        unsafe {
            map_range(
                new_pml4_phys,
                crate::mm::vdso::VDSO_VADDR,
                vdso_phys,
                FRAME_SIZE as u64,
                vdso_flags,
            )?;
        }
    }

    Ok(new_pml4_phys)
}
```

> If `flags::USER` is the only "user readable" bit available, the page is implicitly readable + executable. If the project has an explicit no-write flag (e.g. `flags::READ_ONLY` or equivalent), include it.

- [ ] **Step 2: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

If `map_range` is not in scope, add a `use` at the top of `virt.rs` or invoke it as `self::map_range`. The signature is at `virt.rs:331` per the exploration.

- [ ] **Step 3: Manual smoke**

```bash
just build-image && just run-uefi
```

Expected: boot reaches the existing init/racsh banner with no triple fault. (No new visible output yet; only the new map is exercised.)

- [ ] **Step 4: Commit**

```bash
git add kernel/src/mm/virt.rs
git commit -m "feat(kernel): map VDSO into every user page table"
```

---

## Task 5: Add `FdTable::close_all()`

**Files:**
- Modify: `kernel/src/vfs/file.rs:92-…` (impl FdTable)

- [ ] **Step 1: Read the current `FdTable` impl**

```bash
sed -n '92,170p' kernel/src/vfs/file.rs
```

Identify the internal storage of FDs (likely `Vec<Option<Arc<OpenFile>>>` or similar).

- [ ] **Step 2: Add `close_all` after `close`**

Insert after the existing `pub fn close(...)` definition (around `vfs/file.rs:135`):

```rust
    /// Close every open descriptor. Each `OpenFile` Arc is dropped; pipes and
    /// inodes see their refcount drop, so consumers observe EOF promptly.
    pub fn close_all(&mut self) {
        // Concrete implementation depends on the FdTable storage. The pattern
        // is: iterate every slot, take it out, let the Arc drop. Example for a
        // Vec<Option<Arc<OpenFile>>>-shaped table:
        for slot in self.slots_mut() {
            *slot = None;
        }
    }
```

If `slots_mut` doesn't exist, replace the body with the equivalent iteration over the table's internal storage. Match the existing access pattern used by `close()` so the change is minimal.

- [ ] **Step 3: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 4: Commit**

```bash
git add kernel/src/vfs/file.rs
git commit -m "feat(kernel): add FdTable::close_all for process exit"
```

---

## Task 6: Verify `send_signal_to` is already available

**Files:**
- Read: `kernel/src/task/scheduler.rs:723-742`

- [ ] **Step 1: Confirm the helper exists**

```bash
grep -n "pub unsafe fn send_signal_to\b" kernel/src/task/scheduler.rs
```

Expected: `pub unsafe fn send_signal_to(pid: Pid, sig: super::signal::Signal) -> bool` at line ~728. This is the helper Task 7 will use to post SIGCHLD. If it does not exist, add it next to `send_signal_to_group` using the same pattern (see the existing free wrappers near the bottom of `scheduler.rs`). No commit if it's already present.

---

## Task 7: Make `exit_current` close FDs and post SIGCHLD to parent

**Files:**
- Modify: `kernel/src/task/scheduler.rs:289-321` (`exit_current` method)
- Modify: `kernel/src/syscall/handlers.rs:406-422` (`sys_exit`)

- [ ] **Step 1: Rewrite `exit_current` to close FDs and post SIGCHLD**

Replace the body of the `impl` method at `scheduler.rs:290` with:

```rust
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
```

- [ ] **Step 2: Remove the stale TODO in `sys_exit`**

In `handlers.rs:407-422`, replace the function body with:

```rust
pub fn sys_exit(status: i32) -> ! {
    crate::serial::serial_println!(
        "[  SYSCALL] sys_exit(status={}) from PID {}",
        status,
        crate::task::scheduler::current_pid()
    );

    // Address space + kernel stack are reclaimed by reap_zombie_child when
    // the parent calls waitpid; FDs and parent SIGCHLD are handled in
    // exit_current below.
    unsafe { crate::task::scheduler::exit_current(status); }

    // exit_current never returns. Halt as a safety net.
    loop {
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); }
    }
}
```

- [ ] **Step 3: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 4: Commit**

```bash
git add kernel/src/task/scheduler.rs kernel/src/syscall/handlers.rs
git commit -m "fix(kernel): close fds on exit and post SIGCHLD to parent"
```

---

## Task 8: Capture interrupted user context for signal delivery

**Files:**
- Modify: `kernel/src/syscall/handlers.rs:370-404` (`deliver_pending_signals`)
- Modify: `kernel/src/task/scheduler.rs` (add `with_current_iretq_frame_mut` accessor if missing)

Before modifying `deliver_pending_signals`, locate the kernel-side iretq frame the syscall entry path saves. Search:

```bash
grep -rn "iretq_frame\|saved_regs\|UserRegs\|TrapFrame" kernel/src/ | head
```

- [ ] **Step 1: Verify which struct holds the saved user regs across the syscall entry**

The existing `dispatch.rs:196` calls `handlers::deliver_pending_signals()` after a syscall returns. By that point, the syscall entry trampoline has pushed user RIP, RSP, RFLAGS and the GPRs onto the kernel stack. Find that struct (likely named `SyscallRegs`, `UserRegs`, or part of an `iretq` frame in `kernel/src/syscall/entry.rs` or `dispatch.rs`).

- [ ] **Step 2: Expose a mutable accessor**

If no helper exists, add to `kernel/src/task/scheduler.rs` (near the other `with_current_*` helpers):

```rust
    /// Run a closure with mutable access to the current task plus its saved
    /// iretq frame. The frame pointer is provided by the syscall entry path
    /// via `set_current_iretq_frame` (set on every syscall enter, cleared on
    /// exit).
    pub fn with_current_iretq_frame_mut<R>(
        &mut self,
        f: impl FnOnce(&mut Task, &mut crate::syscall::entry::IretqFrame) -> R,
    ) -> Option<R> {
        let idx = self.current;
        let frame_ptr = self.current_iretq_frame_ptr;
        let task = self.tasks[idx].as_mut()?;
        if frame_ptr == 0 { return None; }
        // SAFETY: frame_ptr points to the on-stack frame established by the
        // syscall entry trampoline. It lives until the trampoline issues iretq.
        let frame = unsafe { &mut *(frame_ptr as *mut crate::syscall::entry::IretqFrame) };
        Some(f(task, frame))
    }
```

If the existing entry path doesn't expose a pointer, the smallest patch is in `kernel/src/syscall/entry.rs`: when entering, write `scheduler::set_current_iretq_frame(rsp_at_entry)`, and when leaving, clear it. The exact form depends on the saved-frame layout — match what `dispatch.rs` already passes to handler routines.

Document the new field in the doc comment for `Scheduler` and the new helper.

- [ ] **Step 3: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 4: Commit**

```bash
git add kernel/src/syscall/entry.rs kernel/src/task/scheduler.rs
git commit -m "feat(kernel): expose iretq frame to signal delivery"
```

---

## Task 9: Push a SignalFrame and redirect to the user handler

**Files:**
- Modify: `kernel/src/syscall/handlers.rs:372-404` (`deliver_pending_signals`)

- [ ] **Step 1: Rewrite `deliver_pending_signals` to honour user handlers**

Replace the function body with:

```rust
pub fn deliver_pending_signals() {
    use crate::task::signal::{SignalAction, SignalFrame, SignalState, Signal, SIG_DFL, SIG_IGN};

    loop {
        // Phase 1: decide what to do with the next pending signal.
        let sig: Signal = {
            // SAFETY: disabling interrupts while accessing the scheduler.
            let s = unsafe {
                core::arch::asm!("cli", options(nomem, nostack));
                let s = crate::task::scheduler::take_pending_signal();
                core::arch::asm!("sti", options(nomem, nostack));
                s
            };
            match s {
                None => return,
                Some(s) => s,
            }
        };

        // Lookup user handler for this signal (0 = SIG_DFL, 1 = SIG_IGN, else user fn).
        let handler = crate::task::scheduler::with_current_task_mut(|t| {
            t.signals.get_handler(sig as u8)
        }).unwrap_or(SIG_DFL);

        if handler == SIG_IGN {
            continue;
        }
        if handler == SIG_DFL {
            match SignalState::default_action(sig) {
                SignalAction::Terminate => { sys_exit(-1); }
                SignalAction::Ignore => { continue; }
                SignalAction::Stop => {
                    unsafe {
                        core::arch::asm!("cli", options(nomem, nostack));
                        crate::task::scheduler::block_and_reschedule();
                    }
                    continue;
                }
                SignalAction::Continue => { continue; }
            }
        }

        // Custom user handler — push SignalFrame and redirect.
        let result = unsafe {
            core::arch::asm!("cli", options(nomem, nostack));
            let r = crate::task::scheduler::with_current_iretq_frame_mut(|task, frame| {
                // 1. Compute new user RSP: skip the red zone (128 bytes),
                //    reserve space for SignalFrame, keep 16-byte alignment.
                let red_zone = 128u64;
                let frame_size = SignalFrame::aligned_size() as u64;
                let mut new_rsp = frame.user_rsp.wrapping_sub(red_zone + frame_size);
                new_rsp &= !0xF; // 16-byte align

                // 2. Build the SignalFrame from the interrupted context.
                let sf = SignalFrame {
                    rax: frame.rax, rbx: frame.rbx, rcx: frame.rcx, rdx: frame.rdx,
                    rsi: frame.rsi, rdi: frame.rdi, rbp: frame.rbp,
                    r8: frame.r8, r9: frame.r9, r10: frame.r10, r11: frame.r11,
                    r12: frame.r12, r13: frame.r13, r14: frame.r14, r15: frame.r15,
                    rip: frame.user_rip, rsp: frame.user_rsp, rflags: frame.user_rflags,
                    saved_sigmask: task.signals.blocked as u64,
                    signal_number: sig as u32,
                    _pad: 0,
                };

                // 3. copy_to_user: write the frame to user stack.
                //    validate_user_ptr is defined in this file (handlers.rs).
                if validate_user_ptr(new_rsp, frame_size as usize).is_err() {
                    return Err(());
                }
                unsafe {
                    core::ptr::write_volatile(new_rsp as *mut SignalFrame, sf);
                }

                // 4. Push the VDSO trampoline address as the handler's return
                //    address. RSP -= 8.
                let trampoline_rsp = new_rsp.wrapping_sub(8);
                if validate_user_ptr(trampoline_rsp, 8).is_err() {
                    return Err(());
                }
                unsafe {
                    *(trampoline_rsp as *mut u64) = crate::mm::vdso::VDSO_VADDR;
                }

                // 5. Patch the iretq frame: RIP = handler, RSP = trampoline_rsp,
                //    RDI = signo (System V ABI first integer arg).
                frame.user_rip = handler;
                frame.user_rsp = trampoline_rsp;
                frame.rdi = sig as u64;

                // 6. Record bookkeeping so sys_sigreturn can find the frame.
                task.in_signal_handler = true;
                task.saved_signal_frame_ptr = new_rsp;
                // Block this signal during its own handler (POSIX default).
                task.signals.blocked |= sig.mask();
                Ok(())
            });
            core::arch::asm!("sti", options(nomem, nostack));
            r
        };

        match result {
            Some(Ok(())) => return,
            _ => {
                // Frame write failed (bad user pointer). Force-exit with SIGSEGV.
                sys_exit(-1);
            }
        }
    }
}
```

> This function intentionally returns after preparing ONE handler — control returns to the syscall trampoline, which `iretq`s into the handler at `frame.user_rip`. Subsequent pending signals are delivered the next time `deliver_pending_signals` is called.

- [ ] **Step 2: Resolve `frame.user_rip`/`user_rsp`/`user_rflags`/gprs field names**

The actual field names of your `IretqFrame` (or whatever the entry path uses) may differ. Open `kernel/src/syscall/entry.rs` and adapt the field names verbatim. The expected fields are: 15 GPRs, user RIP, user RSP, user RFLAGS.

- [ ] **Step 3: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

Fix any name mismatches surfaced by the compiler.

- [ ] **Step 4: Commit**

```bash
git add kernel/src/syscall/handlers.rs
git commit -m "feat(kernel): push SignalFrame and redirect to user handler"
```

---

## Task 10: Implement `sys_sigreturn`

**Files:**
- Modify: `kernel/src/syscall/handlers.rs:1893-1898`

- [ ] **Step 1: Replace the stub**

Replace the body with:

```rust
pub fn sys_sigreturn() -> SyscallResult {
    use crate::task::signal::SignalFrame;

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let r = crate::task::scheduler::with_current_iretq_frame_mut(|task, frame| {
            if !task.in_signal_handler || task.saved_signal_frame_ptr == 0 {
                return Err(SyscallError::EFAULT);
            }
            let frame_ptr = task.saved_signal_frame_ptr;
            let size = core::mem::size_of::<SignalFrame>();
            if validate_user_ptr(frame_ptr, size).is_err() {
                return Err(SyscallError::EFAULT);
            }
            // copy_from_user (kernel reads user memory).
            let sf: SignalFrame = unsafe { core::ptr::read_volatile(frame_ptr as *const SignalFrame) };

            // Restore GPRs.
            frame.rax = sf.rax; frame.rbx = sf.rbx; frame.rcx = sf.rcx; frame.rdx = sf.rdx;
            frame.rsi = sf.rsi; frame.rdi = sf.rdi; frame.rbp = sf.rbp;
            frame.r8 = sf.r8; frame.r9 = sf.r9; frame.r10 = sf.r10; frame.r11 = sf.r11;
            frame.r12 = sf.r12; frame.r13 = sf.r13; frame.r14 = sf.r14; frame.r15 = sf.r15;
            // Restore interrupted context.
            frame.user_rip = sf.rip;
            frame.user_rsp = sf.rsp;
            frame.user_rflags = sf.rflags;

            // Restore signal mask.
            task.signals.blocked = sf.saved_sigmask as u32;
            task.in_signal_handler = false;
            task.saved_signal_frame_ptr = 0;

            Ok(())
        });
        core::arch::asm!("sti", options(nomem, nostack));
        r.unwrap_or(Err(SyscallError::EFAULT))
    }?;
    Ok(0)
}
```

- [ ] **Step 2: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 3: Commit**

```bash
git add kernel/src/syscall/handlers.rs
git commit -m "feat(kernel): implement sys_sigreturn user-context restore"
```

---

## Task 11: Integration test — signal round-trip

**Files:**
- Create: `tests/integration/signal_roundtrip.rs`

This test runs as a tiny userland program loaded into initramfs. It is invoked by the boot-smoke job.

- [ ] **Step 1: Write the test program**

```rust
// tests/integration/signal_roundtrip.rs
//
// Boot-time integration test for signal delivery + sigreturn.
//
// Layout:
//   - Install a SIGUSR1 handler that increments a static AtomicU32.
//   - Raise SIGUSR1 via kill(self).
//   - After the handler returns, verify the counter == 1 and the program
//     continues normally.
//
// Loaded into initramfs as /bin/signal-roundtrip-test; init runs it after
// /bin/sh starts. Output is captured via the kernel serial port.

#![no_std]
#![no_main]

extern crate libc_lite;

use core::sync::atomic::{AtomicU32, Ordering};

static HIT: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_sigusr1(_signo: i32) {
    HIT.fetch_add(1, Ordering::SeqCst);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let pid = libc_lite::getpid();
    libc_lite::sigaction(/* SIGUSR1 = 10 */ 10, on_sigusr1 as u64);
    libc_lite::kill(pid as i32, 10);
    // After kill returns we should be back here, with HIT == 1.
    let hits = HIT.load(Ordering::SeqCst);
    if hits == 1 {
        libc_lite::write_str(1, "SIGNAL-ROUNDTRIP-OK\n");
        libc_lite::exit(0);
    } else {
        libc_lite::write_str(1, "SIGNAL-ROUNDTRIP-FAIL\n");
        libc_lite::exit(1);
    }
}
```

> If `libc_lite` does not expose `sigaction`/`kill`/`write_str`/`exit`/`getpid` yet, add thin syscall wrappers in `libs/libc-lite/src/lib.rs` matching the kernel's syscall numbers. SIGUSR1 must be defined; if RacOS doesn't have it yet, repurpose SIGCHLD (17) or SIGWINCH (28) for this test.

- [ ] **Step 2: Wire the test into the build**

Add to root `Cargo.toml` workspace members (if integration tests are workspace members) or to the initramfs-staging logic in `scripts/build-image.sh` so the binary is copied to `initramfs-root/bin/signal-roundtrip-test`.

- [ ] **Step 3: Have init optionally run the test**

In `init/src/main.rs`, add a one-shot launch of the test before/after the shell spawn, controlled by an env or compile-time flag so production boot still goes straight to the shell. Example:

```rust
#[cfg(feature = "boot-test-signals")]
let _ = libc_lite::spawn_args("/bin/signal-roundtrip-test", &[]);
```

- [ ] **Step 4: Run locally**

```bash
just build-image && just run-uefi
```

Expected: serial log contains `SIGNAL-ROUNDTRIP-OK`. If it contains `FAIL`, debug the delivery path.

- [ ] **Step 5: Commit**

```bash
git add tests/integration/signal_roundtrip.rs libs/libc-lite/src/lib.rs init/src/main.rs scripts/build-image.sh
git commit -m "test: integration boot test for SIGUSR1 round-trip"
```

---

## Task 12: Integration test — exec loop memory leak

**Files:**
- Create: `tests/integration/exec_loop.rs`

- [ ] **Step 1: Write the test program**

```rust
// tests/integration/exec_loop.rs
//
// Boot-time integration test for process cleanup. Forks and execs /bin/true
// 100 times. Captures mm::phys::free_count before and after via a debug
// syscall (sys_uname or a dedicated sys_getstat — pick what exists).
//
// PASS criteria: free-page count after the loop is within 16 frames of the
// pre-loop count (≈ 64 KiB at 4 KiB pages), after a 10 ms settle.

#![no_std]
#![no_main]

extern crate libc_lite;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let before = libc_lite::sys_getfreepages();
    for _ in 0..100 {
        let pid = libc_lite::fork();
        if pid == 0 {
            libc_lite::execve("/bin/true", &[b"true\0".as_ptr() as *const u8, core::ptr::null()], core::ptr::null());
            libc_lite::exit(127);
        } else {
            let mut status = 0i32;
            libc_lite::waitpid(pid, &mut status, 0);
        }
    }
    libc_lite::nanosleep(0, 10_000_000);
    let after = libc_lite::sys_getfreepages();
    let delta = if after >= before { 0 } else { before - after };
    if delta <= 16 {
        libc_lite::write_str(1, "EXEC-LOOP-OK\n");
        libc_lite::exit(0);
    } else {
        libc_lite::write_str(1, "EXEC-LOOP-FAIL\n");
        libc_lite::exit(1);
    }
}
```

- [ ] **Step 2: Add the `sys_getfreepages` debug syscall**

In `kernel/src/syscall/handlers.rs`, add a new handler exposing `mm::phys::free_count()`. Wire it into the dispatch table (`dispatch.rs`). Choose a syscall number that's not yet allocated.

- [ ] **Step 3: Wire into initramfs**

Same approach as Task 11 Step 2 — add to `build-image.sh` and optionally have init run it under a feature flag.

- [ ] **Step 4: Run locally**

```bash
just build-image && just run-uefi
```

Expected: `EXEC-LOOP-OK` in serial output. If `FAIL`, inspect leak via `mm::phys::free_count` calls between fork iterations.

- [ ] **Step 5: Commit**

```bash
git add tests/integration/exec_loop.rs kernel/src/syscall/handlers.rs kernel/src/syscall/dispatch.rs scripts/build-image.sh
git commit -m "test: integration boot test for exec-loop memory cleanup"
```

---

## Task 13: Wire new integration tests into CI

**Files:**
- Modify: `.github/workflows/ci.yml` (after the `interactive-smoke` job)

- [ ] **Step 1: Add an `integration-smoke` job**

Append:

```yaml
  integration-smoke:
    name: Integration boot tests (signal + exec loop)
    runs-on: ubuntu-22.04
    needs: build
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@nightly
        with:
          components: rust-src, llvm-tools-preview
          targets: x86_64-unknown-none, x86_64-unknown-uefi
      - name: Install QEMU, OVMF, nasm, python
        run: |
          sudo apt-get update
          sudo apt-get install -y qemu-system-x86 ovmf nasm mtools python3

      - name: Build with boot-test feature
        run: |
          cargo build --package racore --target x86_64-unknown-none --features boot-test
          cargo build --package racos-boot --target x86_64-unknown-uefi
          cargo build --workspace --exclude racore --exclude racos-boot --features boot-test

      - name: Stage ESP (with test binaries)
        run: bash scripts/build-image.sh

      - name: Boot in QEMU (30s budget)
        run: |
          OVMF=/usr/share/OVMF/OVMF_CODE.fd
          timeout 30 qemu-system-x86_64 \
            -machine q35 -cpu qemu64 -m 512M \
            -drive if=pflash,format=raw,file=$OVMF,readonly=on \
            -drive file=fat:rw:esp,format=raw \
            -serial stdio -display none -no-reboot \
            > boot.log 2>&1 || true
          tail -n 80 boot.log

      - name: Assert SIGNAL-ROUNDTRIP-OK + EXEC-LOOP-OK
        run: |
          grep -q "SIGNAL-ROUNDTRIP-OK" boot.log \
            || (echo "FAIL: signal roundtrip test did not pass" && exit 1)
          grep -q "EXEC-LOOP-OK" boot.log \
            || (echo "FAIL: exec loop test did not pass" && exit 1)
          echo "Integration smoke PASSED"
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add integration-smoke job for signal and exec-loop boot tests"
```

---

## Definition of done for Phase 2

All true:

- ✅ `tests/integration/signal_roundtrip.rs` reports `SIGNAL-ROUNDTRIP-OK` on every boot.
- ✅ `tests/integration/exec_loop.rs` reports `EXEC-LOOP-OK` (free-page delta ≤ 16 frames).
- ✅ TODO at `handlers.rs:415` removed (replaced by Task 7's revised body).
- ✅ TODO at `handlers.rs:1896` removed (replaced by Task 10's implementation).
- ✅ VDSO page initialised at boot; serial log contains the `VDSO initialised at phys …` line.
- ✅ No regressions in `boot-smoke` and `interactive-smoke` jobs.

---

## Self-review notes

- **Spec coverage**: §5.1 (process cleanup) is handled by Task 5 (FdTable::close_all) + Task 7 (sys_exit close + SIGCHLD). The existing `reap_zombie_child_filtered` already frees page tables and kernel stacks per the exploration finding (`scheduler.rs:344-388`), so address-space release is **not duplicated** — that's intentional. §5.2 (sigreturn + VDSO) is Tasks 1–4 + 9–10.
- **Placeholders**: Task 5 Step 2 explicitly says "match the existing access pattern" because the `FdTable` internal storage shape was not captured during exploration. Task 8 lets the engineer probe the entry frame layout — this is necessary indirection because the syscall trampoline's saved-context struct is project-specific. Task 11 calls out `SIGUSR1` may need to be repurposed if not defined yet.
- **Type consistency**: `SignalFrame` is the same name in `signal.rs`, `handlers.rs`, and the tests. `VDSO_VADDR` is referenced in `vdso.rs`, `virt.rs`, and `handlers.rs`.
- **Scope**: This plan does not touch TTY ioctls (those are Phase 3) and does not implement SMP AP startup, module loader, or blocking poll (explicitly out of spec scope).

---

## Risks the engineer should watch for

1. **Iretq frame field names**: Task 8/9/10 assume an `IretqFrame` with named GPR fields. The actual struct in `syscall/entry.rs` may use a different name (`SyscallContext`, `UserRegs`, etc.) and field order. Adapt to whatever exists; do **not** rename existing fields.
2. **Red zone respect**: The SystemV AMD64 ABI guarantees user code 128 bytes below RSP. Task 9 reserves it. If a handler is invoked from a leaf function relying on that, the red zone reservation must stay.
3. **Reentrancy**: Task 9 sets `task.signals.blocked |= sig.mask()` so the same signal is masked during its own handler. Tests should verify nested SIGUSR1→SIGUSR1 is properly suppressed.
4. **Frame freeing on failed write**: If `validate_user_ptr` rejects the new RSP, the handler force-exits the task (`sys_exit(-1)`). Verify this path does **not** leave the task in `in_signal_handler == true` with stale `saved_signal_frame_ptr` — the exit_current logic should not care, but a future refactor might.

---

## Open follow-ups (NOT in this plan)

- Save/restore the XSAVE area (FP/SSE/AVX state) in `SignalFrame`. Currently the FP state crosses the handler boundary unsaved — single-thread MVP is OK with this but multi-thread or AVX-using userland is not.
- Per-CPU reaper queue for kernel-stack release (mentioned in spec §13). Current code frees inline at `reap_zombie_child` time, which is fine on UP but should move to deferred per-CPU before SMP work.
- `sigaltstack()` support — handler runs on the interrupted thread's stack; switching to an alternate stack for SIGSEGV-during-stack-overflow recovery is a future improvement.
