# RacOS — Development Roadmap

> Status: Living document
> Created: 2026-06-16
> Last updated: 2026-06-16 (post-v0.1.0, milestone framing adopted)

This is the source of truth for project direction. Every PR with a meaningful
scope change should also touch this file.

The plan is organised around **release milestones** that ship user-visible
value: **v0.2** (usable shell), **v0.3** (persistent storage), **v0.4**
(graphics base). Once those land, several long-running tracks run in
parallel (networking hardening, SMP scheduler refactor, packaging maturity,
TLS crypto).

The bootstrap-phase tiered plan (Tier 1-4) closed with **v0.1.0**. Detailed
historical context lives in [`CHANGELOG.md`](../CHANGELOG.md) and in the
ADR implementation-status sections under [`docs/adr/`](adr/).

---

## 1. v0.1.0 — Completed cycle (2026-06-16)

The bootstrap phase closed all four tiers from the original tiered plan:

- **Tier 1** (chokepoints) — user-mode signals (T1.1), racsh scripting (T1.2),
  RacInit engine wired to PID 1 (T1.3).
- **Tier 2** (demo → developable) — persistence in CI with two-boot AHCI smoke
  (T2.1), RacTerm ANSI emulator + 31 host tests (T2.2), cross-platform build
  smoke (T2.3).
- **Tier 3** (toward v1.0) — SMP AP bring-up + LAPIC timers (T3.1 partial),
  rpkg install/list/remove (T3.2), userland stubs filled (ps + sed + env +
  awk for T3.3).
- **Tier 4** (strategic) — unsafe-block audit backlog cleared 350 → 0 with
  `--strict` lint as a required CI gate (T4.2), ADR/spec resynced (T4.3).
  T4.1 TLS deferred to the post-v0.4 parallel track.

Plus the post-v0.1.0 quality sprint: `SECURITY.md`, `docs/DEPENDENCIES.md`
(zero kernel deps, 2 bootloader deps), advisory CI for coverage +
cargo-audit, stale-PR cleanup.

For per-subsystem status see the ADR `Implementation status (2026-06-16)`
sections and the `[0.1.0]` entry in [`CHANGELOG.md`](../CHANGELOG.md).

---

## 2. v0.2 — Usable shell (next milestone)

> **Goal**: RacOS becomes pleasant to drive from a terminal. Boot the guest,
> log in, write a small script, install something, see it work, log out.
> Today you *can* do most of this but the UX rough edges turn casual
> exploration into a chore.

### 2.1 Coreutils gap-filling

Many of the conventional Unix tools are missing, three more exist as crates
but never made it into `BIN_LIST` / initramfs.

**Easy wins** (crate exists, just plumb into workspace + build-image
BIN_LIST, write smoke):

| Tool | Status | Smoke marker |
|---|---|---|
| `id` | crate has `src/`, not shipping | `T20-ID-OK` |
| `sort` | crate has `src/`, not shipping | `T20-SORT-OK` |
| `top` | crate skeleton only (no src) | `T20-TOP-OK` after MVP rewrite |

**Net-new**:

| Tool | DoD |
|---|---|
| `touch` | create empty file or update mtime |
| `chmod` | accept `0644`-style octal + symbolic `u+x`; `sys_chmod` exists |
| `chown` | `chown user:group path`; `sys_chown` exists |
| `kill` | `kill -SIG pid`, default TERM; `sys_kill` exists |
| `whoami` | print euid → username (needs `/etc/passwd` lookup) |
| `uname` | `-a` / `-r` / `-m` / `-s`; `sys_uname` exists |
| `free` | parse a new `/proc/meminfo` (procfs entry to add) |
| `ln` | hard links via `sys_link`; symlinks deferred until `sys_symlink` |
| `rmdir` | explicit standalone (not just `rm -d`) |
| `du` | recursive walk + size aggregation |

`pwd` and `cd` are already racsh builtins; standalone `/bin/pwd` is
conventional but not required for v0.2.

### 2.2 racsh UX

- **Persistent history** — `~/.racsh_history`, read at startup, append on
  each line, cap at ~1000 entries.
- **Tab completion** — minimum scope: command names from `$PATH`,
  file paths from the current directory. Matches the existing
  character-mode line editor in `shell/src/readline.rs` (no readline
  dependency).
- **Aliases** — `alias ll='ls -la'`. Expansion happens at parse time;
  stored in `Env::aliases`.
- **Prompt expansions** — `${PS1}` already parses but `\u`, `\h`, `\w`
  substitutions don't happen. Add the basic set.
- **`$(...)` command-substitution edge case** — the one found during the
  awk T3.3 smoke (`'END { ... }'` inside `$(...)` fails with `sh: cannot
  open script:` status 127, even though the kernel-side argc=3 is
  correct). Fix the racsh parser and re-enable the dropped awk END smoke.

### 2.3 Network tools

| Tool | DoD |
|---|---|
| `ping` | ICMP echo; either a raw socket or a new `sys_icmp` |
| `nc` | TCP/UDP listen + connect (uses existing `sys_socket`) |
| `curl`-like | wget exists; need GET/POST/headers/redirect against the existing HTTP/1.0 client |
| `ss` / `netstat` | reads `/proc/net/{tcp,udp}` (procfs entries to add) |

### 2.4 Acceptance criteria (DoD for v0.2)

- All §2.1 binaries ship at `/bin/<tool>` and have a `T02-<TOOL>-OK`
  racos-test marker.
- `racsh` boots into a session where tab-completion, history, and at
  least one user-set alias survive a `clear`, `exit`, new session.
- `ping 8.8.8.8` and `nc -l 8080 &` + `nc 127.0.0.1 8080 < hi.txt` both
  produce expected output in the QEMU interactive-smoke job.
- New CI marker `MILESTONE-V0.2-OK` printed when all of the above pass
  in a single boot.

---

## 3. v0.3 — Persistent storage

> **Goal**: nothing important lives only on a RAM-disk. `/home`, `/etc`,
> `/var/log`, `/var/lib/rpkg` all survive a reboot. Packages, scripts,
> user preferences carry across sessions.

### 3.1 Block driver

- **VirtIO-block** as a second block-device driver alongside AHCI. AHCI
  works for QEMU's emulated SATA; VirtIO-block is the cleaner
  paravirtualised path. Plumb into `drivers/block.rs` so `find("vda")`
  works the same way `find("sda")` does.

### 3.2 racfs maturity

- **Journaling** — log-mode write-then-commit for metadata operations
  (create / unlink / rename / set_metadata). Avoids torn superblock
  states on crash.
- **Allocator** — switch from linear scan to bitmap-based + free-block
  hint to make large-file growth cheaper.
- **fsck-like consistency check** at mount — confirm every allocated
  block is reachable from a live inode.

### 3.3 Mount layout

- `/home` on persistent racfs (currently mountpoint doesn't exist).
- `/etc` config moves out of initramfs into persistent storage with
  fallback-to-defaults if the partition is empty.
- `/var/log` and `/var/lib/rpkg` move to persistent storage.

### 3.4 Boot from real media

- USB-stick boot path documented + smoke-tested (currently only ESP via
  `fat:rw:esp` / pre-baked `esp.img` in CI).

### 3.5 Acceptance criteria (DoD for v0.3)

- Two-boot CI smoke extended: first boot installs an rpkg + writes to
  `/home/test/.racsh_history`; second boot reads both back and prints
  `MILESTONE-V0.3-OK`.
- `bash scripts/check-unsafe-safety.sh --strict` still clean.

---

## 4. v0.4 — Graphics base

> **Goal**: framebuffer console becomes a proper graphical terminal.
> RacOS gets its first "wow" screenshot.

### 4.1 GOP framebuffer plumbing

BootInfo already carries `framebuffer.address` / `pitch` / `height`;
`fb_console.rs` writes pixels directly. Confirm and document the format
invariant (UEFI spec → BGRA 32bpp on QEMU OVMF; physical hardware can
present RGBA). Handle both.

### 4.2 Graphical RacTerm

- Render directly from `Terminal::buffer` to the framebuffer (no PTY
  byte-forwarding). Today the buffer is updated but the actual pixels
  come from `fb_console::put_char` ANSI-naive output.
- Bitmap font rendering — extend the existing 8x16 font or add a 16x16
  option.
- **UTF-8 multibyte** in the print path.
- **Mouse-tracking modes** (1000 / 1006).

### 4.3 Optional: VirtIO-GPU

For 2D acceleration and resolution probing. Stretch goal; baseline is
the GOP framebuffer.

### 4.4 Acceptance criteria (DoD for v0.4)

- New CI job `Graphics smoke` boots QEMU with `-vga std`, asserts a
  24-bit BGRA framebuffer was claimed + RacTerm wrote ≥ 1000 distinct
  non-zero pixel values to it.
- New marker `MILESTONE-V0.4-OK`.

---

## 5. Parallel tracks (after v0.4)

These don't gate any single milestone; pick whichever has the most
leverage at the moment.

### 5.1 Networking hardening

- **TCP retransmissions** — currently a transient packet loss kills the
  connection. Per-segment retransmit timer + exponential backoff.
- **Congestion control** — Reno or simpler Tahoe.
- **Bigger receive window** — current MSS-1 send window pins throughput.
- **HTTP server** in userland — static files + a simple CGI-like hook.
- **More tools** — traceroute stub, `ip` / `route`-style display.
- **IPv6 partial** — addresses, neighbour discovery, RA. Real routing
  is a separate effort.

### 5.2 SMP scheduler refactor

- **Per-CPU run queues** — replace the single global queue.
- **IPI-based preemption** — LAPIC ICR send-vector for cross-CPU yield.
- **Per-CPU TSS** so APs can handle ring-3 IRQs (today APs only run
  the parked timer-tick loop).
- **Work stealing** between queues.

### 5.3 Packaging maturity (rapt layer)

The next "system becomes a platform" milestone:

- **HTTP-only repository protocol** — `rapt update` fetches an
  `index.toml` from a configured mirror, computes the dep graph,
  downloads `.rpk` blobs. Works today without crypto.
- **Local mirror script** — `scripts/rapt-mirror.sh` serves
  `target/packages/` over a tiny HTTP listener for the smoke tests.
- **`/etc/rapt/sources.toml`** — on-disk repo list with priority + channel.
- **Channel selection** — stable / testing / dev.
- **Once T4.1 crypto lands**: Ed25519 signed packages + signed
  repository index. Mandatory by default once shipped (per ADR-019).

### 5.4 T4.1 — TLS / HTTPS (crypto from scratch)

- ChaCha20-Poly1305 (AEAD)
- X25519 (ECDH)
- Ed25519 (signatures — also unblocks ADR-018 signing)
- SHA-256 / SHA-384
- HKDF
- TLS 1.3 handshake (1.2 is a separate KDF + cipher zoo, skipped)
- `libs/tls/` crate wired into `racnet` as a `Stream::Tls(...)` variant
- Pinned-cert MVP (`/etc/ssl/racos-roots/<x>.der`); full X.509 chain
  validation is a follow-up

### 5.5 Memory model improvements

- **Real `mmap`** — `sys_mmap` returns `ENOSYS` today. Anon pages,
  file-backed pages, `PROT_*` enforcement.
- **CoW on `sys_fork`** — fork copies every page eagerly today.
- **`mprotect`** — works for `noexec`/`nowrite` flags but doesn't TLB-flush.

### 5.6 Stability + DX

- **Better panic handling** — walk the kernel stack on panic, print a
  symbolised backtrace from debug info.
- **`unsafe fn` audit** — 137 functions still missing per-call-site
  SAFETY notes. T4.2 closed the `unsafe {` block backlog; the `unsafe fn`
  body backlog is a separate effort.
- **Property-based tests** — `proptest` for parser-heavy crates (racsh,
  init, rpkg).
- **More integration tests** — every new syscall ships with a racos-test
  case.

---

## 6. Carried-forward TODOs

Small, known issues that don't fit a milestone but should stay visible:

- **racsh `$(...)` edge case** — script with leading keyword (`END`, …)
  inside `$(...)` substitution fails with `sh: cannot open script:`
  status 127. Re-enables the dropped awk END smoke once fixed.
- **`try_deliver_user_handler:510` flaky panic** — `gs:[0x10]` syscall-
  frame pointer is occasionally unaligned. Documented in the
  `racos-ci-flakiness` memory. Fix the alignment invariant in
  `syscall::entry`.
- **`servicectl` CLI** — admin frontend for the RacInit engine
  (ADR-011). Engine API exists; binary is the missing piece.
- **Real RTC source** — unblocks proper `[timestamp]` log format from
  ADR-013.
- **Mount-flag enforcement** — `sys_mount` parses `noexec`/`nosuid`/
  `nodev` but the kernel doesn't act on them (ADR-019 §Still deferred).
- **VirtIO-net feature negotiation** — currently asks only for MAC;
  modern devices want more (MRG_RXBUF, CTRL_VQ) for sane performance.
- **`uefi` crate version bump** — pinned at 0.34. Track upstream
  releases; bump when a CVE lands.

---

## 7. Long-term (post v1.0)

Carried from `ARCHITECTURE.md §13` (explicitly excluded from v1.0):

- GUI desktop environment
- Full glibc / Linux userspace compatibility
- Container runtime (Docker-class)
- Wide HW driver support beyond QEMU + a couple of physical test rigs
- ARM, RISC-V, other architectures
- Real-time scheduling

New long-term items added by this revision:

- **Mature packaging** — once rapt + signing are in place, build a real
  community repo. `rapt install hello` returns a binary cryptographically
  tied to a known maintainer key.
- **Sandboxing extensions** — capability model + namespaces + seccomp
  allowlist (capability bits and DAC exist today; ASLR + seccomp are
  documented as deferred in ADR-019).
- **Docs completion** — every `docs/*.md` referenced from README exists
  and is current. Architecture diagrams. More ADRs for decisions made
  post-bootstrap.
- **Community** — `CONTRIBUTING.md`, GitHub issue templates, a real
  release process for tagged versions, public discoverability work
  (r/osdev posts, X / GitHub Discussions).
- **Application ports** — proof of concept: a single non-trivial Rust
  binary from crates.io running on RacOS (likely a TUI like `bottom` or
  `gitui`).

---

## 8. Operating principles (carried forward from v0.1.0)

- **Build from scratch where it teaches something** — the network stack
  (no third-party crate), libc-lite (no `libc` fork), in-tree TOML
  subset (no `toml` crate) are all examples. The kernel currently
  has **zero** external Rust dependencies; the whole shipped target
  carries two (`uefi` 0.34 + `log` 0.4, both bootloader-only). See
  [`docs/DEPENDENCIES.md`](DEPENDENCIES.md) for the full inventory
  and the "what we do not depend on" rationale.
- **Minimise unsafe in userland** — every userland crate uses
  `#![forbid(unsafe_code)]` where lints allow it, or annotates every
  block with `// SAFETY:` per ARCHITECTURE.md §3.3. The
  `Unsafe-safety annotation lint (--strict)` CI gate enforces this for
  kernel and libc-lite.
- **Write ADRs for big decisions** — see [`docs/adr/`](adr/). New ADRs
  go through PR review and link from this roadmap once accepted.
- **Test with QEMU device variety** — `-device virtio-blk-pci`,
  `-vga std`, `-vga virtio`, `-device e1000` vs `-netdev virtio-net`
  all surface different driver paths.
- **Security reports** — see [`SECURITY.md`](../SECURITY.md) for the
  disclosure channel + scope.

---

## 9. How to update this roadmap

- Each merged PR with scope changes also amends this file (mark item
  `[x]`, move to "completed" if a milestone closed, add new TODO if
  one surfaced).
- New milestones come from a discussion + PR that updates §2-§5 and
  references the existing ADRs / planned ADRs.
- Snapshot of §1 (Completed cycle) gets refreshed on each milestone
  release tag.
- [`CHANGELOG.md`](../CHANGELOG.md) gets the user-facing summary; this
  file is the developer-facing plan.
