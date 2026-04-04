# ADR-008: Memory Model and Allocator

**Status**: Accepted
**Date**: 2026-04-04

## Context

Memory management is foundational. The physical allocator, virtual memory manager, and kernel heap must be defined before any subsystem that allocates memory.

## Decision

### Physical Memory
- **Frame allocator**: bitmap-based for MVP (simple, correct), buddy allocator for later
- **Frame size**: 4 KiB (standard pages); 2 MiB huge pages post-MVP
- **Memory map**: obtained from UEFI BootServices via BootInfo

### Virtual Memory
- **Page tables**: 4-level (PML4) for x86_64
- **Kernel mapping**: higher-half (starting at 0xFFFF_8000_0000_0000)
- **User space**: 0x0000_0000_0040_0000 to 0x0000_7FFF_FFFF_FFFF
- **Guard pages**: between stack and heap segments
- **Copy-on-write**: post-MVP optimization (needed when fork is added)

### Kernel Heap
- **Slab allocator** for fixed-size kernel objects (PCB, file descriptors, inodes)
- **General-purpose allocator** for variable-size allocations
- **Initial heap size**: 1 MiB, growable

## Alternatives Considered

| Alternative | Reason Rejected |
|------------|-----------------|
| Buddy allocator from start | More complex; bitmap is sufficient and easier to verify |
| No kernel heap (stack-only) | Impractical for dynamic data structures |
| SLAB only | Doesn't handle variable-size allocations |
| Single flat allocator | Poor fragmentation behavior for mixed workloads |

## Consequences

- Bitmap allocator is O(n) for allocation; acceptable for MVP scale
- Higher-half kernel mapping requires careful early page table setup
- Guard pages detect stack overflow/heap corruption
- Slab allocator needs predefined size classes

## Risks

- Memory corruption bugs in allocator are catastrophic (mitigate: extensive tests, Rust safety)
- Running out of physical memory (mitigate: OOM handling from start)

## Rollback

Allocator upgrade (bitmap → buddy) is internal; no ABI or API change.
