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
| `du` | ✅ shipped (v0.2 §2.1) | `T21-COREUTILS-OK`. `-s` summarise, `-b` bytes; default is 1 KiB blocks rounded up. Reports apparent size (`st_size`). racfs still has no sparse files, but since §3.2 added indirect blocks this is no longer exactly the allocated size: a file past 8 blocks also owns the indirect blocks holding its pointers, one per 128 data blocks, which `st_size` does not count. Recursion is depth-bounded at 32: a directory cycle (possible once hard links exist) must not take the process down. |
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

- ✅ **Journaling** — shipped. Write-ahead log for metadata: an
  operation's sectors are copied into the journal and a commit record is
  written before any of them is allowed in place, so a crash leaves the
  filesystem entirely before the operation or entirely after it.
  `create` / `unlink` / `link` / `set_metadata` each run as one
  transaction; replay happens at mount, before `check()`.

  The journal lives in `[1, bitmap_start)` and its length is derived the
  same way the bitmap's is, so an image from before it existed reports a
  length of 0 and runs unjournaled — which is exactly what it is. Again
  no format version bump.

  **The subtlety that decides whether any of this works** is not in the
  log format, it is in the block cache. `evict_one` could write a dirty
  sector to the device at any moment, and `flush()` is called by the
  operations themselves — either one would put half an operation on disk
  with no journal entry describing it, arriving through a path that looks
  like harmless cache maintenance. Sectors belonging to an open
  transaction are therefore pinned: neither eviction nor flush may write
  them until the commit record exists. That pinning is also what makes
  rollback trivial — a failed operation's sectors never reached the disk,
  so dropping the cached copies undoes it completely.

  File *data* is deliberately not journalled. Logging it would double
  every write and bound file size by the journal, and losing the tail of
  a file that was mid-write is the expected outcome of a crash; losing
  the directory that names it is not.

  The list above says "and the allocation phase of `write_file`", and
  that addition is a correction to this very entry: the original plan
  said "create / unlink / rename / set_metadata", and extending a file
  mutates the same structures (bitmap, superblock free count, inode
  block map). Following the list as written left a crash window where
  the inode's new pointers could land before the bitmap marked the
  blocks used — the dangerous damage class — and left `free_blocks`
  stale on disk after any boot that ended with an extending write.

  Verified two ways, because the ordinary smokes shut the guest down
  properly and so never touch the recovery path:

  - `scripts/test-crash-consistency.ps1` — hard-kills QEMU during
    metadata churn, same disk across iterations, next boot must be clean.
  - `scripts/test-journal-replay.ps1` — forges the crash window instead
    of waiting to hit it: corrupts a live inode-table sector, then boots
    once with an empty journal (control — fsck must see the damage, which
    proves the test can fail) and once with a committed journal entry
    holding the original sector (treatment — must replay and come up
    clean). The control phase is what caught two real defects: the
    `write_file` hole above, and the Phase F AHCI self-test overwriting
    LBA 1 (see §6 note below).
- ✅ **Indirect blocks** — shipped. 8 direct + 128 single-indirect +
  128×128 double-indirect, so a file is 8.06 MiB instead of **4096
  bytes**. Directories share the map, so they are no longer capped at 64
  entries either. Smoke `T34-BIGFILE-OK`.

  This item was not in the plan, and the plan was wrong to omit it. The
  entry below named the single-sector bitmap as what capped racfs, but
  128 inodes × 8 direct pointers is 1024 blocks — a quarter of what that
  bitmap already described, so it never got the chance to bite. The 4 KiB
  file was the real reason `/var/log` and `/var/lib/rpkg` could not move
  to persistent storage in §3.3.

  New pointers sit at inode bytes 52 and 56, which the previous version
  always wrote as zero, so old inodes decode as "no indirect blocks" and
  existing disks still mount. No format version bump.
- ✅ **Allocator** — shipped: multi-sector bitmap + free-block hint. The
  bitmap now spans as many sectors as the device needs (8 on the 16 MiB
  CI disk, **32727 addressable blocks against 4096 before**). Its length
  is derived from `inode_start - bitmap_start` rather than stored, so a
  single-sector-era image reads back as length 1 — which is what it has —
  instead of needing a version bump that would unmount every existing
  disk. `alloc_block` scans from where the last allocation landed and
  wraps; the wrap is required, not an optimisation, because without it a
  block freed below the hint would be unreachable until the next mount.

  `check()` reports `unaddressable_blocks` for the dead space on an old
  image, and `df` totals now clamp to what the bitmap describes rather
  than to the raw device size.
- **Inode count** — 128, unchanged, and the next capacity limit to bite
  now that file size is not. Raising it changes only a fresh format
  (`inode_count` comes from the superblock), so it is cheap; nothing has
  run out yet.
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

  It now also walks the indirect blocks and counts them as referenced —
  they come from the same bitmap, so omitting them would report every one
  of them leaked.

### 3.3 Mount layout

- ✅ `/home` on persistent racfs — shipped.
- ✅ `/etc` config moves out of initramfs into persistent storage with
  fallback-to-defaults if the partition is empty — shipped.
- ✅ `/var/log` and `/var/lib/rpkg` move to persistent storage — shipped.

All four are **subtree mounts of the single racfs on sda**, not separate
filesystems. AHCI here binds one port and registers it as `sda`, so there
is exactly one persistent device — and four directories needing to
survive a reboot is not a reason to demand four disks. `RacfsFilesystem`
gained a root inode; a subtree mount is the same filesystem entered at a
different inode, and the mount table already routes by longest prefix.

The change that made this more than a rename: every write path resolved
from inode 0, so `mkdir /home/x` through a subtree mount would have
created `x` in the **disk root** — succeeding, reading back, and looking
correct. `split_parent_leaf_from` / `lookup_path_from` take the mount's
root instead. `T35-MOUNTS-OK` checks the file through `/mnt` as well as
through `/home`, which is what separates "it worked" from "it went where
it was supposed to".

`/etc` is boot-critical in a way the others are not: RacInit reads
`/etc/racinit/base.target`, so mounting an empty directory over it leaves
PID 1 with no units and the guest with no shell. The defaults are
therefore copied in first, read back through the filesystem that will
serve them, and the mount happens only if that worked — every failure
path leaves `/etc` on the initramfs. Seeding runs only on an `/etc` that
has never been populated: a user who removed a unit file meant to remove
it.

`$HOME` is now `/home/racos`, so racsh's history lands on persistent
storage instead of `/var/.racsh_history`. That settles half of §2.4's
open DoD criterion; aliases still have no `~/.racshrc` to be reloaded
from.

Still open in §3.3: nothing. `/var` itself remains on ram0 by design —
only the subtrees the roadmap named are persistent, and the rest of
`/var` is scratch.

### 3.4 Boot from real media

- ✅ Shipped. `scripts/make-esp-image.py` builds an MBR-partitioned FAT32
  disk image from `esp/` — the FAT32 formatter is written from scratch in
  Python (~250 lines, LFN entries included), because mtools does not exist
  on a stock Windows box and `fat:rw:esp` is not a real filesystem.
  `scripts/test-usb-boot.ps1` boots that image as USB mass storage on an
  XHCI controller with **no** `fat:rw` fallback attached, requiring the
  whole chain — OVMF USB enumeration, partition parse, FAT32 driver,
  `\EFI\BOOT\BOOTX64.EFI` fallback, our bootloader reading kernel +
  initramfs over USB — to produce a racsh prompt. Marker `USB-BOOT PASS`,
  gate 9. The physical-stick procedure and its honest caveats (PS/2-only
  keyboard, no USB stack after boot, no NVMe, no real NICs) live in
  [`docs/BOOT-MEDIA.md`](BOOT-MEDIA.md).

- Making real hardware *reachable* forced one safety change:
  `Racfs::open_or_format` formatted on any superblock mismatch, which was
  fine while every disk in reach was a zeroed QEMU image and would have
  been a data shredder on a machine whose first AHCI disk holds Windows.
  The boot now formats only a **blank** disk (first sector all zeroes);
  anything else is refused with instructions to run `mkfs.racfs sda`
  deliberately. QEMU smokes create their disks zero-filled, so CI still
  auto-formats exactly as before.

### 3.5 Acceptance criteria (DoD for v0.3)

- ✅ **`MILESTONE-V0.3-OK` — printed by the guest itself.**
  `scripts/test-milestone-v03.ps1` (gate 10): boot 1 types a command into
  racsh (history is saved after every line, to `/home/racos/` on the
  persistent disk), installs `/share/demo.rpk` — a sample package now
  shipped in the initramfs, generated by `scripts/make-demo-rpk.py` —
  syncs, and is **hard-killed**. Boot 2 verifies the history line and the
  installed package survived and prints the marker on its own console;
  the host only greps for it. The marker coming from inside the system is
  the point: it proves racsh, grep, rpkg, the journal and the persistent
  mounts cooperate after an unfriendly reboot.
- ✅ `bash scripts/check-unsafe-safety.sh --strict` still clean (gate 5,
  which since 2026-08-29 also refuses to pass when it scanned nothing).

**v0.3 is complete**: §3.2 (capacity + fsck + journal), §3.3 (persistent
mount layout), §3.4 (real-media boot), §3.5 (DoD). §3.1 VirtIO-block was
not needed for any of it — subtree mounts made one AHCI disk enough — and
moves to the parallel tracks as nice-to-have.

---

## 4. v0.4 — Graphics base

> **Goal**: own the framebuffer properly and get RacOS its first "wow"
> screenshot.
>
> Read this section against §6b. The work below is the same either way,
> but what it should turn into differs: build the framebuffer owner as
> something that hands out *surfaces*, with RacTerm as its first client,
> rather than as a terminal that happens to own the screen. A terminal
> that owns the screen has to be taken apart again the day a second
> window exists.

### 4.1 GOP framebuffer plumbing

- ✅ Shipped, as `kernel/src/gfx.rs` — the framebuffer **owner** from §6b.
  Nothing in the kernel writes a pixel except through it: clients get a
  region or a `Surface`, and the owner decides where the bytes land. The
  console asks for its region (`console_region()`) instead of assuming it
  owns the screen; the status bar at the bottom is the first client
  rendered through a real off-screen surface (`Surface` + `present`).

  The format invariant, confirmed and handled: GOP hands over 32bpp
  linear in **BGRX** (QEMU OVMF, always) or **RGBX** (possible on real
  hardware); BootInfo carried `PixelFormat` all along and nothing read
  it. The old console wrote raw `0xRRGGBB` u32s — which happens to match
  BGRX byte order on little-endian, so it looked correct for two
  milestones and would have swapped red and blue on RGBX hardware.
  `gfx::encode()` is now the single place that knows the channel order.

  Also in this slice from §4.2's list: **UTF-8 multibyte in the print
  path**. Bytes ≥ 0x80 were silently dropped — multibyte text vanished
  and the cursor pretended it was never there. The console now decodes
  the sequence length and draws one replacement glyph (`FONT[0x7F]`, a
  hollow box) per character, with correct cursor arithmetic. Real glyph
  coverage beyond ASCII needs a bigger font and is still open.

  Verified by gate 11 (`scripts/test-graphics.ps1`): the claim line with
  geometry and channel order, plus a **QMP screendump** required to
  contain ≥ 1000 distinct non-zero pixel values — 25 571 in practice.
  The dump is the assertion the serial log cannot make: the first version
  of this slice drew the status bar before the heap existed, the bar
  silently never appeared, and every log line still looked perfect.

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

- ✅ The graphics smoke exists as gate 11: boots with `-vga std`, asserts
  the claim line (geometry + channel order) and ≥ 1000 distinct non-zero
  pixel values in a QMP screendump of the actual display.
- ⏳ `MILESTONE-V0.4-OK` — not yet: §4.2's RacTerm-from-buffer rendering
  and mouse tracking are still open, and the milestone marker waits for
  them.

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

- ~~**Phase F AHCI persistence self-test**~~ — **REMOVED 2026-08-29**,
  because it was corrupting the filesystem it shared a disk with. It
  wrote a 17-byte marker into LBA 1 on every boot that did not find it
  there; LBA 1 has belonged to the filesystem since Phase F ended (first
  the allocation bitmap, now the journal header). The "genuinely damaged"
  evidence image this roadmap cites for fsck's verification carries that
  marker at LBA 1 — most of its damage was manufactured by this
  self-test, not by power loss. Its job is done better by `boot-counter`
  and `big-probe` surviving reboots through the real filesystem.
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

## 6b. Destination: a graphical workspace

> **The stated goal of this project is a full graphical desktop** — real
> windows, real applications, and responsiveness a Linux user would
> recognise as normal rather than excuse as "for a hobby OS". This is
> not a post-v1.0 curiosity; it is what the rest of the plan is for, and
> it is written here because it changes decisions taken *now*.

**On the language.** Rust already is the right choice, and the reason is
not taste. Smooth interaction means bounded, predictable latency, and
the two things that most reliably destroy that in a desktop stack are
garbage-collection pauses and a copy on every frame. Rust has no GC and
gives direct control of memory layout, so the ceiling here is the same
one C has on Linux. The kernel carries **zero** external dependencies
today (`docs/DEPENDENCIES.md`), which is what keeps that ceiling
reachable rather than mortgaged to somebody else's abstractions.

So the honest statement is: the language is not the risk. The risk is
everything below.

**What actually decides whether it feels smooth**, roughly in order of
how much each one hurts if it is missing:

1. **`mmap` and copy-on-write `fork`** (§5.5). `sys_mmap` returns
   `ENOSYS` and `fork` copies every page eagerly. A window server hands
   clients shared buffers; without `mmap` every pixel is copied through
   a syscall, and no amount of fast drawing recovers that. **This is the
   single largest gap between here and a usable desktop**, and it is a
   memory-model problem, not a graphics one.
2. **GPU-backed surfaces** (§4.3 VirtIO-GPU). Compositing in software on
   a CPU-drawn framebuffer sets a hard ceiling on window count and
   animation. The GOP framebuffer in §4.1 is the right baseline and the
   wrong destination.
3. **Per-CPU scheduling and IPI preemption** (§5.2). One global run queue
   means input latency is at the mercy of whatever else is runnable. A
   desktop is judged on the worst frame, not the average.
4. **A display server protocol and a toolkit.** Neither exists. This is
   the largest *volume* of work by far, and the least novel — which is
   why it is last: it is the part that cannot start until 1-3 are real.

**What this changes about v0.4.** Section 4 currently reads as "make the
framebuffer console a nicer terminal". Under this goal it should be
designed as the bottom of a compositor instead: an owner of the
framebuffer that hands out surfaces, with RacTerm as its first client
rather than as the thing that owns the screen. Same amount of work at
this stage, very different thing to build on.

**What it does not change.** v0.3 and the correctness work stay in front
of it. A desktop on a filesystem that loses data on a hard reset is not
a desktop anyone keeps using, which is why journaling (§3.2) is still
the next item and not a detour.

---

## 7. Long-term (post v1.0)

Carried from `ARCHITECTURE.md §13`. The GUI line is no longer an
exclusion — see §6b, which makes it the destination — but the rest still
stand as out of scope for v1.0:

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
