# Cross-platform build + kernel correctness sprint

**Status**: Draft for review
**Author**: Claude (brainstorming session, 2026-05-20)
**Owner**: @RaCzKoViC
**Repo**: https://github.com/RaCzKoViC/RacOS

## 1. Goal

Unblock native RacOS development on Linux and close five critical correctness gaps in the kernel that, together, prevent the system from running real userland workloads reliably. After this sprint:

- `just build` and `just qemu` work natively on Ubuntu/Debian Linux without any user-side patches.
- Fork/exec/exit loops no longer leak kernel memory.
- Signal handlers return correctly to the interrupted user context.
- Terminal resize and job control work end-to-end in `racsh` on a PTY.

Out of scope for this sprint: SMP AP startup, module loader, blocking poll, network stack, package manager completion, ARM/RISC-V port, GUI desktop. See §11.

## 2. Background

RacOS (https://github.com/RaCzKoViC/RacOS) is an x86_64 UEFI operating system written from scratch in Rust. As of HEAD `0e325c8` it ships:

- ~17K LOC kernel with 79 syscalls, VFS (initramfs/tmpfs/devfs/racfs), SMP-aware scheduler skeleton, ELF loader, framebuffer console, fork/exec/wait wired
- 36 userland coreutils (most full-featured, a few stubs)
- 45-crate Cargo workspace
- 20 closed ADRs + 11 specs

Two classes of blockers prevent shipping a credible MVP today:

**Class A — developer experience**: `justfile` is PowerShell-only with hardcoded `C:\Users\Maciej\…` paths. The maintainer recently switched to Linux and cannot build the project natively.

**Class B — kernel correctness**: Eight `// TODO` markers in kernel code; five are on the critical path for any real userland (process cleanup, sigreturn, TTY winsize, TTY foreground pgid, SIGWINCH delivery).

This sprint addresses Class A in full and the five critical Class B TODOs. Three remaining TODOs (SMP AP startup, blocking poll, module loader) are deferred.

## 3. Scope and constraints

### In scope

1. **Cross-platform build system** (Phase 1)
2. **Process cleanup on exit** — `kernel/src/syscall/handlers.rs:415` (Phase 2)
3. **`sigreturn` user-context restore** — `kernel/src/syscall/handlers.rs:1896` (Phase 2)
4. **TIOCGWINSZ / TIOCSWINSZ** — `kernel/src/syscall/handlers.rs:1256` (Phase 3)
5. **TIOCGPGRP / TIOCSPGRP** — `kernel/src/syscall/handlers.rs:1283` (Phase 3)
6. **SIGWINCH delivery to foreground process group** — `kernel/src/tty/tty.rs:41` (Phase 3)

### Constraints

- Must not regress existing Windows development flow (parallel scripts, no removal of `.ps1`).
- Must not break boot-smoke on `main`.
- Must follow ADR-002 (modular monolithic kernel) and ADR-005 (versioned syscall ABI) — no breaking changes to existing syscall numbers or semantics.
- Every `unsafe` block introduced must carry the `SAFETY:`/`INVARIANT:`/`FAILURE:`/`TESTED BY:` comment block per `docs/architecture/ARCHITECTURE.md` §3.3.
- Every closed TODO must be paired with a test that would have failed against the prior code.

## 4. Architecture: Phase 1 — Cross-platform build

### 4.1 Justfile strategy

The current `justfile` declares `set shell := ["powershell", …]`. Replace with `just`'s OS-attribute recipes:

```just
# Each recipe that needs OS-specific behaviour declares both variants:
[unix]
build:
    ./scripts/build-image.sh

[windows]
build:
    powershell -File scripts/build-image.ps1
```

For recipes whose logic is portable (mostly `cargo` invocations), keep a single recipe with no `[os]` attribute.

Drop hardcoded `C:\Users\Maciej\RacOS-target` — replace with `env_var_or_default("RACOS_TARGET_DIR", "target")`. Document the env var in both DEVELOPMENT_*.md.

### 4.2 Scripts: parallel `.sh` + `.ps1`

For each existing `scripts/*.ps1`, add a 1-to-1 `scripts/*.sh` next to it:

| PowerShell script | New bash equivalent |
|---|---|
| `build-image.ps1` | `build-image.sh` |
| `make-image.ps1` | `make-image.sh` |
| `make-iso.ps1` | `make-iso.sh` |
| `pack-initramfs.ps1` | `pack-initramfs.sh` (or thin wrapper over existing `pack-initramfs.py`) |
| `run-qemu.ps1` | `run-qemu.sh` |
| `runtime-validation.ps1` | `runtime-validation.sh` |
| `runtime-validation-interactive.ps1` | `runtime-validation-interactive.sh` |
| `validate-direct-boot.ps1` | `validate-direct-boot.sh` |
| `validate-esp-boot.ps1` | `validate-esp-boot.sh` |
| `validate-runtime.ps1` | `validate-runtime.sh` |
| `validate-system.ps1` | `validate-system.sh` |

Bash scripts target `bash >= 5`, use `set -euo pipefail`, and rely on standard Ubuntu/Debian tools: `qemu-system-x86_64`, `mtools`, `dosfstools`, `xorriso`. The existing `scripts/pack-initramfs.py` stays as a portable helper.

### 4.3 Developer documentation

Create:

- `docs/DEVELOPMENT_LINUX.md` — apt install one-liner for dependencies (`qemu-system-x86 mtools dosfstools xorriso just rustup`), how to set `RACOS_TARGET_DIR`, `just build`, `just qemu`, troubleshooting block.
- `docs/DEVELOPMENT_WINDOWS.md` — extract current Windows knowledge from `.github/instructions/development.instructions.md` (or link to it), add same env-var instructions.

Add a "Quick start" section to root `README.md` linking to both.

### 4.4 CI matrix

Extend `.github/workflows/ci.yml`:

```yaml
strategy:
  fail-fast: false
  matrix:
    include:
      - os: ubuntu-22.04
        runtime_tests: true
        required: true
      - os: windows-latest
        runtime_tests: false
        required: false           # advisory
      - os: macos-latest
        runtime_tests: false
        required: false           # build-only sanity
```

- `ubuntu-22.04` runs build + unit tests + integration + boot-smoke. Blocking.
- `windows-latest` runs build + unit tests. `continue-on-error: true`.
- `macos-latest` runs `cargo check` only. `continue-on-error: true`.

The pre-existing `interactive-smoke` job stays on `ubuntu-22.04` only.

### 4.5 Definition of done (Phase 1)

- Fresh clone on Ubuntu 22.04 → `apt install` per `DEVELOPMENT_LINUX.md` → `just build` succeeds.
- `just qemu` boots RacOS through to the shell prompt on Linux.
- CI workflow green on `ubuntu-22.04`; `windows-latest` and `macos-latest` jobs report status without blocking.
- Existing Windows build manually verified by a maintainer on a Windows machine before merging (smoke).
- No deletion of `.ps1` scripts; no removal of Windows-only paths from anywhere they were authoritative.

## 5. Architecture: Phase 2 — Kernel correctness

### 5.1 Process cleanup (`handlers.rs:415`)

**Current behaviour**: `sys_exit(code)` marks the task as exited but does not release the user address space, file descriptors, kernel stack, or process struct. Each `fork → exec → exit → waitpid` cycle leaks the user page tables and the kernel stack of the exited child.

**Target behaviour**: model after Linux's `do_exit` + `release_task` split.

```text
sys_exit(code):
    1. Close all file descriptors: for fd in task.fd_table.drain(): close(fd)
    2. Switch CR3 to kernel-only page table (snapshot from mm::virt::capture_kernel_cr3())
    3. Drop strong ref to task.mm (Arc<UserAddressSpace>):
         when refcount → 0, UserAddressSpace::drop() walks the user portion of the
         page table and returns every mapped frame to mm::phys via free_frame()
    4. Reparent children: for child in task_table.iter() where child.ppid == self.pid:
         child.ppid = 1                  // RacInit adopts orphans
    5. task.state = TaskState::Zombie { exit_code }
    6. Wake parent if waiting: parent.wait_cv.notify_all() if parent.state == Waiting
    7. task::scheduler::yield_to_next() — current task stays on its kernel stack
       in Zombie state until waitpid() collects it.

sys_waitpid(pid, status_out, options) — extension to existing code:
    1. After reading task.exit_code into status_out and removing it from parent's
       children list:
    2. task_table.remove(task.pid)
    3. Schedule kernel-stack free via task::reaper queue (deferred to avoid
       freeing the stack of a task that is being context-switched away from).
```

**Reaper queue**: a per-CPU queue of `Box<KernelStack>` (or `Arc<TaskStruct>`) drained at the top of the idle loop and at every scheduler tick. This keeps the free path off the exiting task's hot path.

**Locking**: cleanup runs under `task_table.write_lock()` to prevent races with `waitpid` and `fork` on the same parent. Reparenting iterates atomically.

**Edge cases**:

- Parent already dead at exit time → reparent self to PID 1 inside step 4 logic (RacInit becomes the parent). RacInit's existing reaper loop (in `init/src/engine.rs`) collects status.
- Process with no children → step 4 is a no-op.
- Kernel thread (no userland mm) → skip step 3.

### 5.2 `sigreturn` (`handlers.rs:1896`)

**Current behaviour**: `sys_sigreturn` returns dummy values. Signal handlers cannot safely return to interrupted user code.

**Target architecture**:

When a signal is delivered (logic already exists in `sigaction` stub), the kernel pushes a `SigFrame` onto the user stack:

```rust
#[repr(C)]
struct SigFrame {
    saved_regs: UserRegs,    // RIP, RSP, RBP, RFLAGS, RAX..R15
    saved_sigmask: u64,
    fp_state: [u8; 512],     // placeholder for XSAVE; MVP zero-fills
    signal_number: u32,
    _pad: u32,
}
```

`task.in_signal_handler = true` is set before transferring to the handler. Handler runs at user-level RIP = `task.sigaction[signo].sa_handler`. Handler's return address is the VDSO trampoline (see below).

**VDSO trampoline**: a single read-execute page mapped per-process in user space at a fixed virtual address (chosen below the kernel range, e.g. `0x7FFF_FFFE_F000`). Contents:

```asm
mov rax, SYS_sigreturn
syscall
```

Mapping happens during `mm::virt::create_user_page_table` (next to existing initramfs map). No ELF parsing, no relocations, no dynamic loader.

**`sys_sigreturn` implementation**:

```text
1. Verify task.in_signal_handler == true; else SIGSEGV the task (corrupted state).
2. Read saved_signal_frame_ptr from task struct (kernel stored it at delivery).
3. copy_from_user(frame_ptr, &mut frame: SigFrame). On fault → SIGSEGV.
4. Validate: frame_ptr in user range, signal_number matches what was delivered.
5. Restore task.regs = frame.saved_regs (UserRegs).
6. Restore task.sigmask = frame.saved_sigmask.
7. task.in_signal_handler = false.
8. Return from syscall — the dispatch trampoline does iretq with restored regs.
```

**Nesting**: if a second signal arrives while the first handler runs, the kernel pushes a second SigFrame on top of the first. `sigreturn` for the inner signal restores the outer signal's frame as "the interrupted context". Both unwind in LIFO order. No special code needed — the mechanism nests naturally.

**FP state caveat**: XSAVE area zero-filled in MVP. Userland code that uses SSE/AVX through a signal boundary will see clobbered FP regs. This is a known limitation, documented in `DEVELOPMENT_LINUX.md` and tracked as a follow-up.

### 5.3 Definition of done (Phase 2)

- `tests/unit/exit_cleanup.rs`: synthetic test that exits a fake task, asserts that `mm::phys::free_count()` increases by the number of pages the task held.
- `tests/integration/exec_loop.rs`: spawn 100 fork→exec(`/bin/true`)→waitpid cycles in a row; assert kernel RSS delta < 64 KiB.
- `tests/unit/sigreturn.rs`: hand-crafted SigFrame on user stack, verify register restore.
- `tests/integration/signal_roundtrip.rs`: process raises SIGUSR1 on itself, handler sets a flag, returns; assert flag == 1 and RIP after handler is the instruction after `kill()`.
- TODO comments at `handlers.rs:415` and `handlers.rs:1896` removed.
- Boot-smoke green.

## 6. Architecture: Phase 3 — TTY pack

### 6.1 State additions to `Tty`

In `kernel/src/tty/tty.rs`:

```rust
struct Tty {
    // existing fields ...
    winsize: AtomicCell<WinSize>,       // or Mutex<WinSize> if AtomicCell unavailable
    fg_pgid: AtomicI32,                 // 0 = no foreground group attached
    // session_id: already present
}

#[repr(C)]
#[derive(Copy, Clone)]
struct WinSize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}
```

### 6.2 Four ioctls

Numbers chosen to match the conventional Linux values for ABI familiarity:

```rust
const TIOCGWINSZ: u32 = 0x5413;
const TIOCSWINSZ: u32 = 0x5414;
const TIOCGPGRP:  u32 = 0x540F;
const TIOCSPGRP:  u32 = 0x5410;
```

Each ioctl resolves `fd → File → Tty` through the existing VFS plumbing, then:

**TIOCGWINSZ** (read winsize):
1. `copy_to_user(arg_ptr, &tty.winsize.load(), size_of::<WinSize>())` → return 0.

**TIOCSWINSZ** (write winsize):
1. `copy_from_user(arg_ptr, &mut new: WinSize)`.
2. Validate: `new.ws_row > 0 && new.ws_row < 10_000`, same for `ws_col`. Else EINVAL.
3. `let old = tty.winsize.swap(new)`.
4. If `old != new`: `tty.deliver_sigwinch()`.
5. Return 0.

**TIOCGPGRP** (read foreground pgid):
1. Verify caller's session: `task.sid == tty.session_id`. Else EPERM.
2. `copy_to_user(arg_ptr, &tty.fg_pgid.load(), size_of::<i32>())` → return 0.

**TIOCSPGRP** (write foreground pgid):
1. `copy_from_user(arg_ptr, &mut new_pgid: i32)`.
2. Validate: `new_pgid > 0`. Else EINVAL.
3. Verify a process with that pgid exists: `task::pgroup::exists(new_pgid)`. Else EPERM.
4. Verify same-session: `task.sid == tty.session_id && pgroup_session(new_pgid) == tty.session_id`. Else EPERM.
5. `tty.fg_pgid.store(new_pgid)`. Return 0.

### 6.3 SIGWINCH delivery (`tty/tty.rs:41`)

New method on `Tty`:

```rust
impl Tty {
    fn deliver_sigwinch(&self) {
        let pgid = self.fg_pgid.load(Ordering::Acquire);
        if pgid == 0 { return; }
        task::pgroup::iter_pgroup(pgid, |task| {
            task::signal::queue_signal(task, Signal::SIGWINCH);
        });
    }
}
```

**Helper to add** if absent: `task::pgroup::iter_pgroup(pgid, callback)` — linear scan over the task table; O(N tasks). Acceptable for MVP. A pgid→task-list index is a known future optimisation.

### 6.4 Definition of done (Phase 3)

- `tests/unit/tty_ioctl.rs`: positive/negative cases for each of the four ioctls (set/get round-trip, invalid input, cross-session EPERM, nonexistent pgid EPERM).
- `tests/integration/tty_signals.rs`: spawn shell on PTY pair, write a SIGWINCH-counter handler, master-side `TIOCSWINSZ` with new dimensions, assert shell received SIGWINCH.
- Boot-smoke extension: after shell starts, fake a resize event via test hook, assert kernel log line `SIGWINCH delivered to pgid=N`.
- TODO comments at `handlers.rs:1256`, `handlers.rs:1283`, `tty/tty.rs:41` removed.
- No regressions in existing tests.

## 7. Cross-cutting testing strategy

| Phase | Unit | Integration | Boot-smoke addition |
|---|---|---|---|
| 1 | — | — | CI matrix runs existing suite on Linux + Windows |
| 2 | `exit_cleanup.rs`, `sigreturn.rs` | `exec_loop.rs` (RSS check), `signal_roundtrip.rs` | Assert `free_pages ≥ baseline` after 100 exec'd processes |
| 3 | `tty_ioctl.rs` | `tty_signals.rs` (PTY resize → SIGWINCH on pgid) | Resize-event hook + kernel-log assertion |

**Test-first discipline**: each closed TODO is paired with at least one test that would fail against the pre-fix code. The test is committed in the same PR as the fix.

## 8. Milestones

```
W1 ────────── W2 ────────── W3 ────────── W4
│ Phase 1                                  │
│ • Port 11 .ps1 → .sh                     │
│ • Justfile [unix]/[windows] split        │
│ • CI matrix                              │
│ • DEVELOPMENT_LINUX.md                   │
│ • PR #1 → merge                          │
│         │ Phase 2                        │
│         │ • #4 process cleanup           │
│         │ • #7 sigreturn + VDSO          │
│         │ • Unit + integration tests     │
│         │ • PR #2 → merge                │
│                  │ Phase 3               │
│                  │ • 4 TTY ioctls        │
│                  │ • SIGWINCH delivery   │
│                  │ • pgroup iter helper  │
│                  │ • Tests               │
│                  │ • PR #3 → merge       │
│                                  │ Buffer
│                                  │ Bug fixes, review, changelog
```

Estimate: **~3 weeks active work + 1 week buffer = 1 month** for a solo full-time developer. Multiply 2–3× if part-time.

**Hard gates between phases**:

- Phase 1 → 2: `just qemu` on Linux must boot through to shell prompt. Without this, Phase 2 can't be iterated locally.
- Phase 2 → 3: signal round-trip must work end-to-end (SIGUSR1 to self, handler runs, returns). Without sigreturn, SIGWINCH delivery has nowhere to return to.

## 9. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `.ps1` → `.sh` port surfaces hidden Windows assumptions (paths, encoding, EOL) | High | Medium | Phase 1 ships standalone before any kernel work. Full `just build && just qemu` smoke on both OSes before Phase 2. |
| Process cleanup race: parent calls `waitpid` before child marks itself Zombie | Medium | High (deadlock or lost exit status) | Cleanup runs under `task_table.write_lock`. Status field is written before `wait_cv.notify_all`. Memory-ordering-tested via a dedicated stress test in `exec_loop.rs`. |
| VDSO trampoline complicates user-space loader | Medium | Medium | Trampoline is a single 4 KiB read-execute page with hardcoded bytes, mapped in `create_user_page_table` next to initramfs. No ELF parsing, no relocations. Address fixed by convention. |
| TTY ioctls reveal master/slave coupling issues in `pty.rs` | Low | Medium | Read `kernel/src/tty/pty.rs` end-to-end during Phase 3 kickoff. Allocate 2 hours for a spike before writing code. |
| Windows CI flakes block merge | Low (mitigated by advisory status) | Low | `continue-on-error: true` on Windows + macOS jobs. Linux-only is required. |
| Local-edit data loss (analogue of the recent `sed -i` incident) | Low | High | Mandatory `git stash` before any in-place file mutation. Prefer `git apply` over scripted text replacement. No `sed -i` on tracked files without prior backup. |

## 10. Decisions made during brainstorming

| Decision | Choice | Alternative considered |
|---|---|---|
| Build-system strategy | True cross-platform (parallel scripts + matrix CI) | Linux-only, or Python consolidation |
| Script layout | `.sh` and `.ps1` side by side | Single Python source-of-truth, or `just` recipes only |
| CI gating | Linux required, Windows advisory | Both required |
| TODO scope | 5: process cleanup, sigreturn, 3× TTY | Minimal 2, or all 8 including SMP |
| PR shape | 3 sequential PRs | 8 micro-PRs, or one big-bang PR |

## 11. Out of scope

Explicitly **not** in this sprint, with rationale:

- **SMP AP startup (#1)** — separate, large effort; needs its own sprint with bring-up procedure, IPI design, per-CPU runqueue audit.
- **Module loader (#3)** — post-MVP per ADR-002.
- **Blocking poll (#8)** — busy-wait suffices for shell input in the interim; needs separate sleep/wake design.
- **Full job control beyond foreground-pgid + SIGWINCH** — SIGTSTP/SIGCONT delivery on Ctrl-Z, `tcsetattr`/`termios` configuration, line discipline canonical-mode quirks, stopped-job tracking in the shell. Phase 3 implements *foreground pgid bookkeeping* (TIOCGPGRP/TIOCSPGRP) and SIGWINCH delivery only; the rest of job control is the next sprint.
- **Full FP state in sigframe** — XSAVE area zero-filled; documented limitation.
- **SYSCALL_SPEC.md / PACKAGE_FORMAT.md completion** — documentation sprint, separate.
- **macOS CI runtime** — build-check only; no QEMU.
- **ARM/RISC-V port** — post-1.0 per ADR-001.
- **Network stack, package manager finalisation, GUI desktop** — separate sprints.

## 12. Acceptance criteria (sprint-level)

The sprint is complete when **all** of the following hold:

1. On a fresh Ubuntu 22.04 install with the documented `apt install` line, `git clone && just build && just qemu` reaches the RacOS shell prompt without further user-side patches.
2. `tests/integration/exec_loop.rs` runs 100 fork→exec→exit cycles; the test asserts `mm::phys::free_count()` after the loop is within 16 frames (≈ 64 KiB at 4 KiB pages) of the value captured before the loop, after a 10 ms settle to let the reaper queue drain.
3. `tests/integration/signal_roundtrip.rs` round-trips a SIGUSR1 handler and observes the post-handler RIP at the expected instruction.
4. `tests/integration/tty_signals.rs` delivers SIGWINCH to the foreground pgid after a master-side `TIOCSWINSZ`.
5. CI is green on `ubuntu-22.04`. Windows and macOS jobs run and report.
6. All five targeted TODO markers are removed from the source tree.
7. The pre-existing Windows build flow is manually verified by a maintainer.

## 13. Decisions resolved during spec self-review

- **VDSO virtual address**: `0x7FFF_FFFE_F000` (one 4 KiB page just below the user/kernel split). If this collides with existing user-mode mappings in `mm::virt`, the implementation plan picks the next free address in the same neighbourhood and updates this section.
- **WinSize storage in `Tty`**: `Mutex<WinSize>` (already-used primitive in the kernel; no new dependency on `crossbeam-utils`). Atomic upgrade is a follow-up only if profiling shows contention.
- **Reaper queue placement**: **per-CPU** queue. SMP work is post-1.0 but the data structure is built SMP-ready from day one to avoid a rewrite later. Concretely: a `PerCpu<Vec<ReaperEntry>>` drained at the top of the idle loop and at every scheduler tick.
