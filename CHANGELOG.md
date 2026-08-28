# Changelog

All notable changes to RacOS are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 entries describe development progress relative to the tiered plan in
[`docs/ROADMAP.md`](docs/ROADMAP.md). Each entry references the merged PRs and
the architectural sub-task IDs (T1.x, T2.x, …) that motivated it.

## [Unreleased]

### Added — v0.2 §2.1 (coreutils)

`free`, `rmdir`, `du` and `clear` ship, closing all of §2.1 except `ln`.
Smoke `T21-COREUTILS-OK`.

- **`free`** — parses `/proc/meminfo`, which already existed, so no procfs
  work was needed. `-k` (default) / `-m`. Fields are looked up **by name**
  rather than line position: procfs is free to add fields, and a positional
  parser would start reporting the wrong numbers the day it does.
- **`rmdir`** — emptiness is enforced by racfs, whose `unlink` refuses a
  directory that still has entries. Re-checking in userland would be a race
  and a lie, so this reports what the kernel says. It does `stat` first only
  to refuse a regular file with an accurate message.
- **`du`** — `-s` summarise, `-b` bytes, default 1 KiB blocks rounded up.
  Reports apparent size (`st_size`); racfs has no sparse files, so that
  equals allocated size today. Recursion is bounded at depth 32 — a directory
  cycle becomes possible the moment hard links exist, and a shell tool has no
  business taking the process down.
- **`clear`** — not in the original plan, but its absence reads as a broken
  system even though Ctrl-L was always bound. Emits ED 2 + CUP in a single
  write: RacTerm and the framebuffer console parse CSI per write, so a split
  sequence prints its tail literally.

`ln` is **not** shipped and is blocked on the kernel rather than userland:
`sys_link` is a stub returning `ENOSYS`, and hard links need racfs support
(inode link count, two dirents for one inode, unlink decrementing rather than
freeing). That belongs in its own change with its own tests.

### Fixed

- **Tab completion could not see mountpoints.** Completion listed a directory
  with `getdents`, but `/proc`, `/dev`, `/tmp`, `/mnt` and friends are entries
  in the kernel's mount table, not directory entries — `/` in the initramfs
  carries only `bin`, `etc`, `sbin`. So `/pro<Tab>` found nothing. Candidates
  now fold in `/proc/mounts`, filtered to the mounts whose parent is the
  directory being completed. Found by exercising the running guest.
- **`build-image.sh` had drifted from `build-image.ps1`.** The bash script,
  which CI uses on Linux, was missing `dig`, `wget`, `mount`, `df`, `umount`,
  `sync` and both `mkfs.*` tools — so CI built a smaller image than a local
  Windows build, and tested something subtly different from what ships. Both
  lists are back in sync, and the bash copy gained the `src=dst` rename
  handling its PowerShell counterpart already had (cargo cannot emit a bin
  name containing a dot, hence `mkfs_racfs` → `mkfs.racfs`).

### Added — v0.2 §2.2 (racsh UX)

The shell half of the "usable shell" milestone. `docs/ROADMAP.md` §2.2 is now
closed; §2.1 (`free`, `ln`, `rmdir`, `du`) and §2.3 (network tools) remain.

- **Aliases.** `alias`, `alias NAME`, `alias NAME=VALUE`, `unalias NAME...`
  and `unalias -a`, stored sorted in `Env::aliases` and carried into command
  substitution. Expansion happens at execution time in `exec_simple` rather
  than at parse time as originally sketched — the parser has no `Env`, and
  the command word is already resolved by then. A name expands at most once
  per command, so the idiomatic `alias ls='ls --color'` terminates instead of
  looping. Replacements split on whitespace, so quoting inside an alias body
  does not survive; use a shell function for that. Smoke `T22-ALIAS-OK`.
- **Tab completion** (`shell/src/complete.rs`). Command position offers
  builtins plus every executable on `$PATH`; elsewhere it completes paths. A
  word containing `/` is always path-completed, so `./x` and `/bin/l` behave
  as they read. One match is inserted with a trailing space; several extend
  the word by their common prefix; an ambiguous word that gains no prefix
  prints the candidates and redraws the line. The decision logic and prefix
  arithmetic are pure functions, host-tested; only candidate gathering
  touches the filesystem.
- **Persistent history.** `History::load_file` / `save_file`, capped at 1000
  entries and rewritten in full after each line so the cap holds without a
  seek. Path is `$HOME/.racsh_history`, falling back to
  `/var/.racsh_history` because `/` is the read-only initramfs. Surviving a
  reboot still waits on v0.3 §3.3. `history` and `history -c` are handled in
  the REPL loop, not `racsh::builtin`, because the list is session state
  `exec_simple` cannot reach — so they do not work inside a pipeline.
- **Prompt escapes** (`shell/src/prompt.rs`). `\u`, `\h`, `\w`, `\W`, `\$`,
  `\n`, `\\`. `\w` abbreviates `$HOME` to `~` on whole path components only,
  so `/home/adam` is not mangled when HOME is `/home/ada`. An unknown escape
  is emitted verbatim rather than swallowed, so a typo in PS1 is visible.

26 new host tests — racsh 28 → 54, workspace 81 → 107 — plus the in-guest
`T22-ALIAS-OK` smoke.

### Added

- **SIGUSR1 (10) / SIGUSR2 (12)** in the kernel's `Signal` enum and
  `Signal::from_u8`. They carry no kernel semantics (default action is
  Terminate, via the existing catch-all) but must exist so `sys_kill`
  accepts what `sys_sigaction` already allowed.
- **awk END-block smoke** re-enabled in `racos-test` (`T33-AWK-OK` now
  covers five cases instead of four). It was dropped while the argv
  corruption below made any `$(...)` script fail intermittently.

### Fixed

- **A corrupt racfs directory entry halted the kernel.**
  `direntry_from_bytes` clamped how many name bytes it *copied* out of a disk
  sector but stored the raw, unclamped `name_len` in the returned struct. Disk
  content is untrusted — a torn write or stale slot can put anything in that
  byte — so all three consumers (`dir_lookup`, `dir_remove_entry`, `readdir`)
  could slice `name[..name_len]` past the 56-byte array and take the whole
  system down: `KERNEL PANIC at racfs.rs:687, range end index 111 out of range
  for slice of length 56`, triggered by nothing more exotic than
  `test -f /mnt/notes.txt`. The decoder now clamps `name_len` itself, so the
  invariant `name_len <= name.len()` holds for every in-memory entry and all
  consumers are safe by construction. `dir_add_entry` likewise records what
  actually fits instead of `name.len() as u8`, which wrapped past 255.
  Verified by replaying the exact failing sequence against the damaged image:
  the guest now boots, reports the bad entries as filesystem errors, and stays
  alive.
- **`readdir` listed unusable directory slots.** An entry with a zero-length
  name, or one whose inode cannot be read, is now skipped instead of being
  emitted as a blank row or aborting the whole listing on `?`.
- **LAPIC timer IRQ corrupted the syscall frame pointer (GS-base aliasing).**
  `lapic_timer_handler` did `inc qword ptr gs:[16]` to bump its per-CPU
  `tick_count`. But `IA32_GS_BASE` only points at a `PerCpu` slot on the
  idle/boot path — during a syscall `swapgs` makes it
  `&syscall::entry::PER_CPU`, whose offset 16 is `syscall_frame_ptr`, and in
  user mode `enter_ring3` sets it to 0. So a timer tick taken mid-syscall
  incremented the saved syscall-frame pointer, and a tick taken in user mode
  wrote to absolute address `0x10`.
  This is the root cause of the long-standing flaky
  `try_deliver_user_handler:510` "misaligned pointer dereference" panic
  tracked in ROADMAP §6 — the pointer wasn't *unaligned*, it was a tick
  counter. When the corrupted value happened to be 8-aligned the panic did
  not fire and the kernel patched an arbitrary address instead.
  The handler now finds its slot with `percpu::peek(lapic::current_apic_id())`,
  which is correct in all three GS states, and `OFFSET_TICK_COUNT` is gone so
  the pattern can't be reintroduced. Both `try_deliver_user_handler` and
  `sys_sigreturn` additionally reject a non-8-aligned frame pointer instead
  of dereferencing it.
- **argv/envp corruption in `prepare_user_stack`** (`kernel/src/task/
  process.rs`). The envp pointer array was written one slot too high:
  entries went to `rsp + 8 + 8*(argc+1+i+1)` and the NULL terminator to
  `rsp + 8 + 8*(argc+1+envc+1)`, while libc-lite's `_start` reads
  `envp = argv + (argc+1)*8`. Two consequences:
  - userland saw an uninitialised slot as `envp[0]`, so no process ever
    received an environment (`/bin/env` printed nothing, `T33-ENV-OK`
    never fired);
  - the NULL write landed one slot *past* the reserved block, on top of
    the argv string data stacked directly above it. Whether it corrupted
    anything depended on the 16-byte alignment gap, i.e. on total argv
    length — which is what made `sh -c` fail as `sh: cannot open script:`
    for some scripts and not others. This was the long-standing
    ROADMAP §6 "racsh `$(...)` edge case"; it was never a racsh bug.
- **`sys_kill` rejected SIGUSR1/SIGUSR2 that `sys_sigaction` accepted.**
  `sys_sigaction` validates only `1..=31` minus SIGKILL/SIGSTOP, so a
  handler could be installed for signal 10, but `Signal::from_u8(10)`
  returned `None` and `sys_kill` answered `EINVAL`. Unblocks
  `PHASE21-USER-HANDLER-REENTRANT-OK`.
- **`grep` matched a prefix, not a substring.** `simple_regex_match`
  (renamed `match_at`) anchors at offset 0 and returns on the first
  literal mismatch without retrying later offsets, so `grep ma` missed
  `ala ma kota` while `grep ala` matched. `buf_contains` now scans every
  starting offset, which is what its name always promised.
- **`grep` dropped a final line with no trailing newline.** The line was
  only tested when a `\n` arrived, so an unterminated last line was
  silently discarded.
- **racsh rejected reserved words used as `case` patterns.**
  `case $x in done) ...;; esac` failed with `parse error: Expected word, got
  Done`. POSIX only recognises reserved words in command-word position;
  elsewhere — notably a case pattern — they are ordinary words. The parser
  now maps keyword tokens back to literals via a dedicated
  `parse_pattern_word`, kept deliberately narrow so a `for` word list still
  terminates at `do`. Covered by two new host tests
  (`case_patterns_accept_reserved_words`,
  `reserved_words_still_reserved_in_command_position`).

With these, in-guest `racos-test` goes from **120 passed / 9 failed** to
**130 passed / 0 failed**, and all 20 smoke markers fire.

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
