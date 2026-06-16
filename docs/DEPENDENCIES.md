# Dependency Inventory

Snapshot of every third-party crate that ends up in a shipped RacOS
artifact. Updated whenever a new dependency is added.

> Run `cargo tree --workspace --depth 2` for the live graph.

## Summary

| Component | External crates | Notes |
|---|---|---|
| `racore` (kernel) | **0** | Uses only `core` + `alloc` rebuilt by `-Z build-std`. `compiler_builtins` is provided by build-std. |
| `racos-boot` (UEFI bootloader) | `uefi` 0.34, `log` 0.4 | UEFI services + the `log`-crate facade. |
| `libs/libc-lite` (userland system library) | **0** | Pure `#![no_std]` inline-asm syscall wrappers. |
| `libs/libcore-user` (userland helpers) | **0** | Pure `#![no_std]`. |
| `userland/coreutils/*` | **0** (via `libc-lite`) | Every binary depends only on `libc-lite`. |
| `userland/network/*` | **0** (via `libc-lite`) | Same. |
| `shell` (racsh) | **0** | Pure `#![no_std]`. |
| `terminal` (racterm) | **0** | Pure `#![no_std]` ANSI emulator. |
| `init` (RacInit engine) | **0** | Pure `#![no_std]`. |
| `pkg/rpkg` (package format lib) | **0** | Pure `#![no_std]`. |
| `pkg/rpkg-bin` (`/bin/rpkg`) | **0** (via `libc-lite`) | |
| `pkg/rapt` (host-side dep planner) | **0** | Pure host `std` Rust, no third-party. |

**Total third-party crates shipped on the target: 2** (`uefi`, `log`).
Both live only in the bootloader; the kernel and every userland
artifact link zero external code.

## Bootloader-only deps

### `uefi` 0.34
- Source: <https://github.com/rust-osdev/uefi-rs>
- License: MPL-2.0
- Why: UEFI services (`BootServices`, `RuntimeServices`, memory map,
  graphics output protocol, file system protocol). Without it the
  bootloader would have to hand-roll the UEFI calling convention and
  GUID tables.
- Features enabled: `alloc`, `global_allocator`.
- Audit notes: the crate is maintained by rust-osdev, has an active
  release cycle, and pins itself to UEFI 2.10. No `unsafe-impl-Send`
  type smuggling in the version we link against (0.34).

### `log` 0.4
- Source: <https://github.com/rust-lang/log>
- License: MIT OR Apache-2.0
- Why: `uefi` 0.34 expects a `log` facade installed for its boot-services
  diagnostics. We provide one that writes to serial via the
  `simple_logger`-style adapter in `boot/src/`.
- Audit notes: maintained by rust-lang. Zero internal `unsafe` in the
  0.4 line; the version range is conservative (`= "0.4"`).

## What we do not depend on

Worth calling out because each of these is a typical kernel/OS
dependency we explicitly avoided:

- **`libc`** — replaced by `libs/libc-lite`, the in-tree no-std crate of
  syscall wrappers. Doing it ourselves was easier than maintaining a
  `libc` fork without `mmap`, `dlopen`, `__libc_start_main`, or the C
  runtime's hidden global state.
- **`spin` / `parking_lot`** — `kernel/src/sync.rs` is an in-tree
  `SpinLock<T>` that fits the kernel's `cli/sti` invariants. No third
  party.
- **`bitflags`** — the kernel uses `const` bitmasks (page-table flags,
  capability bits, FAT32 dir entries) without a macro layer.
- **`linked_list_allocator` / `slab_allocator`** — `kernel/src/mm/heap.rs`
  is a hand-rolled first-fit free-list allocator with coalescing. No
  third party.
- **`serde` / `serde_json` / `toml` (kernel side)** — the kernel uses
  ad-hoc parsers for `procfs` output and `racfs` superblocks. Userland
  `racinit` and `rpkg` use a 200-line in-tree TOML subset parser, not
  the full `toml` crate.
- **`x86_64` / `bootloader` / `multiboot2` / `volatile`** — kernel uses
  raw inline asm, raw MMIO via `read_volatile`/`write_volatile`, and a
  custom UEFI handoff convention with `racos-boot`. No third party.
- **`async-std` / `tokio` / `embassy`** — no async runtime; the
  scheduler runs synchronous kernel tasks.
- **`crypto-*`** — there is no crypto in tree yet. T4.1 will bring in
  ChaCha20-Poly1305, X25519, Ed25519, SHA-256, HKDF, and TLS 1.3
  handshake code. Whether that lives in-tree or pulls a vetted crate
  is an open ADR; the current preference (driven by the no-third-party
  baseline) is in-tree from scratch.

## Host-side tooling

These are deps of `cargo test` on the host, not anything that ships:

- `cargo` 1.x (workspace target builds)
- `nightly-2026-05-21` pinned by `rust-toolchain.toml`
- `nasm` (assembler for the AP trampoline)
- `qemu-system-x86_64` + `OVMF` (CI smoke targets)
- `mtools` + `python3` (ESP image staging on CI)

None of these are linked into a shipped artifact.

## Adding a new dependency

Before adding a `Cargo.toml` `[dependencies]` entry:

1. Check whether the in-tree approach is ~100 lines of Rust. If yes,
   prefer in-tree (see the §"What we do not depend on" list for why).
2. If a third-party crate is genuinely the right call (UEFI services,
   future crypto), add a row to the §Summary table and a paragraph in
   §Bootloader-only deps (or a new section).
3. Note the crate's license, source URL, and whether it pulls in
   transitive `unsafe`-heavy code (run `cargo geiger` if in doubt).
4. Pin the version (`= "x.y"` rather than `^x.y`) — we'd rather take an
   explicit upgrade PR than absorb a transitive bump silently.
