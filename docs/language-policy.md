# RacOS — Language & Toolchain Policy

> Status: enforced (toolchain pin landed 2026-05-23)
> Owner: project maintainer
> Applies to: every crate in this workspace

## 1. Primary language

Rust is the **primary implementation language** for every layer of RacOS:

| Layer                | Language                              | Crate(s)                                 |
|----------------------|---------------------------------------|------------------------------------------|
| Bootloader           | Rust `x86_64-unknown-uefi`            | `boot`                                   |
| Kernel               | Rust `#![no_std]` + x86_64 assembly   | `racore` (kernel)                        |
| Userland tools       | Rust `#![no_std]` over `libc-lite`    | `userland/coreutils/*`, `userland/network/*` |
| Shell, terminal, init| Rust `#![no_std]`                     | `shell`, `terminal`, `init`              |
| Package manager      | Rust                                  | `pkg/rpkg`, `pkg/rapt`                   |
| C ABI surface        | Rust + a thin C ABI                   | `libs/libc-lite`                         |

The kernel **is not** being or going to be ported to C or C++. C remains an
option only for:

- ABI conformance tests against `libc-lite`,
- future userland ports of existing C software that goes through the
  POSIX-like compatibility layer (planned, not yet implemented),
- third-party drivers integrated via FFI (none today).

C++ is not a supported implementation language anywhere in this repository.

## 2. Assembly

Assembly is permitted **only** in architecture-specific paths and must be
isolated behind a Rust surface. Today this is:

- `kernel/src/arch/mod.rs::_start` — kernel entry trampoline
- `kernel/src/arch/ap.rs` — application-processor real→protected→long
  mode trampoline (Phase G.3)
- `kernel/src/arch/context.rs` — context switch (saves callee-saved +
  RFLAGS, including AC for SMAP)
- `kernel/src/syscall/entry.rs` — SYSCALL/SYSRETQ stub with STAC/CLAC
- `kernel/src/task/process.rs::user_entry_trampoline` — first IRETQ
  into ring 3

New assembly should not leak outside of `kernel/src/arch/**` or its
syscall/task counterparts. If you need assembly elsewhere, audit whether
the same job can be done with `core::arch::asm!` inside a documented
unsafe block, or whether it actually belongs in `arch/`.

## 3. Toolchain pinning

`rust-toolchain.toml` pins a dated nightly (e.g. `nightly-2026-05-21`).
This is **load-bearing**:

- The kernel's `core::arch::asm!`, `naked_asm!`, and `extern "x86-interrupt"`
  ABIs have changed across nightlies before in ways that silently broke
  our IDT handlers.
- `core::ptr::read::precondition_check` UD2 stubs got noisier on a recent
  nightly and forced us to compile userland with
  `-C debug-assertions=off` (see `scripts/build-image.ps1`).
- The kernel-side `RUSTFLAGS="-C relocation-model=static -C link-arg=-no-pie"`
  is required because the bootloader doesn't apply RIP-relative
  relocations.

Bumping the pin is a chore, not a no-op. Required steps before a bump:

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets` (some targets need `--target x86_64-unknown-none`)
3. `cargo check --workspace`
4. Rebuild kernel + userland: `powershell -File scripts/build-image.ps1`
5. ci-smoke green: `powershell -File scripts/run-ci-smoke.ps1 -Disk -Smp 4`
6. Interactive QEMU sanity: `scripts/launch-interactive.ps1`, type `ls /mnt`,
   `echo > /mnt/x`, `cat /mnt/x`, `dig example.com`.

## 4. Unsafe Rust policy

Unsafe Rust is allowed **only at hardware/ABI/memory/allocator boundaries**
and must be documented. Concretely:

### 4.1 Where unsafe is expected

- Anything in `kernel/src/arch/**` — direct CPU/MSR/MMIO access.
- `kernel/src/syscall/**` — touches user pointers, SYSCALL/SYSRETQ asm.
- `kernel/src/task/{context,process,scheduler}.rs` — context switch,
  TSS, kernel stack management.
- `kernel/src/mm/**` — page table walks, physical allocator, raw frame
  pointers.
- `kernel/src/drivers/**` — PCI BAR I/O, AHCI DMA, virtio queues, PS/2,
  serial, framebuffer MMIO.
- `kernel/src/interrupts/**` — IDT handler installation, EOI writes.
- `boot/**` — UEFI boot services + manual ELF load.
- `libs/libc-lite/src/lib.rs` — raw `syscall` instruction wrappers.
- `shell/src/exec.rs` — argv pointer-table assembly.

### 4.2 Where unsafe is forbidden

The following crates carry `#![forbid(unsafe_code)]` and must stay pure:

- `pkg/rpkg`
- `pkg/rapt`
- `libs/libcore-user`

The following userland binaries carry `#![deny(unsafe_code)]` with a
single `#[allow(unsafe_code)]` on the `#[no_mangle] extern "C" fn main`
entry point (the only ABI-facing item):

- `userland/coreutils/{true,false,df,env,id,mount,sync,sh,init}`

This list is intentionally conservative — adding new entries requires
verifying the crate has **zero** other unsafe blocks, unsafe impls,
extern blocks, `global_asm!`, `asm!`, or `link_section`.

### 4.3 Unsafe style — `unsafe_op_in_unsafe_fn`

Workspace-wide lint config in the root `Cargo.toml`:

```toml
[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "deny"
```

This is the Rust 2024-edition default backported to our 2021-edition
workspace: every unsafe op must sit in its own `unsafe { ... }` block,
even inside an `unsafe fn`. Each new unsafe block must carry a
`// SAFETY:` comment explaining the invariant it relies on:

```rust
// SAFETY: COM1 I/O ports are owned by the serial driver; we hold a
// &mut self so no other thread is contending. write_byte spins until
// THRE is set, so no transmit clash.
unsafe { crate::arch::outb(COM1_PORT, byte); }
```

If the invariant isn't obvious, write a TODO note instead of inventing
one:

```rust
// SAFETY: TODO(racos): document the exact invariant for ...
```

The kernel crate (`racore`) opts out of the lint at crate level today
(`#![allow(unsafe_op_in_unsafe_fn)]`) because it has 239 historical
sites that pre-date this policy. Migrating each subsystem is tracked
piecemeal; **new** subsystems and any unsafe block you touch should
follow the per-op style.

## 5. Dependency policy

- The kernel takes **no** external crates beyond what `core` and `alloc`
  provide via `-Z build-std`. `compiler_builtins` is provided by
  build-std. Adding any other dep to `kernel/Cargo.toml` requires a
  justification commit message paragraph.
- `boot` may depend on `uefi`, `log`, and minimal helpers — no further
  growth without explicit review.
- `libs/libc-lite` is the single source of syscall wrappers for
  userland; userland binaries should not call `syscall` directly.
- No `std`-dependent crate may be added to any `#![no_std]` member.

## 6. Testing & CI checks

- `cargo fmt --check` — enforced (advisory job today; aim for required).
- `cargo clippy --workspace --all-targets` — advisory.
- Kernel `ci-smoke` feature — required (asserts VFS topology, racfs/FAT32
  round-trip, /mnt persistence, per-CPU LAPIC ticks, SMP bring-up).
- Interactive shell smoke (TCP serial) — required.
- Boot smoke (UEFI) — required.

When a change touches any unsafe boundary, the commit message should
state which invariant changed and which test exercises it.
