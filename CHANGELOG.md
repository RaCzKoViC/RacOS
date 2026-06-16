# Changelog

All notable changes to RacOS are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 entries describe development progress relative to the tiered plan in
[`docs/ROADMAP.md`](docs/ROADMAP.md). Each entry references the merged PRs and
the architectural sub-task IDs (T1.x, T2.x, …) that motivated it.

## [Unreleased]

### Added
- Nothing yet.

## [0.1.0] — 2026-06-16

First public development milestone. Two largest backlog items from §Decision
in the ADRs (the unsafe-block audit and the userland stub set) are closed; the
shell scripting and service manager foundations are in place; the network
stack is end-to-end on IPv4; persistence works across reboots via AHCI; the
build cross-compiles on Ubuntu / Windows / macOS.

### Added — Tier 1 (chokepoints)

- **T1.1 — User-mode signal delivery** (PR #6). `sys_sigaction` installs a
  cooked handler, the kernel pushes a `UserSignalFrame` on the user stack and
  redirects RIP to libc-lite's `__signal_dispatcher`; `sys_sigreturn` restores
  the pre-signal context. Covered by `racos-test::PHASE21-USER-HANDLER-OK`
  and `PHASE21-USER-HANDLER-REENTRANT-OK`.
- **T1.2 — Shell scripting in racsh**. Runtime for `if`/`while`/`for`/`case`,
  parameter expansion (`$?`, `$0..$9`, `${VAR:-default}`, `${#VAR}`, …),
  `source` / `.` builtin, `sh script.sh` from a file, IFS-based field
  splitting. Smoke `T12-SHELL-CONTROL-FLOW-OK`.
- **T1.3 — RacInit engine wired to PID 1**. Unit file parser
  (`.service`/`.target`/`.timer`/`.mount`/`.device`), Kahn topological sort
  with cycle detection, restart policies plus 5-in-30s burst limit. Default
  unit files `base.target` + `shell.service` ship in the initramfs. Falls
  back to the legacy spawn-shell loop when no unit files are present. 13
  host tests plus the `T13-INIT-ENGINE-OK` QEMU smoke.

### Added — Tier 2 (developable OS)

- **T2.1 — Persistence in CI**. boot-smoke now attaches a 16 MiB AHCI disk
  (`ich9-ahci`) and boots QEMU twice — first boot formats the disk and writes
  a `boot-counter`, second boot reads it back as 1 and increments to 2.
  `kernel/src/main.rs` mounts racfs at `/mnt`. `flushd` writeback daemon
  flushes dirty block-cache entries on every block-backed mount.
- **T2.2 — RacTerm ANSI emulator** (already implemented at 1616 lines pre-
  T2.2): 31 host tests in `terminal/tests/ansi.rs` cover the CSI parser,
  cursor movement, SGR colours, alternate buffer, DECTCEM, DECSTBM, DSR,
  scrollback, OSC 0 title. Fixed: `Terminal::drain_response()` is now called
  in racterm's main PTY loop so DSR / DA replies actually reach the shell.
- **T2.3 — Cross-platform build**. `scripts/run-ci-smoke.sh` (bash port of
  the PS smoke runner), `just smoke` / `just smoke-disk` recipes routed by
  `[unix]` / `[windows]` attributes, `docs/DEVELOPMENT_LINUX.md`.

### Added — Tier 3 (toward v1.0)

- **T3.1 — SMP**. AP bring-up via INIT-SIPI-SIPI from `arch::ap::bring_up_all`
  with a real-mode → protected → long-mode trampoline. Each AP loads the
  kernel GDT/IDT, enables its LAPIC, binds GS to its PerCpu slot, starts its
  own LAPIC timer. `/proc/cpuinfo` enumerates online CPUs.
  CI runs QEMU with `-smp 4` and grep-gates the enumeration logs.
- **T3.2 — rpkg MVP**. End-to-end install/list/remove CLI in
  `pkg/rpkg-bin/`. Writes to `/var/lib/rpkg/info/<name>/`; tracks installed
  files for clean removal. Smoke `T32-RPKG-OK`.
- **T3.3 — Userland: ps + sed + env + awk**. Each binary rewritten from a
  dead-code stub (wrong libc-lite path, missing `alloc` feature, not in
  workspace, missing from BIN_LIST) into a working MVP with a racos-test
  smoke marker: `T33-PS-OK`, `T33-SED-OK`, `T33-ENV-OK`, `T33-AWK-OK`.
  awk supports `BEGIN`/`{}`/`END` blocks, `$0..$N` fields, `-F` separator,
  literal strings, comma-separated print items.

### Added — Tier 4 (strategic)

- **T4.2 — Unsafe-block audit (BACKLOG COMPLETE)**. 391 `// SAFETY:`
  annotations added across 12 sweep PRs (#18 – #29) reducing the missing
  count from 350 → 0. Every `unsafe {` block under `kernel/` and
  `libs/libc-lite/` now has a `// SAFETY:` comment in the preceding 5 lines.
  `scripts/check-unsafe-safety.sh --strict` passes clean. CI gate
  `Unsafe-safety annotation lint (--strict)` was promoted from advisory to
  required in PR #31.
- **T4.3 — ADR/spec resync, second pass**. Implementation-status sections
  added to ADR-009 (VFS — 6 filesystems mounted, racsysfs collapsed into
  procfs), ADR-011 (init/service manager — engine shipped, servicectl
  deferred), ADR-013 (logging — serial shipped, journal deferred), ADR-018
  (repository signing — package format shipped, signing waits on T4.1
  crypto), ADR-019 (security baseline — DAC + caps + SMEP/SMAP + NX
  shipped; ASLR + seccomp + secure-boot deferred). First pass covered
  ARCHITECTURE.md §1.3 + ADRs 003/006/007/008.

### Changed

- `kernel/src/main.rs`'s crate-level discipline comment was rephrased to
  stop triggering a false positive in the `check-unsafe-safety.sh` regex.
- `racterm` and `init` were added to the host-side `cargo test` set so the
  31 ANSI tests + 13 engine tests run on every PR.

### Fixed

- racterm's PTY relay never called `Terminal::drain_response()`, so DSR
  (`\e[6n`) and DA (`\e[c`) replies queued in the emulator never reached the
  shell. ncurses-style apps that waited for the reply would hang.
- `RacInit::resolve_start_order` had a buggy Kahn's-algorithm implementation
  that lost edges across iterations; rewritten with a proper in-degree
  decrement and a separate cycle-detection pass.
- `/bin/ps` looped `getdents` expecting a cursor that the kernel API
  doesn't have, returning duplicate rows. Single-call read fixes it.
- The userland stub binaries (`ps`, `sed`, `env`, `awk`) all had the same
  four wiring bugs (libc-lite path, `alloc` feature, workspace member, two
  BIN_LIST entries in the build scripts). Each was fixed in its T3.3
  rewrite PR.
- AHCI persistence in boot-smoke had two earlier flakes: `-drive if=ide`
  didn't surface as `sda` on q35, and `fat:rw:esp` auto-FAT occasionally
  produced an unbootable ESP on the second QEMU run. Now uses an explicit
  `ich9-ahci` controller and a pre-baked 256 MiB FAT32 ESP image.
- LAPIC MMIO register access was unaligned; switched to `read_volatile` /
  `write_volatile` at the documented 16-byte register offsets.

### Security

- The unsafe-block audit (T4.2) makes every cross-ring memory access in
  the kernel inspectable: the `// SAFETY:` comment names the invariant
  (kernel singleton, cli/sti window, validate_user_ptr-bounded, etc.) so
  reviewers don't have to reconstruct it from the surrounding code.
- New CI gate `Unsafe-safety annotation lint (--strict)` blocks PRs that
  add `unsafe {` without a SAFETY note in the preceding 5 lines.
- `sys_reboot` is gated on `CAP_SYS_BOOT`; `sys_mount` / `sys_umount` /
  `sys_mkfs` on `CAP_SYS_ADMIN`; `sys_chown` on `CAP_CHOWN`; `sys_setuid`
  / `sys_setgid` consult `CAP_SETUID` / `CAP_SETGID` (verified by
  `racos-test::test_security_syscalls`).

[Unreleased]: https://github.com/RaCzKoViC/RacOS/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/RaCzKoViC/RacOS/releases/tag/v0.1.0
