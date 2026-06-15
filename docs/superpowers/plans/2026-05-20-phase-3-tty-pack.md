# Phase 3 — TTY pack Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `TIOCGWINSZ`/`TIOCSWINSZ`/`TIOCGPGRP`/`TIOCSPGRP` operate on the TTY backing the file descriptor (currently they ignore the fd and use the caller's pgid). Make `Tty::set_winsize` deliver `SIGWINCH` to the TTY's foreground process group, matching the behaviour `PtyMaster::set_winsize` already implements.

**Architecture:** Add a thin VFS-side resolver that turns an `fd` into a mutable reference to the underlying `PtyMaster` (or `Tty`) when the file is a TTY device. Replace the placeholder bodies in the four ioctl arms with code that drives the resolver. Mirror the existing `PtyMaster::set_winsize` SIGWINCH path inside `Tty::set_winsize` so the kernel console TTY behaves the same as a PTY.

**Tech Stack:** Rust, no_std kernel; existing `kernel/src/vfs/devfs.rs` device routing; `kernel/src/tty/{tty.rs, pty.rs}`; `task::scheduler::send_signal_to_group` (already in tree).

**Spec reference:** `docs/superpowers/specs/2026-05-20-cross-platform-build-and-kernel-correctness-design.md` §6.

**Prerequisites:**
- Phase 1 complete (need `just qemu` on Linux to run the integration test).
- Phase 2 complete (without sigreturn the integration test's handler can't return cleanly).

**Note on existing code (from exploration):**
- `Tty` already has `winsize`, `session_id`, `foreground_pgid` fields (`tty.rs:12-25`).
- `Tty::set_winsize` exists but contains a TODO and does **not** deliver SIGWINCH (`tty.rs:39-42`).
- `PtyMaster::set_winsize` **already** delivers SIGWINCH to `foreground_pgid` (`pty.rs:116-129`).
- The four ioctl handlers exist as placeholders at `handlers.rs:1236-1302` that act on the caller's pgid, not the TTY-bound fg group.

---

## File structure

| File | Action | Responsibility |
|---|---|---|
| `kernel/src/vfs/devfs.rs` | Modify | Expose a `with_tty_at_fd` helper for ioctl handlers (or add `InodeOps::as_pty_master_mut` / `as_tty_mut`) |
| `kernel/src/tty/tty.rs` | Modify | Replace TODO at line 41 with SIGWINCH delivery; mirror `PtyMaster` pattern |
| `kernel/src/syscall/handlers.rs` | Modify | Rewrite the four ioctl arms (1238–1286) to use the fd-resolver |
| `tests/integration/tty_signals.rs` | Create | Boot-test: pair `/dev/ptmx` + slave, `TIOCSWINSZ` on master, slave-side handler counts SIGWINCH |
| `.github/workflows/ci.yml` | Modify | Wire the new boot-test into the integration-smoke job from Phase 2 |

---

## Task 1: Expose a fd→TTY resolver

**Files:**
- Modify: `kernel/src/vfs/devfs.rs` (or `kernel/src/vfs/file.rs` — whichever owns `OpenFile`)
- Modify: `kernel/src/task/scheduler.rs` (helper wrapping `with_current_fd_table` + the resolver)

- [ ] **Step 1: Read existing inode/devfs registration**

```bash
sed -n '1,80p' kernel/src/vfs/devfs.rs
grep -n "register\|ptmx\|pty" kernel/src/vfs/devfs.rs
```

Understand how PTY master/slave inodes are created and what concrete type sits behind their `InodeOps` impl. Likely a `PtyMasterInode { master: Arc<Mutex<PtyMaster>> }` or similar.

- [ ] **Step 2: Decide on the resolver shape**

Pick the smallest patch:

- **If `InodeOps` is a trait**: add a default-`None` method `fn as_pty_master(&self) -> Option<&core::cell::RefCell<PtyMaster>> { None }` (or whichever interior-mutability primitive the existing PTY storage uses). Override on the PTY-master inode type.
- **If PTY storage is `Arc<Mutex<PtyMaster>>`**: add `fn pty_master_arc(&self) -> Option<Arc<Mutex<PtyMaster>>> { None }` for the same reason.

Pick the variant that compiles against the actual existing types. The principle: **do not introduce a new locking primitive** — reuse whatever PtyMasterInode already uses internally.

- [ ] **Step 3: Add a scheduler wrapper that resolves an fd to a PtyMaster**

In `kernel/src/task/scheduler.rs`, near `with_current_fd_table`, add:

```rust
    /// Look up the file descriptor `fd` in the current task's table, fetch
    /// the backing inode, and call `f` with a mutable PtyMaster reference if
    /// the file is a PTY master. Returns `None` if the fd is invalid or not
    /// a PTY master.
    pub fn with_current_pty_master_mut<R>(
        &mut self,
        fd: i32,
        f: impl FnOnce(&mut crate::tty::pty::PtyMaster) -> R,
    ) -> Option<R> {
        let task = self.tasks[self.current].as_mut()?;
        let file = task.fd_table.get(fd).ok()?;
        let arc = file.inode.pty_master_arc()?; // see Step 2
        let mut guard = arc.lock();              // adjust to the actual lock kind
        Some(f(&mut *guard))
    }
```

(If the PTY storage is a `RefCell`, replace `.lock()` with `.borrow_mut()`. Stay consistent with the existing access pattern in `pty.rs`.)

Expose a free wrapper too. The scheduler is a `static mut SCHEDULER: Option<Scheduler>` accessed via `core::ptr::addr_of_mut!`. Match the existing pattern (see `take_pending_signal` at `scheduler.rs:714` and `send_signal_to` at `scheduler.rs:728`):

```rust
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn with_current_pty_master_mut<R>(
    fd: i32,
    f: impl FnOnce(&mut crate::tty::pty::PtyMaster) -> R,
) -> Option<R> {
    let sched = (*core::ptr::addr_of_mut!(SCHEDULER)).as_mut()?;
    sched.with_current_pty_master_mut(fd, f)
}
```

- [ ] **Step 4: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 5: Commit**

```bash
git add kernel/src/vfs/devfs.rs kernel/src/task/scheduler.rs kernel/src/vfs/file.rs
git commit -m "feat(kernel): fd-to-PtyMaster resolver for ioctl dispatch"
```

(Include `vfs/file.rs` only if the `InodeOps` trait lives there.)

---

## Task 2: Route TIOCGWINSZ through the resolver

**Files:**
- Modify: `kernel/src/syscall/handlers.rs:1238-1248`

- [ ] **Step 1: Replace the TIOCGWINSZ arm**

In the `match request` block of `sys_ioctl`, replace the existing `TIOCGWINSZ =>` arm with:

```rust
        TIOCGWINSZ => {
            validate_user_ptr(arg, 4)?;
            let ws = unsafe {
                crate::task::scheduler::with_current_pty_master_mut(fd, |pty| pty.winsize())
            };
            let ws = ws.unwrap_or(crate::tty::line_discipline::WinSize::default());
            unsafe {
                let ptr = arg as *mut u16;
                *ptr = ws.rows;
                *ptr.add(1) = ws.cols;
            }
            Ok(0)
        }
```

This preserves the "fall back to default" behaviour when the fd is not a TTY (e.g. tests).

- [ ] **Step 2: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 3: Commit**

```bash
git add kernel/src/syscall/handlers.rs
git commit -m "feat(kernel): TIOCGWINSZ now reads from the TTY behind the fd"
```

---

## Task 3: Route TIOCSWINSZ through the resolver

**Files:**
- Modify: `kernel/src/syscall/handlers.rs:1249-1271`

- [ ] **Step 1: Replace the TIOCSWINSZ arm**

Replace the existing arm with:

```rust
        TIOCSWINSZ => {
            validate_user_ptr(arg, 4)?;
            let (rows, cols) = unsafe {
                let ptr = arg as *const u16;
                (*ptr, *ptr.add(1))
            };
            if rows == 0 || cols == 0 || rows >= 10_000 || cols >= 10_000 {
                return Err(SyscallError::EINVAL);
            }
            let updated = unsafe {
                crate::task::scheduler::with_current_pty_master_mut(fd, |pty| {
                    pty.set_winsize(rows, cols);   // already delivers SIGWINCH
                })
            };
            if updated.is_none() {
                // Not a PTY — fall back to current behaviour: deliver SIGWINCH
                // to the caller's pgid so legacy shells still see resize events.
                let pgid = crate::task::scheduler::current_pgid();
                if pgid != 0 {
                    unsafe {
                        core::arch::asm!("cli", options(nomem, nostack));
                        crate::task::scheduler::send_signal_to_group(
                            pgid,
                            crate::task::signal::Signal::SIGWINCH,
                        );
                        core::arch::asm!("sti", options(nomem, nostack));
                    }
                }
            }
            Ok(0)
        }
```

The kernel-side TODO is now removed because `PtyMaster::set_winsize` (pty.rs:116-129) already updates winsize **and** delivers SIGWINCH to its tracked `foreground_pgid`.

- [ ] **Step 2: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 3: Commit**

```bash
git add kernel/src/syscall/handlers.rs
git commit -m "feat(kernel): TIOCSWINSZ updates TTY state and routes SIGWINCH to fg group"
```

---

## Task 4: Route TIOCGPGRP through the resolver

**Files:**
- Modify: `kernel/src/syscall/handlers.rs:1272-1278`

- [ ] **Step 1: Replace the arm**

```rust
        TIOCGPGRP => {
            validate_user_ptr(arg, 4)?;
            let pgid = unsafe {
                crate::task::scheduler::with_current_pty_master_mut(fd, |pty| pty.foreground_pgid)
            };
            let out = pgid.map(|v| v as u32).unwrap_or_else(crate::task::scheduler::current_pgid);
            unsafe { *(arg as *mut u32) = out; }
            Ok(0)
        }
```

When the fd is not a TTY, fall back to `current_pgid()` (preserves the old observable behaviour).

- [ ] **Step 2: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 3: Commit**

```bash
git add kernel/src/syscall/handlers.rs
git commit -m "feat(kernel): TIOCGPGRP returns the TTY foreground pgid"
```

---

## Task 5: Route TIOCSPGRP through the resolver

**Files:**
- Modify: `kernel/src/syscall/handlers.rs:1279-1286`

- [ ] **Step 1: Replace the arm**

```rust
        TIOCSPGRP => {
            validate_user_ptr(arg, 4)?;
            let pgid = unsafe { *(arg as *const u32) };
            if pgid == 0 {
                return Err(SyscallError::EINVAL);
            }
            // Verify the target pgid contains at least one task in the same session
            // as the caller.
            let caller_session = crate::task::scheduler::current_session_id();
            let valid = unsafe {
                core::arch::asm!("cli", options(nomem, nostack));
                let pids = crate::task::scheduler::pids_in_group(pgid);
                let same_session = pids.iter().any(|pid| {
                    crate::task::scheduler::session_id_of(*pid)
                        .map(|s| s == caller_session)
                        .unwrap_or(false)
                });
                core::arch::asm!("sti", options(nomem, nostack));
                !pids.is_empty() && same_session
            };
            if !valid {
                return Err(SyscallError::EPERM);
            }
            let updated = unsafe {
                crate::task::scheduler::with_current_pty_master_mut(fd, |pty| {
                    pty.set_foreground(pgid as i32);
                })
            };
            if updated.is_none() {
                // No TTY behind fd — accept silently to preserve old behaviour.
            }
            Ok(0)
        }
```

If `session_id_of(pid)` does not yet exist as a free function, add it next to the other free wrappers (e.g. just after the `current_pgid` free function, around `scheduler.rs:633`). Match the `static mut SCHEDULER` access pattern:

```rust
/// Return the session_id of a task by PID, or None if no such task exists.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn session_id_of(pid: Pid) -> Option<Pid> {
    let sched = (*core::ptr::addr_of!(SCHEDULER)).as_ref()?;
    sched.tasks.iter().flatten().find(|t| t.pid == pid).map(|t| t.session_id)
}
```

Also expose `pids_in_group` as a free wrapper if it isn't already:

```rust
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn pids_in_group(pgid: Pid) -> alloc::vec::Vec<Pid> {
    (*core::ptr::addr_of!(SCHEDULER))
        .as_ref()
        .map(|s| s.pids_in_group(pgid))
        .unwrap_or_default()
}
```

- [ ] **Step 2: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 3: Commit**

```bash
git add kernel/src/syscall/handlers.rs kernel/src/task/scheduler.rs
git commit -m "feat(kernel): TIOCSPGRP updates TTY fg pgid with session check"
```

---

## Task 6: Make `Tty::set_winsize` deliver SIGWINCH

**Files:**
- Modify: `kernel/src/tty/tty.rs:39-42`

- [ ] **Step 1: Replace `Tty::set_winsize`**

Replace the existing method with:

```rust
    pub fn set_winsize(&mut self, rows: u16, cols: u16) {
        let changed = self.winsize.rows != rows || self.winsize.cols != cols;
        self.winsize = WinSize { rows, cols };
        if changed && self.foreground_pgid > 0 {
            // SAFETY: signal delivery requires the scheduler lock; the caller
            // holds whatever lock protects the Tty, not the scheduler.
            unsafe {
                core::arch::asm!("cli", options(nomem, nostack));
                crate::task::scheduler::send_signal_to_group(
                    self.foreground_pgid as u32,
                    crate::task::signal::Signal::SIGWINCH,
                );
                core::arch::asm!("sti", options(nomem, nostack));
            }
        }
    }
```

This mirrors the `PtyMaster::set_winsize` pattern at `pty.rs:116-129`.

- [ ] **Step 2: Build**

```bash
cargo build --package racore --target x86_64-unknown-none
```

- [ ] **Step 3: Commit**

```bash
git add kernel/src/tty/tty.rs
git commit -m "feat(kernel): Tty::set_winsize delivers SIGWINCH to fg pgid"
```

---

## Task 7: Integration test — TTY SIGWINCH round-trip

**Files:**
- Create: `tests/integration/tty_signals.rs`
- Modify: `scripts/build-image.sh` (copy the test binary into initramfs)
- Modify: `init/src/main.rs` (run the test under the `boot-test` feature gate)

- [ ] **Step 1: Write the test program**

```rust
// tests/integration/tty_signals.rs
//
// Open /dev/ptmx (master side), spawn a child that opens the matching slave,
// install a SIGWINCH handler, then call TIOCSWINSZ on the master from the
// child. The handler increments a counter; we then verify it reached 1.

#![no_std]
#![no_main]

extern crate libc_lite;

use core::sync::atomic::{AtomicU32, Ordering};

const TIOCSWINSZ: u32 = 0x5414;
static HIT: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_sigwinch(_signo: i32) {
    HIT.fetch_add(1, Ordering::SeqCst);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Acquire the master + slave pair via ptmx.
    let master_fd = libc_lite::open("/dev/ptmx\0", 0o2 /* O_RDWR */);
    if master_fd < 0 {
        libc_lite::write_str(1, "TTY-SIGWINCH-FAIL-OPEN-MASTER\n");
        libc_lite::exit(1);
    }
    let slave_fd = libc_lite::open("/dev/pts/0\0", 0o2);
    if slave_fd < 0 {
        libc_lite::write_str(1, "TTY-SIGWINCH-FAIL-OPEN-SLAVE\n");
        libc_lite::exit(1);
    }

    // Install handler then trigger resize via master ioctl.
    libc_lite::sigaction(/* SIGWINCH */ 28, on_sigwinch as u64);
    let ws: [u16; 2] = [40, 80];
    libc_lite::ioctl(master_fd, TIOCSWINSZ, ws.as_ptr() as u64);

    // Allow the delivery to land before reading the counter. nanosleep
    // gives the scheduler a chance to run deliver_pending_signals.
    libc_lite::nanosleep(0, 5_000_000);

    if HIT.load(Ordering::SeqCst) >= 1 {
        libc_lite::write_str(1, "TTY-SIGWINCH-OK\n");
        libc_lite::exit(0);
    } else {
        libc_lite::write_str(1, "TTY-SIGWINCH-FAIL\n");
        libc_lite::exit(1);
    }
}
```

> If `/dev/ptmx`/`/dev/pts/0` are not yet created in `devfs`, add minimal devices (PTY master at `/dev/ptmx`, slave at `/dev/pts/0`) in `kernel/src/vfs/devfs.rs` registration — match the `register_defaults` pattern noted in the exploration.

- [ ] **Step 2: Stage in initramfs**

In `scripts/build-image.sh`, after the userland coreutils copy, add:

```bash
if [[ -f "$TARGET_DIR/x86_64-racos-user/$PROFILE_DIR/tty_signals" ]]; then
    cp -f "$TARGET_DIR/x86_64-racos-user/$PROFILE_DIR/tty_signals" \
        "$INITRAMFS_ROOT/bin/tty-signals-test"
fi
```

- [ ] **Step 3: Run from init under the feature flag**

In `init/src/main.rs`, alongside the Phase 2 test invocations:

```rust
#[cfg(feature = "boot-test-tty")]
let _ = libc_lite::spawn_args("/bin/tty-signals-test", &[]);
```

- [ ] **Step 4: Run locally**

```bash
just build-image && just run-uefi
```

Expected: serial output contains `TTY-SIGWINCH-OK`. If not, inspect the serial log around the TIOCSWINSZ call.

- [ ] **Step 5: Commit**

```bash
git add tests/integration/tty_signals.rs scripts/build-image.sh init/src/main.rs kernel/src/vfs/devfs.rs
git commit -m "test: integration boot test for TTY SIGWINCH delivery"
```

---

## Task 8: Wire the TTY test into CI

**Files:**
- Modify: `.github/workflows/ci.yml` (extend the `integration-smoke` job from Phase 2)

- [ ] **Step 1: Add the assertion to the existing integration-smoke job**

In the "Assert SIGNAL-ROUNDTRIP-OK + EXEC-LOOP-OK" step, append:

```bash
          grep -q "TTY-SIGWINCH-OK" boot.log \
            || (echo "FAIL: tty signals test did not pass" && exit 1)
```

And in the `Build with boot-test feature` step, expand the feature list:

```yaml
      - name: Build with boot-test feature
        run: |
          cargo build --package racore --target x86_64-unknown-none --features boot-test-signals,boot-test-tty
          cargo build --package racos-boot --target x86_64-unknown-uefi
          cargo build --workspace --exclude racore --exclude racos-boot --features boot-test-signals,boot-test-tty
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: extend integration-smoke to assert TTY SIGWINCH"
```

---

## Definition of done for Phase 3

All true:

- ✅ `tests/integration/tty_signals.rs` reports `TTY-SIGWINCH-OK` on every boot.
- ✅ TODO at `kernel/src/tty/tty.rs:41` removed (replaced by Task 6 body).
- ✅ TODO at `kernel/src/syscall/handlers.rs:1256` removed (replaced by Task 3 body).
- ✅ TODO at `kernel/src/syscall/handlers.rs:1283` removed (replaced by Task 5 body).
- ✅ No regressions in `boot-smoke`, `interactive-smoke`, or Phase 2 `integration-smoke` assertions.

---

## Self-review notes

- **Spec coverage**: §6.1–6.4 are addressed by Tasks 1–6 (resolver + four ioctl rewrites + Tty::set_winsize SIGWINCH). §6.5 is satisfied by Task 7 + Task 8.
- **Placeholders**: Task 1 Step 2 explicitly forks based on what the existing PTY storage uses (RefCell vs Mutex) — this is a deliberate "match what's there" instruction, not a hidden TODO. Task 7 Step 1 has a callout that PTY device files may need devfs registration.
- **Type consistency**: `with_current_pty_master_mut` is referenced from Tasks 2–5; same name used in Task 1's definition. `WinSize`'s field names (`rows`, `cols`) match the existing struct at `kernel/src/tty/line_discipline.rs:21`.
- **Scope**: This plan does not implement `TIOCGSID`, `TIOCNOTTY`, controlling-terminal acquisition, SIGTSTP delivery, or `tcsetattr` — those are next-sprint work. SIGWINCH delivery to the foreground pgid is the only signal-routing change.

---

## Open follow-ups (NOT in this plan)

- Controlling terminal acquisition: when a process calls `setsid()` and then opens a TTY, the kernel should make it the controlling TTY. Today this isn't tracked.
- `tcsetattr` / `tcgetattr` (termios manipulation) — needed for canonical/raw mode toggle from user space.
- SIGTSTP/SIGCONT lifecycle: Ctrl-Z handling and job-control state machine in the shell.
- `TIOCSCTTY`, `TIOCNOTTY`, `TIOCGSID` — the rest of the POSIX TTY ioctl set.
- Per-`Tty` registry (one master ↔ one slave ↔ one Tty) so kernel-console TTYs and PTYs share the same code path.
