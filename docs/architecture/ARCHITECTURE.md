# RacOS — System Architecture

> Version: 0.1.0 (Draft)
> Status: Faza 0 — definicja architektury
> Last updated: 2026-04-04

## 1. Overview

RacOS is an original operating system with a layered architecture inspired by Ubuntu's organizational model. It is not a fork, clone, or derivative of any existing system. All code, formats, interfaces, and processes are original.

### 1.1 Architecture Style

**Modular monolithic kernel** — critical drivers run in kernel space, extensibility via kernel modules planned after ABI stabilization. Separate HAL/arch layer abstracts platform specifics.

### 1.2 Target Platform (v1.0)

| Parameter | Value |
|-----------|-------|
| CPU Architecture | x86_64 |
| Firmware | UEFI |
| Runtime Environment | QEMU/KVM |
| Boot Medium | ISO image + disk image |
| Boot Mode | UEFI → bootloader → kernel ELF64 + initramfs |

### 1.3 Language Stack

| Layer | Language | Rationale |
|-------|----------|-----------|
| Kernel core | Rust | Memory safety, zero-cost abstractions |
| Arch stubs, boot, context switch | x86_64 assembly | Hardware interface requirements |
| Userland (phase 1) | C17 | Lightweight, libc-lite compatible |
| Userland (phase 2) | Rust (optional) | After ABI stabilization |

## 2. Layered Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Layer 19: Release Engineering / Build / Updates        │
├─────────────────────────────────────────────────────────┤
│  Layer 18: Logging / Tracing / Metrics                  │
├─────────────────────────────────────────────────────────┤
│  Layer 17: Security Policy                              │
├─────────────────────────────────────────────────────────┤
│  Layer 16: Package System (rpkg + rapt)                 │
├─────────────────────────────────────────────────────────┤
│  Layer 15: TTY/PTTY + RacTerm                           │
├─────────────────────────────────────────────────────────┤
│  Layer 14: racsh (Shell)                                │
├─────────────────────────────────────────────────────────┤
│  Layer 13: Core Userland                                │
├─────────────────────────────────────────────────────────┤
│  Layer 12: RacInit + Service Manager                    │
├─────────────────────────────────────────────────────────┤
│  Layer 11: Drivers                                      │
├─────────────────────────────────────────────────────────┤
│  Layer 10: Filesystems (initramfs, tmpfs, racfs)        │
├─────────────────────────────────────────────────────────┤
│  Layer 9:  VFS + Device Model                           │
├─────────────────────────────────────────────────────────┤
│  Layer 8:  IPC / Signals / Pipes / Sockets              │
├─────────────────────────────────────────────────────────┤
│  Layer 7:  Syscall ABI                                  │
├─────────────────────────────────────────────────────────┤
│  Layer 6:  Scheduler / Tasking                          │
├─────────────────────────────────────────────────────────┤
│  Layer 5:  Memory Subsystem                             │
├─────────────────────────────────────────────────────────┤
│  Layer 4:  Interrupt / Exception Subsystem              │
├─────────────────────────────────────────────────────────┤
│  Layer 3:  Kernel Core (RaCore)                         │
├─────────────────────────────────────────────────────────┤
│  Layer 2:  Bootloader                                   │
├─────────────────────────────────────────────────────────┤
│  Layer 1:  Firmware / UEFI Boot                         │
└─────────────────────────────────────────────────────────┘
```

**Separation rule**: Each layer may only depend on the layer directly below it or on explicitly defined cross-cutting contracts (logging, security). No upward dependencies.

## 3. Kernel Architecture (RaCore)

### 3.1 Kernel Modules

```
racore/
├── arch/x86_64/     — GDT, IDT, TSS, paging, context switch, boot stub
├── boot/            — kernel entry, early init, multiboot/UEFI handoff
├── cpu/             — CPU feature detection, per-CPU data
├── interrupts/      — IDT management, IRQ routing, exception handlers
├── time/            — PIT/HPET/TSC timers, tick management
├── mm/              — physical allocator, virtual memory, kernel heap
├── sched/           — scheduler (RR → priority → fairness)
├── task/            — process/thread model, PID, PPID, sessions, groups
├── syscall/         — syscall entry/exit, dispatch table, ABI
├── ipc/             — signals, pipes, shared memory
├── vfs/             — VFS layer, inode/file/descriptor model
├── fs/              — filesystem implementations (initramfs, tmpfs, racfs)
├── dev/             — device model, /dev subsystem
├── drivers/         — serial, framebuffer, keyboard, block devices
├── net/             — network stack (post-MVP)
├── security/        — capabilities, permissions, isolation
└── debug/           — kernel debugger, panic handler, crash dumps
```

### 3.2 Kernel Space / User Space Separation

```
┌──────────────────────────┐  0xFFFF_FFFF_FFFF_FFFF
│     Kernel Space         │  
│  (RaCore + drivers)      │  Higher-half kernel mapping
│                          │
├──────────────────────────┤  0xFFFF_8000_0000_0000
│     Kernel Heap          │
├──────────────────────────┤
│     (unmapped guard)     │
├──────────────────────────┤  0x0000_7FFF_FFFF_FFFF
│     User Space           │
│  (per-process mappings)  │
│  Stack ↓                 │
│                          │
│  Heap ↑                  │
│  .data / .bss            │
│  .text                   │
├──────────────────────────┤  0x0000_0000_0040_0000
│     (unmapped guard)     │
└──────────────────────────┘  0x0000_0000_0000_0000
```

### 3.3 Unsafe Code Policy

Every `unsafe` block must include:
1. **WHY** it is necessary (no safe alternative)
2. **INVARIANTS** it relies on
3. **FAILURE MODES** — what can go wrong
4. **TEST COVERAGE** — which tests validate correctness

Format:
```rust
// SAFETY: <reason>
// INVARIANT: <what must hold>
// FAILURE: <what breaks if invariant violated>
// TESTED BY: <test name>
unsafe { ... }
```

## 4. Process Model

### 4.1 Process Properties

| Field | Type | Description |
|-------|------|-------------|
| pid | u32 | Process ID (1 = RacInit) |
| ppid | u32 | Parent PID |
| session_id | u32 | Session ID |
| pgrp | u32 | Process group |
| state | enum | Running, Ready, Blocked, Zombie, Stopped |
| address_space | PageTable | Per-process virtual memory |
| file_descriptors | FdTable | Open file descriptors |
| exit_status | i32 | Exit code |
| capabilities | CapSet | Capability bitmask |
| uid / gid | u32 | User / group |

### 4.2 Scheduler Progression

1. **MVP**: Round-robin with fixed time quantum
2. **v0.3**: Static priority levels (0–31)
3. **v0.5**: Fairness improvements (CFS-inspired, not copied)
4. **Post-1.0**: Real-time scheduling class

## 5. Memory Management

### 5.1 Physical Memory

- Frame allocator: bitmap-based (MVP), buddy allocator (later)
- Frame size: 4 KiB standard, 2 MiB huge pages (later)
- Memory map from UEFI BootServices

### 5.2 Virtual Memory

- 4-level page tables (PML4)
- Higher-half kernel mapping
- Per-process user address spaces
- Guard pages between stack/heap
- Copy-on-write: post-MVP optimization

### 5.3 Kernel Heap

- Slab allocator for fixed-size objects
- General-purpose allocator for variable-size allocations

## 6. Syscall ABI

### 6.1 ABI Convention

| Property | Value |
|----------|-------|
| Instruction | `syscall` (x86_64) |
| Syscall number | RAX |
| Arguments | RDI, RSI, RDX, R10, R8, R9 |
| Return value | RAX (result or negated error) |
| Clobbered | RCX, R11 |
| Versioning | Each syscall has a stability level |

### 6.2 Syscall Table v1

| Number | Name | Stability |
|--------|------|-----------|
| 0 | sys_exit | Stable |
| 1 | sys_read | Stable |
| 2 | sys_write | Stable |
| 3 | sys_open | Stable |
| 4 | sys_close | Stable |
| 5 | sys_stat | Stable |
| 6 | sys_mmap | Stable |
| 7 | sys_munmap | Stable |
| 8 | sys_pipe | Stable |
| 9 | sys_dup | Stable |
| 10 | sys_dup2 | Stable |
| 11 | sys_exec | Stable |
| 12 | sys_spawn | Stable |
| 13 | sys_wait | Stable |
| 14 | sys_getpid | Stable |
| 15 | sys_chdir | Stable |
| 16 | sys_ioctl | Unstable |
| 17 | sys_kill | Stable |
| 18 | sys_getcwd | Stable |

### 6.3 ABI Versioning

- ABI changes require an ADR and migration plan
- Deprecated syscalls remain available for 2 minor versions
- New syscalls do not change existing numbers

## 7. VFS and Filesystem

### 7.1 VFS Model

```
                ┌──────────┐
 userland  ──→  │ Syscalls │
                └────┬─────┘
                     │
                ┌────▼─────┐
                │   VFS    │  inode ops, file ops, dentry cache
                └────┬─────┘
          ┌──────────┼──────────┐
     ┌────▼────┐ ┌───▼───┐ ┌───▼───┐
     │initramfs│ │ tmpfs │ │ racfs │
     └─────────┘ └───────┘ └───────┘
```

### 7.2 Filesystems (v1.0)

| Filesystem | Purpose | Read | Write |
|------------|---------|------|-------|
| initramfs | Early boot, rootfs seed | Yes | No |
| tmpfs | Runtime temporary storage | Yes | Yes |
| racfs | Persistent root filesystem | Yes | Yes |

### 7.3 Special Directories

| Path | Purpose |
|------|---------|
| /dev | Device nodes |
| /proc | Process information (racproc) |
| /sys | System information (racsys) |
| /tmp | tmpfs mount |
| /etc | Configuration |
| /var | Variable data, logs |

## 8. Init and Service Manager (RacInit)

### 8.1 Boot Sequence

```
UEFI → Bootloader → RaCore (kernel) → RacInit (PID 1) → base.target → services
```

### 8.2 Unit Types

| Type | Extension | Purpose |
|------|-----------|---------|
| service | .service | Daemon or one-shot process |
| target | .target | Grouping / milestone |
| timer | .timer | Scheduled execution |
| mount | .mount | Filesystem mount |
| device | .device | Device availability |

### 8.3 Service Lifecycle

```
          ┌──────┐
          │ Loaded│
          └──┬───┘
             │ start
          ┌──▼───┐
     ┌────│Active │────┐
     │    └──┬───┘    │
     │       │ fail   │ stop
     │    ┌──▼───┐    │
     │    │Failed │    │
     │    └──┬───┘    │
     │       │        │
     │    restart?    │
     │    ┌──▼───┐    │
     └────│Active │    │
          └──────┘    │
                   ┌──▼───┐
                   │Stopped│
                   └──────┘
```

### 8.4 Admin CLI: `servicectl`

- `servicectl start <unit>`
- `servicectl stop <unit>`
- `servicectl restart <unit>`
- `servicectl status [unit]`
- `servicectl enable <unit>`
- `servicectl disable <unit>`
- `servicectl list [--all]`

## 9. Shell (racsh)

### 9.1 Architecture

```
Input → Lexer → Parser → AST → SemanticValidation → Expansion → ExecutionPlan → Runtime
```

Each stage is a separate module. Parser never executes code. Expansion is separate from lexing.

### 9.2 AST Nodes

`SimpleCommand`, `Pipeline`, `Sequence`, `And`, `Or`, `Subshell`, `Redirect`, `Assignment`, `FunctionDef`

### 9.3 Builtins

`cd`, `pwd`, `export`, `unset`, `alias`, `unalias`, `set`, `exit`, `jobs`, `fg`, `bg`, `kill`, `history`, `source`

## 10. Terminal (RacTerm)

### 10.1 Architecture

```
Input Decoding → Escape Sequence Parser → Screen Buffer → Style/Cursor State → Renderer
                                                                        ↕
                                                                  PTY Session
```

### 10.2 Capabilities

- ANSI colors (16 + 256 + truecolor)
- Cursor movement / positioning
- Clear screen / clear line
- Insert / delete line
- Scroll regions
- Alternate screen buffer
- UTF-8 text
- Resize handling (SIGWINCH-like)

### 10.3 Performance Constraints

- Dirty region rendering (only repaint changed areas)
- Scrollback: ring buffer with configurable limit (default 10,000 lines)
- Bounded memory growth under high-throughput output

## 11. Package System (rpkg + rapt)

### 11.1 Two-Layer Architecture

```
┌───────────────────────────┐
│  rapt (high-level)        │  repositories, dependency resolution,
│                           │  channels, signatures, updates
├───────────────────────────┤
│  rpkg (low-level)         │  install/remove files, database,
│                           │  hooks, verify, rollback metadata
└───────────────────────────┘
```

### 11.2 Package Format

Custom archive format (`.rpk`) containing:
- `MANIFEST` — name, version, arch, description, dependencies, conflicts
- `CHECKSUMS` — SHA-256 per file
- `SIGNATURE` — ed25519 signature
- `hooks/` — pre-install, post-install, pre-remove, post-remove scripts
- `data/` — filesystem payload

### 11.3 Repository Channels

| Channel | Purpose |
|---------|---------|
| stable | Production releases |
| testing | Pre-release validation |
| dev | Development builds |

## 12. Security Baseline

### 12.1 Principles

1. **Deny by default** where possible
2. **Least privilege** for all services
3. **Signed artifacts** for packages and images
4. **Capability separation** between services
5. **Reproducible builds** (target)

### 12.2 Mechanisms

- User/group model with ownership and permissions
- Capability bits per process
- Mount flags (noexec, nosuid, ro)
- Signed package verification (ed25519)
- Kernel panic policy (configurable: halt, reboot, dump)
- Crash dump sanitization (strip secrets before write)

## 13. Excluded from v1.0

- GUI desktop environment
- Full glibc/Linux userspace compatibility
- Docker-level containerization
- Broad hardware driver support
- SMP tuning for all architectures
- ARM, RISC-V, or additional architectures
- Browser-based GUI stack
- Real-time scheduling

## 14. Roadmap

| Milestone | Scope |
|-----------|-------|
| **MVP** | Boot + kernel + memory + scheduler + syscalls + first user process |
| **Alpha** | VFS + init + TTY + basic shell |
| **Beta** | Full shell + terminal + package system |
| **RC** | Security hardening + observability + release tests |
| **1.0** | Production-ready with full test coverage and release engineering |

## 15. Cross-Cutting Concerns

### 15.1 Logging

- Kernel: ring buffer, serial output, structured log entries
- Userland: structured JSON logs via libc-lite
- RacInit: log routing to journal (file-based)

### 15.2 Error Handling

- Kernel: `Result<T, KernelError>` everywhere, panic only for unrecoverable state
- Userland: errno-style via syscall return values
- Services: exit codes + restart policies

### 15.3 Testing Strategy

| Level | Scope | Tool |
|-------|-------|------|
| Unit | Individual functions/modules | `cargo test`, custom C test harness |
| Integration | Multi-module interaction | Custom test runner |
| Boot | QEMU boot → serial output validation | `scripts/boot-test.sh` |
| E2E | Full system scenarios | QEMU + expect-like automation |
| Fuzz | Shell parser, terminal escape parser | `cargo-fuzz` or `afl` |
| Fault injection | Service manager, IPC | Custom fault injector |
