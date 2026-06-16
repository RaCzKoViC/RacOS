# ADR-009: VFS Model

**Status**: Accepted
**Date**: 2026-04-04

## Context

A Virtual File System (VFS) layer abstracts filesystem implementations from the rest of the kernel and from user space. Without VFS, every filesystem would need its own syscall handling.

## Decision

RaCore uses a VFS layer with:
- **Inode** abstraction (metadata, operations)
- **File** abstraction (open file state, offset, operations)
- **File descriptor** table per process
- **Dentry cache** for path lookup (simple hash map for MVP)
- **Mount table** tracking filesystem mounts

VFS dispatches operations to registered filesystem implementations.

### Filesystems for v1.0:
1. **initramfs** — read-only, for early boot
2. **tmpfs** — in-memory read-write, for /tmp and runtime state
3. **racfs** — persistent read-write filesystem (simple design: extent-based, journaling post-MVP)

### Special filesystems:
- **racprocfs** — process info (mounted at /proc equivalent)
- **racsysfs** — system info (mounted at /sys equivalent)
- **devfs** — device nodes (mounted at /dev)

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| No VFS (direct FS calls) | Unmaintainable, every FS duplicates syscall handling |
| Plan 9 style (everything is a file server) | Too experimental for v1 |
| ext2/ext4 support from start | Complex; own simple FS is faster to implement and fully controllable |

## Consequences

- All file operations go through VFS → filesystem driver
- Adding new filesystems requires implementing the VFS trait/interface
- Path resolution is centralized in VFS
- Mount points are explicit and tracked

## Risks

- racfs design may need revision after real usage (acceptable if journaling is deferred)
- Dentry cache memory growth (mitigate: bounded cache with LRU eviction)

## Rollback

Individual filesystem implementations can be replaced without changing VFS or syscall layer.

## Implementation status (2026-06-16)

The VFS layer is in place and load-bearing. Most of §Decision is shipped; the racprocfs vs racsysfs split was collapsed into procfs-only.

**Shipped:**
* `Inode` + `OpenFile` + `FdTable` + `mount_table` singleton in `kernel/src/vfs/`. Operations dispatched through the `Filesystem` and `InodeOps` traits.
* Path resolution via `mount_table().resolve()` (longest-prefix match) and per-FS `lookup_path` — no separate dentry cache; the per-FS implementations cache their own metadata where it helps (`racfs::metadata_cache`, `fat32::metadata_cache`).
* Five built-in filesystems mounted at boot from `kernel/src/main.rs:217-318`:
  - **initramfs** at `/` (read-only, from the bootloader-supplied binary blob)
  - **devfs** at `/dev` (`/dev/null`, `/dev/zero`, `/dev/console`)
  - **tmpfs** at `/tmp` (in-memory R/W)
  - **procfs** at `/proc` (`status`, `cmdline`, `cpuinfo`, `uptime`, `mounts`, `cachestats`, `diskstats`)
  - **racfs** at `/var` (ramdisk-backed, ephemeral)
* Two extra filesystems mounted opportunistically:
  - **fat32** at `/fat` (volatile, formatted fresh each boot on ram1) — `kernel/src/main.rs:296`
  - **racfs** at `/mnt` (persistent, on AHCI sda) — `kernel/src/main.rs:322`
* Block device abstraction in `kernel/src/drivers/block.rs`; AHCI driver in `drivers/ahci.rs`; two-boot persistence smoke in CI (T2.1).
* `sys_mount` / `sys_umount` / `sys_mkfs` accept user-space mount requests for tmpfs/racfs/proc/dev/fat32, gated on `CAP_SYS_ADMIN`.

**Still deferred / different from §Decision:**
* **racsysfs** was never built. `/sys` doesn't exist; the system-info bits §Decision put under racsysfs ended up in procfs (cpuinfo, uptime, etc.).
* No separate dentry cache. The hash-map design from §Decision was replaced by per-FS metadata caches plus mount-table resolution, which is faster for the current small-tree workload but doesn't help directory lookups across mount points.
* racfs journaling is still deferred — racfs writes superblock + extent allocator updates directly. The kernel-side flushd daemon (T2.1, `kernel/src/main.rs:351-362`) flushes dirty cache entries periodically as a partial mitigation.
* FAT32 supports R/W but the on-disk dir-walk has a cycle guard added after a chase bug — a real journal is post-MVP.
