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
