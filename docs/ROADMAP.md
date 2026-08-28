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
| `id` | ✅ shipped (v0.2 §2.1) | `T20-ID-OK` |
| `sort` | ✅ shipped (v0.2 §2.1) | `T20-SORT-OK` |
| `top` | ✅ shipped (v0.2 §2.1, batch mode) | `T20-TOP-OK` |

Interactive `top` (live refresh, signal-driven redraw, alternate screen
buffer) is post-MVP — today the binary is the moral equivalent of `top
-b -n 1` on Linux: prints uptime + the same per-PID table `ps` emits,
then exits. The CI smoke is much simpler that way.

**Net-new**:

| Tool | Status | DoD / Notes |
|---|---|---|
| `touch` | ✅ shipped (v0.2 §2.1) | `T20-TOUCH-OK`. MVP: O_CREAT a missing path; existing path is a no-op exit 0. `utime`/`utimensat` syscall is post-MVP, so `-a`/`-m`/`-t` flags are intentionally absent for now. |
| `chmod` | ✅ shipped (v0.2 §2.1) | `T20-CHMOD-OK`. Octal mode only (`644`, `0644`, `0o644`). Symbolic `u+x`/`g-w`/`a=rwx` is post-MVP. |
| `chown` | ✅ shipped (v0.2 §2.1) | `T20-CHOWN-OK`. Numeric `uid` / `uid:gid` / `:gid` only. Symbolic usernames need `/etc/passwd` + `/etc/group` lookup (post-MVP). |
| `kill` | ⏳ | `kill -SIG pid`, default TERM; `sys_kill` exists. Also a racsh builtin already. |
| `whoami` | ⏳ | print euid → username (needs `/etc/passwd` lookup) |
| `uname` | ⏳ | `-a` / `-r` / `-m` / `-s`; `sys_uname` exists |
| `free` | ✅ shipped (v0.2 §2.1) | `T21-COREUTILS-OK`. `/proc/meminfo` already existed, so no procfs work was needed. `-k` (default) / `-m`. Fields are read **by name**, not line position — procfs may add fields, and a positional parser would silently start reporting the wrong numbers. |
| `ln` | ⏳ | **Blocked on the kernel, not on userland.** `sys_link` is a stub returning `ENOSYS`; hard links need racfs support (inode link count, a second dirent pointing at one inode, and unlink decrementing rather than freeing). That is kernel work deserving its own change, not a coreutil. Symlinks wait on `sys_symlink`, also a stub. |
| `rmdir` | ✅ shipped (v0.2 §2.1) | `T21-COREUTILS-OK`. Emptiness is enforced by racfs (`unlink` refuses a directory with entries), not re-checked in userland where it would be a race. Refuses a regular file after `stat` so the message names the real problem. |
| `du` | ✅ shipped (v0.2 §2.1) | `T21-COREUTILS-OK`. `-s` summarise, `-b` bytes; default is 1 KiB blocks rounded up. Reports apparent size (`st_size`) — racfs has no sparse files, so it equals allocated size today. Recursion is depth-bounded at 32: a directory cycle (possible once hard links exist) must not take the process down. |
| `clear` | ✅ shipped (not in the original plan) | `T21-COREUTILS-OK`. Ctrl-L was always bound, but the missing command reads as a broken system. Emits ED 2 + CUP in a **single** write — RacTerm and the framebuffer console parse CSI per write, so a split sequence prints its tail literally. |

`pwd` and `cd` are already racsh builtins; standalone `/bin/pwd` is
conventional but not required for v0.2.

### 2.2 racsh UX

- ✅ **Persistent history** — shipped. `History::load_file` / `save_file`,
  cap 1000 entries, rewritten in full after each line so the cap holds and
  no seek is needed. Path is `$HOME/.racsh_history`, falling back to
  `/var/.racsh_history` because `/` is the read-only initramfs. Surviving a
  *reboot* still waits on v0.3 §3.3, which puts `/home` on persistent
  storage. `history` and `history -c` are handled in the REPL loop rather
  than `racsh::builtin`, because the list is session state that
  `exec_simple` has no handle on — the cost is that they don't work inside
  a pipeline.
- ✅ **Tab completion** — shipped. Command position offers builtins plus
  every executable on `$PATH`; anywhere else completes paths. A word
  containing `/` is always path-completed, so `./x` and `/bin/l` behave as
  they read. One match is inserted with a trailing space, several extend by
  their common prefix, and an ambiguous word with no further prefix prints
  the candidates and redraws. Logic lives in `shell/src/complete.rs`, split
  so the decision and prefix arithmetic are pure and host-tested.
- ✅ **Aliases** — shipped. `alias`, `alias NAME`, `alias NAME=VALUE`,
  `unalias NAME...`, `unalias -a`; stored sorted in `Env::aliases`.
  Expansion happens at execution time in `exec_simple`, not parse time as
  originally sketched: the parser has no `Env`, and exec-time expansion is
  where the command word is already known. A name is expanded at most once
  per command, so `alias ls='ls --color'` terminates. Replacements are split
  on whitespace, so quoting *inside* an alias body does not survive — use a
  shell function for that. Smoke `T22-ALIAS-OK`.
- ✅ **Prompt expansions** — shipped. `\u`, `\h`, `\w`, `\W`, `\$`, `\n`,
  `\\` in `shell/src/prompt.rs`; `\w` abbreviates `$HOME` to `~` on whole
  components only. An unknown escape is emitted verbatim so a typo is
  visible instead of silently deleting text.
- ✅ **`$(...)` command-substitution edge case** — fixed, and it was never a
  racsh bug: `prepare_user_stack` wrote the envp NULL terminator past the
  reserved block, corrupting argv strings depending on their total length.
  The dropped awk END smoke is back.

Still open in §2.2: nothing. The remaining v0.2 work is §2.1 (`free`, `ln`,
`rmdir`, `du`) and §2.3 (network tools).

### 2.3 Network tools

| Tool | Status | Notes |
|---|---|---|
| `ping` | ✅ shipped | `T23-NETTOOLS-OK`. New `SYS_ICMP_ECHO` (81) wrapping the stack's existing `send_icmp_echo`; a raw-socket API was not worth inventing for one tool. Replies are matched by the reply counter changing, not by sequence number — the ICMP receive path only bumps `echo_replies`. With one echo in flight that race is theoretical but real. **Under QEMU slirp only the gateway answers ICMP**, so `ping 10.0.2.2` replies while `ping example.com` resolves and reports 100% loss. |
| `nc` | ✅ shipped | `T23-NETTOOLS-OK`. TCP only, on the existing socket syscalls. Relays with `poll()` on stdin and the socket together (userland has no threads); half-closes with `shutdown(SHUT_WR)` on stdin EOF so a piped body is not truncated. UDP needs `SOCK_DGRAM` through `sys_send`/`sys_recv`, which the kernel does not offer. |
| `ss` / `netstat` | ✅ shipped | `T23-NETTOOLS-OK`. `/proc/net/{tcp,udp}` added. `snapshot()` copies rows under `try_lock` — formatting text while holding the connection table would deadlock against the timer's retransmit path. `/proc/net/udp` carries a header and no rows: the UDP path is connectionless and keeps no socket table. |
| `curl`-like | ⏳ | `wget` exists; still needs GET/POST, headers and redirects against the HTTP/1.0 client. Not required for the v0.2 DoD. |

### 2.4 Acceptance criteria (DoD for v0.2)

**Status: the three feature sections are done.** §2.1, §2.2 and §2.3 all ship,
covered by `T20-*`, `T21-COREUTILS-OK`, `T21-HARDLINK-OK`, `T22-ALIAS-OK` and
`T23-NETTOOLS-OK` — 160 in-guest assertions, 24 markers.

- [x] §2.1 binaries ship at `/bin/<tool>` with racos-test markers. (The
      markers are grouped per section rather than one `T02-<TOOL>-OK` each;
      grouping keeps a tool's cases together and made the missing-marker
      failure mode obvious.)
- [x] racsh has tab completion, history and aliases.
- [x] `ping` and `nc` work and are smoked.
- [ ] **`MILESTONE-V0.2-OK`** — not yet emitted. Two DoD items are worth
      revisiting before it is:
      - Aliases and history surviving *a new session* is untested. History
        does survive within a boot (`/var/.racsh_history`) but not a reboot
        until v0.3 §3.3 moves `/home` onto persistent storage; aliases have
        no `~/.racshrc` to be reloaded from at all. Either the criterion or
        the shell needs to change.
      - The `nc -l 8080 &` + `nc 127.0.0.1 8080` round trip is not smoked.
        Backgrounding a listener and connecting to it from the same shell
        needs job control and loopback TCP to cooperate; worth confirming by
        hand before it becomes a CI gate.

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
  hint to make large-file growth cheaper. **Bigger than it reads:** the
  allocation bitmap is a single sector, so it can only describe
  `SECTOR_SIZE * 8` = 4096 blocks, and `alloc_block` scans exactly that
  range. On a 16 MiB disk that leaves 4096 of 32734 data blocks reachable
  and the other 87% dead space. A multi-sector bitmap is the prerequisite,
  not the optimisation.
- ✅ **fsck-like consistency check** at mount — shipped. `Racfs::check()`
  walks live inodes, builds a per-block reference count, and compares it
  against the bitmap and superblock. Findings are split by whether they can
  destroy data: leaked blocks and superblock drift are untidy but safe;
  `unallocated_in_use` and `doubly_claimed` are not, because the linear
  allocator will hand those blocks out again. It reports and does not
  repair — choosing an owner for a doubly-claimed block belongs to whoever
  can see which file matters. Verified against a genuinely damaged image
  from this project's history, which it diagnosed as
  `leaked=52 unallocated_in_use=4 doubly_claimed=1 out_of_range=1`.

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

- ~~**racsh `$(...)` edge case**~~ — **FIXED**. Was never a racsh bug:
  `prepare_user_stack` wrote the envp NULL terminator one slot past the
  reserved argc/argv/envp block, clobbering the argv string data directly
  above it. Whether it corrupted anything depended on total argv length,
  which is why it looked like "only scripts with a leading `END`". The
  dropped awk END smoke is re-enabled in `racos-test`.
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
