# Changelog

All notable changes to RacOS are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 entries describe development progress relative to the tiered plan in
[`docs/ROADMAP.md`](docs/ROADMAP.md). Each entry references the merged PRs and
the architectural sub-task IDs (T1.x, T2.x, …) that motivated it.

## [Unreleased]

### Added — v0.4 first slice: the framebuffer gets an owner (§4.1, per §6b)

- **`kernel/src/gfx.rs` — the framebuffer owner.** Nothing in the kernel
  writes a pixel except through it: clients get a region or a `Surface`,
  and the owner decides where those bytes land. The console now *asks* for
  its region instead of assuming it owns the screen, and the status bar at
  the bottom — a hue gradient with the OS name, drawn into an off-screen
  `Surface` and presented by the owner — is the first client on the new
  path. Per ROADMAP §6b this inversion is the point: a terminal that owns
  the screen has to be taken apart the day a second window exists, so
  v0.4's work is built as the bottom of a compositor from the start.

- **The §4.1 format invariant, confirmed and handled.** GOP hands over
  32bpp linear in BGRX (QEMU OVMF, always) or RGBX (possible on hardware);
  BootInfo carried `PixelFormat` all along and nothing read it. The old
  console wrote raw `0xRRGGBB` u32s — which happens to match BGRX byte
  order on little-endian, so it looked correct for two milestones and
  would have swapped red and blue on RGBX hardware. `gfx::encode()` is now
  the one place that knows the channel order.

- **UTF-8 multibyte in the print path** (from §4.2's list). Bytes ≥ 0x80
  were silently dropped: multibyte text vanished and the cursor pretended
  it was never there. The console decodes the sequence length and draws
  one replacement glyph (a hollow box, `FONT[0x7F]`) per *character*, with
  correct cursor arithmetic. Glyphs beyond ASCII need a bigger font —
  still open.

- **Gate 11, the graphics smoke** (`scripts/test-graphics.ps1`): asserts
  the owner's claim line (geometry + channel order) and takes a **QMP
  screendump** required to contain ≥ 1000 distinct non-zero pixel values
  — 25 571 in practice, which only the gradient's per-pixel shading can
  produce, so the number is reachable only if `Surface`/`present` actually
  work. The dump earned its keep immediately: the first version drew the
  status bar before the heap existed, the bar silently never appeared, and
  every serial line still looked perfect. One distinct pixel value in the
  dump — white text — was the only witness.

### Added — v0.3 §3.4 + §3.5: real-media boot, and MILESTONE-V0.3-OK

**v0.3 is complete.** The guest prints the milestone marker itself, after a
hard-kill reboot, from a system that can also boot off a USB stick.

- **§3.4 — boot from real media.** `scripts/make-esp-image.py` builds an
  MBR-partitioned FAT32 disk image from `esp/`. The FAT32 formatter is
  written from scratch (~250 lines of dependency-free Python, LFN entries
  included), because mtools does not exist on a stock Windows box and
  QEMU's `fat:rw:esp` is not a real filesystem. The image was verified by
  an independent from-scratch FAT32 *reader* comparing every file
  byte-for-byte against the source tree, then by the real consumer:
  `scripts/test-usb-boot.ps1` (gate 9) attaches it as USB mass storage on
  an XHCI controller with no `fat:rw` fallback and requires OVMF USB
  enumeration → partition parse → FAT32 → `\EFI\BOOT\BOOTX64.EFI`
  fallback → our bootloader reading kernel + 157 MB initramfs over USB →
  racsh prompt. Passed on the first boot. The physical-stick procedure and
  its honest limits (PS/2-only keyboard, no USB stack after handover, no
  NVMe, no real NICs) are in `docs/BOOT-MEDIA.md`.

- **Reaching real hardware forced a safety fix: RacOS no longer formats
  disks it does not recognise.** `Racfs::open_or_format` formatted on any
  superblock mismatch — fine while every disk in reach was a zeroed QEMU
  image, a data shredder on a machine whose first AHCI disk holds Windows.
  The boot now formats only a *blank* disk (first sector all zeroes) and
  refuses anything else with instructions to run `mkfs.racfs sda`
  deliberately. Destroying a filesystem is a decision for the person who
  can see what it is, not for a boot path.

- **§3.5 — the milestone smoke** (`scripts/test-milestone-v03.ps1`,
  gate 10). Boot 1 types a command into racsh — history is saved after
  every line to `/home/racos/` on the persistent disk — installs
  `/share/demo.rpk`, syncs, and is hard-killed. Boot 2 verifies both
  survived and prints `MILESTONE-V0.3-OK` on its own console; the host
  only greps. The marker coming from inside the system is the point: it
  proves racsh, grep, rpkg, the journal and the persistent mounts
  cooperate after an unfriendly reboot.

- **`/share/demo.rpk`** — a sample package now ships in the initramfs,
  generated at build time by `scripts/make-demo-rpk.py`, so "install
  something and see it work" needs no network.

- **A testing lesson, recorded because it will bite again:** the first
  version of the milestone smoke reported an impossible result — both
  checks FAIL yet the final marker "passed". The serial console echoes
  keystrokes, so any pattern that appears in the typed command matches its
  own echo before the guest has produced output. Success tokens are now
  assembled from shell variables (`echo $mm`), so the token text never
  appears in the typed line and can only come from real output.

### Added — v0.3 §3.2 (racfs metadata journal)

- **Write-ahead log for metadata.** An operation's sectors are copied into
  the journal and a commit record is written before any of them is allowed
  to land in place, so a crash leaves the filesystem entirely before the
  operation or entirely after it — never half-applied. `create` / `unlink` /
  `link` / `set_metadata` and the allocation phase of `write_file` each run
  as one transaction; replay happens at mount, before the consistency check,
  so `check()` sees what the completed operation left rather than the middle
  of a write.

  The journal lives in `[1, bitmap_start)` and its length is derived the
  same way the bitmap's is, so an image from before it existed reports a
  length of 0 and simply runs unjournaled. Again no format version bump.

  **The part that decides whether any of this works is in the block cache,
  not the log format.** `evict_one` could write a dirty sector to the device
  at any moment, and `flush()` is called by the operations themselves —
  either would put half an operation on disk with no journal entry
  describing it, through a path that looks like harmless cache maintenance.
  Sectors belonging to an open transaction are therefore pinned: neither
  eviction nor flush may write them until the commit record exists. The same
  pinning makes rollback trivial — a failed operation's sectors never
  reached the disk, so dropping the cached copies undoes it completely.

  File *data* is deliberately not journalled: logging it would double every
  write and bound file size by the journal, and losing the tail of a file
  that was mid-write is the expected outcome of a crash, in a way losing the
  directory that names it is not.

- **`write_file`'s allocation phase is a transaction too — this was a real
  hole, found by testing.** ROADMAP §3.2 listed "create / unlink / rename /
  set_metadata" and the first implementation followed that list. But
  extending a file mutates the bitmap, the superblock's free count and the
  inode's block map — the same structures. The cache flushes in whatever
  order it likes, so a crash could land the inode's new pointers before the
  bitmap marked those blocks used: an inode referencing blocks the allocator
  will hand out again, the damage class fsck calls dangerous. And nothing on
  that path wrote the superblock at all, so any boot whose last operation
  was an extending write left `free_blocks` stale on disk. Data is written
  after the allocation commit, and the size is updated last — so a crash
  reads the old length, never uninitialised bytes. `sync()` now flushes the
  superblock as well; it promised durability while omitting it.

- **Two new hostile smokes**, because the ordinary ones cannot test recovery
  — they shut the guest down cleanly, so they exercise the commit path and
  never the replay path:

  - `scripts/test-crash-consistency.ps1` kills QEMU outright during a
    metadata churn loop and asserts the next boot mounts clean. The disk is
    deliberately reused across iterations so damage would accumulate.
  - `scripts/test-journal-replay.ps1` forges the crash window instead of
    waiting to hit it: it corrupts a live inode-table sector on the image
    directly, and boots twice — once with an empty journal (the **control**:
    fsck must report the damage, proving the test can fail) and once with a
    committed journal entry holding the original sector (the **treatment**:
    the boot must replay it and come up clean). Both phases start from a
    byte-identical snapshot of the seeded image. Result:
    `replay ran: transaction 4242, 1 sector restored` → `fsck clean`.

### Removed — the Phase F AHCI self-test, which was corrupting the disk

- **`ahci_self_test()` wrote "RACOS-AHCI-PhaseF" into LBA 1 on every boot
  that did not find it there.** It was written when nothing else lived on
  sda; LBA 1 has belonged to the filesystem ever since — first to the
  allocation bitmap, now to the journal header. Every boot that re-wrote
  the marker overwrote 17 bytes of live filesystem metadata.

  This rewrites the history of this project's corruption evidence.
  `racos-disk-corrupt-evidence.img`, kept as proof of "genuine corruption"
  and cited below for its `leaked=52` diagnosis, holds
  `SACOS-AHCI-PhaseF` at LBA 1 — the marker with one extra bit where the
  filesystem later allocated block 0. The marker sets 55 bits; fsck
  reported 52 leaked blocks and `unallocated_in_use=4`. **The corruption
  fsck was built to detect was largely being manufactured by this
  self-test**, boot after boot, not by unlucky power loss. The consistency
  check entry below stands as written — the check does detect exactly this
  damage — but the damage's origin was us.

  Deleted rather than moved: what it proved is now proved properly by
  `boot-counter` and `big-probe` surviving reboots through the real
  filesystem, which says more about AHCI persistence than a raw sector
  marker ever did. Found by the journal-replay test's control phase, whose
  first run showed a journal header full of ASCII.

### Added — v0.3 §3.3 (mount layout: /home, /etc, /var/log, /var/lib/rpkg)

- **Four directories now survive a reboot**, as subtree mounts of the single
  racfs on sda rather than as filesystems of their own. AHCI here binds one
  port and registers it as `sda`, so there is exactly one persistent device,
  and four directories needing to outlive a boot is not a reason to demand
  four disks. `RacfsFilesystem` gained a root inode: a subtree mount is the
  same filesystem entered at a different inode, and the mount table already
  routes by longest prefix.

- **The part that was not a rename.** Every write path resolved from inode 0,
  so `mkdir /home/x` through a subtree mount would have created `x` in the
  **disk root** — succeeding, reading back, and looking entirely correct.
  `lookup_path_from` / `split_parent_leaf_from` take the mount's root
  instead, and `WritableStore::Racfs` carries it. The smoke checks the file
  through `/mnt` as well as through `/home`, and asserts it is *not* in the
  disk root; that second half is what separates "it worked" from "it went
  where it was supposed to".

- **`/etc` is handled differently from the rest, because it can break the
  boot.** RacInit reads `/etc/racinit/base.target`, so mounting an empty
  directory over `/etc` leaves PID 1 with no units and the guest with no
  shell — and the serial log shows a clean boot right up to the point where
  nothing happens. So the initramfs defaults are copied in first, read back
  through the filesystem that will serve them, and the mount happens only if
  that worked. Every failure path leaves `/etc` on the initramfs, which is
  the arrangement that boots today. Seeding runs only on an `/etc` that has
  never been populated: a user who deleted a unit file meant to delete it,
  and restoring it each boot would make it undeletable.

- **`$HOME` is `/home/racos`**, so racsh's history goes to persistent storage
  instead of `/var/.racsh_history`. That settles half of the v0.2 §2.4 DoD
  criterion that has been open since the milestone; aliases still have no
  `~/.racshrc` to reload from.

- **`/var/log` and `/var/lib` exist as real directories on the ram0 racfs**,
  so `ls /var` shows them. `readdir` lists the filesystem *below* a mount
  point, not the mount table, so a mount point with nothing underneath is
  reachable but invisible.

- **Smoke `T35-MOUNTS-OK`** plus six cross-reboot assertions in the two-boot
  persistence smoke. The sharpest of those is a negative: boot 2 must mount
  `/etc` **without** re-seeding it. A boot that re-seeds would mean the
  persistent copy did not survive — which is also exactly the state that
  leaves init with no units.

### Added — v0.3 §3.2 (racfs capacity: indirect blocks + multi-sector bitmap)

- **Inodes gained single- and double-indirect block pointers.** A racfs file
  was eight direct blocks — **4096 bytes** — and returned `ENOSPC` past that
  on a disk reporting 16 MiB free. It is now 8 direct + 128 single-indirect +
  128×128 double-indirect blocks, so 8.06 MiB.

  ROADMAP §3.2 named the single-sector allocation bitmap as the thing capping
  racfs, and that was measuring the wrong limit. 128 inodes × 8 direct
  pointers is 1024 blocks — a quarter of what the 4096-block bitmap already
  described. The bitmap never got the chance to bite. What actually blocked
  v0.3's goal of moving `/var/log` and `/var/lib/rpkg` onto persistent
  storage was the 4 KiB file, and no rpkg blob or log fits in that.

  The two limits are complements, not alternatives: indirect blocks are what
  make the bitmap's size matter, so both ship together.

  New pointers live at inode bytes 52 and 56, which the previous version
  always wrote as zero. An inode written before this decodes as "no indirect
  blocks", which is exactly true of it — no format version bump, and existing
  disks still mount.

- **Directories use the same block map**, so a directory is no longer capped
  at the 64 entries eight blocks of entries hold. Nothing had hit that yet
  only because nothing persistent had that many names in it; `/home` would
  have.

- **The allocation bitmap now spans as many sectors as the device needs.**
  Its length is not a new superblock field: it is `inode_start -
  bitmap_start`, which the layout has always implied, so an image from the
  single-sector era reports a length of 1 — precisely what it has. A stored
  field would have required a version bump and made every existing disk
  unmountable to buy nothing.

  `format_and_new` sizes it by fixed point, since the bitmap's size and the
  data area's size each depend on the other. On the 16 MiB CI disk that is 8
  bitmap sectors and **32727 addressable data blocks, against 4096 before**.

- **Allocation got a free-block hint.** `alloc_block` scans from where the
  last one landed and wraps, instead of restarting at block 0 and re-reading
  the same full bitmap sectors every time. The wrap is not optional: without
  it a block freed below the hint would stay unreachable until the next
  mount. `free_block` lowers the hint to the hole it just made, so a
  create/delete loop stays inside one bitmap sector.

- **`check()` walks the indirect blocks too**, counting them as referenced.
  They are allocated from the same bitmap, so omitting them would have
  reported every one of them as leaked. It also gained
  `unaddressable_blocks`, which reports the dead space on a
  single-sector-era image — deliberately *not* part of `is_clean()`, because
  wasted capacity on an old layout is a fact about that layout and not
  damage, and marking a healthy old disk unclean would train the reader to
  ignore the line. The boot message says reformatting is what recovers it.

- **`du` now slightly understates what a large file costs.** It reports
  apparent size (`st_size`), and a file past 8 blocks also owns the indirect
  blocks holding its pointers — one per 128 data blocks — which `st_size`
  does not count. Not worth an `st_blocks` field yet, but ROADMAP §2.1's
  claim that apparent size equals allocated size is no longer true and has
  been corrected there.

- **`df`-style totals report what can actually be allocated.** `stats()`
  clamps to what the bitmap describes rather than to the raw device size:
  promising 16 MiB where `alloc_block` will only ever find 2 MiB is a lie
  the user then has to debug.

- **`free_block` refuses a block the bitmap cannot describe** instead of
  indexing a 512-byte array with a byte offset of up to 4091. Only a corrupt
  inode reaches that, and it used to take the kernel down. `free_inode` now
  ignores such failures so a damaged file can still be `rm`ed — which is
  what the fsck warning tells the user to do.

- **Smoke `T34-BIGFILE-OK`** — six in-guest assertions: a 16402-byte file
  (single indirect) with markers in its first and last block, a 262432-byte
  file (double indirect), free-block accounting returning to its exact
  starting value after a delete, a 73-entry directory, and `/proc/diskstats`
  reporting more than one bitmap sector's worth of blocks.

- **The two-boot persistence smoke now proves the indirect map survives a
  reboot.** `boot-counter` is one byte long, so it only ever exercised
  `direct[0]` — it would have kept passing on a filesystem whose indirect
  blocks were written somewhere else entirely, which is exactly the code this
  release adds. Alongside it the kernel now keeps `big-probe`, 8192 bytes (8
  direct blocks and 8 through the single indirect), whose last 16 bytes carry
  the number of the boot that wrote them. The second boot reads *those*
  bytes, because they are the part only the indirect map can reach.

- **Verified against images from the single-sector era.** A healthy one still
  mounts, its `boot-counter` still increments across the reboot, and it
  reports `28638 data blocks are unreachable` — exactly 32734 − 4096. The
  damaged image kept from earlier in this project's history still diagnoses
  as `leaked=52 unallocated_in_use=4 doubly_claimed=1 out_of_range=1
  sb_drift=-49`, byte for byte what it reported before this change.

### Fixed

- **`tail` reported the last line of the *first* 8 KiB.** It read input into
  a fixed 8 KiB buffer and stopped there, so on anything longer the answer
  was not the tail of anything. Input is now a ring buffer over the most
  recent 8 KiB, which moves the bound from the input to the answer. No test
  caught this because no racfs file could exceed 4096 bytes until this
  release; `tail` on a large initramfs binary was always wrong.

- **The unsafe-safety gate could pass without reading a line.** Run under a
  bash whose PATH lacks coreutils — Git for Windows' inner `usr\bin\bash.exe`
  does exactly that — `grep` was not found, the scan matched nothing, and the
  script printed `scanned 0 unsafe blocks; 0 missing SAFETY` and exited 0.
  The one outcome a lint must never produce. It now checks for its tools up
  front and treats a scan that found no unsafe blocks at all as a failure,
  since the kernel has hundreds. Its header comment also no longer claims the
  exit code is "always 0"; `--strict` has been a required CI gate since the
  T4.2 backlog cleared.

- **`run-all-gates.ps1` resolved `bash` to WSL's,** which refuses a CRLF
  script outright, so gate 5 hard-failed for an environmental reason on a
  stock Windows box. It now names Git's wrapper `bin\bash.exe` explicitly.

- **`wc` ignored `-c`, `-l` and `-w`,** always printing all three counts, and
  accepted no file operands at all. `n=$(wc -c < f); test "$n" -eq 16402`
  therefore compared `"  258  258  16402"` against a number and failed for a
  reason unrelated to whatever it was testing. Flags now select (and bundle,
  `-lc`), file operands work with a `total` line for more than one, and
  counts print unpadded so a shell can use them.

### Added — v0.3 §3.2 (racfs consistency check)

- **`Racfs::check()` runs when the persistent disk is mounted.** It walks
  every live inode, builds a per-block reference count, and compares it
  against the allocation bitmap and the superblock.

  Findings are separated by whether they can destroy data, because the
  answers differ. Leaked blocks and superblock drift are untidy but safe.
  Blocks a live inode uses while the bitmap calls them free, or blocks two
  inodes both claim, are not: the allocator is a linear bitmap scan, so it
  will hand those blocks out again and the two files will overwrite each
  other. That is precisely the damage that once made a disk in this project
  report `name_len = 111` — ASCII `'o'`, file text sitting in a directory
  block — and take the kernel down with it.

  It reports and does not repair. Repairing a doubly-claimed block means
  choosing which inode owns it, and that belongs to whoever can see which
  file matters, not to a boot-time routine. It also does not refuse to
  mount: that would strand the one shell able to fix the disk. When the
  findings are dangerous the warning says what to do.

  Verified against a genuinely damaged image kept from earlier in this
  project's history, which it diagnosed as `leaked=52 unallocated_in_use=4
  doubly_claimed=1 out_of_range=1 sb_drift=-49` — while still booting to a
  usable shell. A fresh disk reports clean.

- **Documented a real limit it exposed:** the allocation bitmap is a single
  sector, so it can only describe 4096 blocks. `alloc_block` scans exactly
  that range, so on a 16 MiB disk 4096 of 32734 data blocks are reachable
  and the rest is dead space. Not a regression — it has always been so — but
  ROADMAP §3.2's allocator item did not say it, and now both it and the code
  do.

### Added — v0.2 §2.3 (network tools)

- **`ping`** — new `SYS_ICMP_ECHO` (81) wrapping the stack's existing
  `send_icmp_echo`; inventing a raw-socket API for one tool was not worth it.
  The wait runs with interrupts on, like `resolve()` and `sys_connect()`:
  SYSCALL entry clears IF, and without the PIT firing nothing drains the NIC.
  Replies are detected by the reply counter changing rather than by matching
  the sequence number — the ICMP receive path only bumps `echo_replies`. With
  one echo in flight that race is theoretical, but it is real and is recorded
  at the call site. **Under QEMU slirp only the gateway answers ICMP**, so
  `ping 10.0.2.2` replies while `ping example.com` resolves and then reports
  100% loss; that is the emulated network, not the stack.
- **`nc`** — TCP connect and listen on the existing socket syscalls. Relays
  with `poll()` over stdin and the socket together, because userland has no
  threads; on stdin EOF it half-closes with `shutdown(SHUT_WR)` and keeps
  draining, so a piped body is not truncated. TCP only.
- **`/proc/net/tcp` and `/proc/net/udp`** — `tcp::snapshot()` copies rows
  under `try_lock`: formatting text while holding the connection table would
  deadlock against the timer's retransmit path on a single CPU. `/proc/net/udp`
  ships a header and no rows, because the UDP path is connectionless and keeps
  no socket table; the file exists so a parser finds an empty table rather
  than no file.
- **`netstat`** `[-t] [-u]` — concatenates the kernel's already-aligned
  tables rather than re-parsing them, and says so explicitly when they are
  missing.

With this, all three v0.2 feature sections (§2.1, §2.2, §2.3) ship. The
`MILESTONE-V0.2-OK` marker is deliberately **not** emitted yet — see
`docs/ROADMAP.md` §2.4 for the two acceptance criteria that still need
settling.

### Fixed

- **`head -1` and `tail -1` were treated as filenames.** Both tools accepted
  `-n 1` and `-n1` but not the bare `-N` everyone actually types; it fell
  through to the FILE branch and was passed to `open()`, so a valid command
  answered "cannot open file". The same code had been copied into both.
- **`tail -N` returned nothing at all.** The backwards scan counted the
  trailing newline as a line boundary, so for input ending `three\n` the first
  step already satisfied the count and the emitted slice was empty. A trailing
  newline terminates the last line rather than starting a new one; the scan
  now skips it before counting. `tail -0` prints nothing rather than
  everything.

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
