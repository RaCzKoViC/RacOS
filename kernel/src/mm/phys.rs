// RaCore — Physical Frame Allocator (Bitmap)
//
// Manages physical memory frames using a bitmap allocator.
// Each bit represents one 4 KiB frame: 0 = free, 1 = allocated.
//
// Design decisions (ADR-008):
// - Bitmap-based for MVP (simple, correct)
// - 4 KiB frames only (huge pages post-MVP)
// - Memory map from UEFI BootInfo
//
// Invariants:
// - A frame cannot be double-allocated
// - A frame cannot be double-freed
// - Only usable memory regions are tracked

use core::sync::atomic::{AtomicU64, Ordering};

/// Size of a physical page frame.
pub const FRAME_SIZE: usize = 4096;

/// Maximum physical memory supported: 4 GiB (1M frames).
/// This keeps the bitmap at 128 KiB — fits in static allocation.
const MAX_FRAMES: usize = 1024 * 1024;
const BITMAP_SIZE: usize = MAX_FRAMES / 64;

/// Physical frame number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysFrame(pub u64);

impl PhysFrame {
    /// Physical address of this frame.
    #[inline]
    pub fn addr(self) -> u64 {
        self.0 * FRAME_SIZE as u64
    }

    /// Frame containing the given physical address.
    #[inline]
    pub fn containing(addr: u64) -> Self {
        PhysFrame(addr / FRAME_SIZE as u64)
    }
}

/// Error type for frame allocation.
#[derive(Debug)]
pub enum FrameError {
    OutOfMemory,
    DoubleFree(PhysFrame),
    OutOfRange(PhysFrame),
}

/// Bitmap-based physical frame allocator.
///
/// Uses atomic operations for future SMP safety, though MVP is UP-only.
pub struct BitmapAllocator {
    bitmap: [AtomicU64; BITMAP_SIZE],
    total_frames: usize,
    usable_frames: usize,
}

// SAFETY: AtomicU64 is Send+Sync, and BitmapAllocator only accesses
// its own bitmap through atomic operations.
unsafe impl Send for BitmapAllocator {}
unsafe impl Sync for BitmapAllocator {}

impl BitmapAllocator {
    /// Create a new allocator with all frames marked as allocated (reserved).
    /// Call `mark_usable` to free regions from the memory map.
    pub const fn new() -> Self {
        // Start with all bits set to 1 (allocated/reserved).
        // The memory map will clear usable regions.
        const FULL: AtomicU64 = AtomicU64::new(!0u64);
        BitmapAllocator {
            bitmap: [FULL; BITMAP_SIZE],
            total_frames: 0,
            usable_frames: 0,
        }
    }

    /// Mark a range of frames as usable (free).
    /// Called during init with each usable region from the memory map.
    ///
    /// `base` and `size` are in bytes and will be aligned to frame boundaries.
    pub fn mark_usable(&mut self, base: u64, size: u64) {
        if size == 0 {
            return;
        }

        // Align base up and end down to frame boundaries
        let start_frame = ((base + FRAME_SIZE as u64 - 1) / FRAME_SIZE as u64) as usize;
        let end_frame = ((base + size) / FRAME_SIZE as u64) as usize;

        if start_frame >= MAX_FRAMES {
            return;
        }
        let end_frame = end_frame.min(MAX_FRAMES);

        for frame in start_frame..end_frame {
            let idx = frame / 64;
            let bit = frame % 64;
            self.bitmap[idx].fetch_and(!(1u64 << bit), Ordering::Relaxed);
            self.usable_frames += 1;
        }
        self.total_frames = self.total_frames.max(end_frame);
    }

    /// Mark a range of frames as reserved (allocated).
    /// Used to protect kernel, BootInfo, framebuffer, etc.
    pub fn mark_reserved(&mut self, base: u64, size: u64) {
        if size == 0 {
            return;
        }

        let start_frame = (base / FRAME_SIZE as u64) as usize;
        let end_frame =
            (((base + size + FRAME_SIZE as u64 - 1) / FRAME_SIZE as u64) as usize).min(MAX_FRAMES);

        for frame in start_frame..end_frame {
            let idx = frame / 64;
            let bit = frame % 64;
            let prev = self.bitmap[idx].fetch_or(1u64 << bit, Ordering::Relaxed);
            if prev & (1u64 << bit) == 0 {
                // Was free, now reserved — decrement usable count
                self.usable_frames = self.usable_frames.saturating_sub(1);
            }
        }
    }

    /// Allocate a single physical frame.
    /// Returns the frame number on success.
    pub fn alloc(&self) -> Result<PhysFrame, FrameError> {
        let search_limit = (self.total_frames + 63) / 64;
        for idx in 0..search_limit.min(BITMAP_SIZE) {
            let val = self.bitmap[idx].load(Ordering::Relaxed);
            if val == !0u64 {
                continue; // All 64 bits set — no free frames here
            }

            // Find first zero bit
            let bit = (!val).trailing_zeros() as usize;
            let frame = idx * 64 + bit;
            if frame >= self.total_frames {
                break;
            }

            // Try to set the bit atomically (CAS for correctness)
            let mask = 1u64 << bit;
            let prev = self.bitmap[idx].fetch_or(mask, Ordering::AcqRel);
            if prev & mask == 0 {
                // Successfully allocated
                return Ok(PhysFrame(frame as u64));
            }
            // Bit was already set by another caller — try next
        }
        Err(FrameError::OutOfMemory)
    }

    /// Allocate `count` contiguous physical frames.
    ///
    /// Two-phase scan:
    ///   1. Find a candidate run of `count` consecutive free frames by reading
    ///      the bitmap.
    ///   2. Atomically set each bit. If any was already set (concurrent alloc
    ///      on SMP — not possible on UP but cheap insurance), undo and search
    ///      again starting past the conflict.
    ///
    /// The previous version interleaved scanning and claiming inside a single
    /// loop. That worked for the happy path but tracked `run_start` and the
    /// outer loop counter independently — after a CAS race-back-out the two
    /// could end up describing overlapping regions, so the next CAS attempt
    /// would silently corrupt a previously-allocated run (the back-out
    /// `fetch_and` would clear bits that another allocation owned). On UP that
    /// looked like: a 512-frame user-stack alloc returns successfully but a
    /// later 16-frame alloc finds free frames *inside* the 512-frame range
    /// because some bits had been cleared by the buggy back-out. Symptom: user
    /// stack and kernel stack shared physical memory, child processes silently
    /// hung after iretq because their kernel stack was being overwritten by
    /// their own user stack writes (or vice versa).
    pub fn alloc_contiguous(&self, count: usize) -> Result<PhysFrame, FrameError> {
        if count == 0 {
            return Err(FrameError::OutOfMemory);
        }
        if count == 1 {
            return self.alloc();
        }

        let total = self.total_frames;
        if count > total {
            return Err(FrameError::OutOfMemory);
        }

        let mut search_from = 0usize;

        'outer: loop {
            // Phase 1: scan for a run of `count` free frames starting at or
            // after `search_from`.
            let mut run_start = search_from;
            let mut run_len = 0usize;
            let mut found_start = None;
            let mut frame = search_from;
            while frame < total {
                let idx = frame / 64;
                let bit = frame % 64;
                let allocated = self.bitmap[idx].load(Ordering::Relaxed) & (1u64 << bit) != 0;
                if allocated {
                    run_start = frame + 1;
                    run_len = 0;
                } else {
                    run_len += 1;
                    if run_len == count {
                        found_start = Some(run_start);
                        break;
                    }
                }
                frame += 1;
            }
            let start = match found_start {
                Some(s) => s,
                None => return Err(FrameError::OutOfMemory),
            };

            // Phase 2: atomically claim each bit. If any was already set by a
            // racing allocator, undo our claims and resume scanning past the
            // conflict.
            let mut claimed_through = start;
            let mut conflict_at: Option<usize> = None;
            for f in start..start + count {
                let i = f / 64;
                let b = f % 64;
                let prev = self.bitmap[i].fetch_or(1u64 << b, Ordering::AcqRel);
                if prev & (1u64 << b) != 0 {
                    conflict_at = Some(f);
                    break;
                }
                claimed_through = f + 1;
            }

            if let Some(conflict) = conflict_at {
                // Undo only the bits *we* set this round. Bits at and beyond
                // `conflict` are owned by whoever beat us — never touch them.
                for f2 in start..claimed_through {
                    let i2 = f2 / 64;
                    let b2 = f2 % 64;
                    self.bitmap[i2].fetch_and(!(1u64 << b2), Ordering::Release);
                }
                search_from = conflict + 1;
                if search_from + count > total {
                    return Err(FrameError::OutOfMemory);
                }
                continue 'outer;
            }

            return Ok(PhysFrame(start as u64));
        }
    }

    /// Free a previously allocated frame.
    pub fn free(&self, frame: PhysFrame) -> Result<(), FrameError> {
        let f = frame.0 as usize;
        if f >= MAX_FRAMES {
            return Err(FrameError::OutOfRange(frame));
        }
        let idx = f / 64;
        let bit = f % 64;
        let mask = 1u64 << bit;
        let prev = self.bitmap[idx].fetch_and(!mask, Ordering::AcqRel);
        if prev & mask == 0 {
            // Was already free — double free
            // Re-mark as free (idempotent) but report error
            return Err(FrameError::DoubleFree(frame));
        }
        Ok(())
    }

    /// Number of usable frames at init time.
    pub fn usable_frame_count(&self) -> usize {
        self.usable_frames
    }

    /// Number of currently free frames (expensive — scans bitmap).
    pub fn free_frame_count(&self) -> usize {
        let mut count = 0usize;
        let limit = (self.total_frames + 63) / 64;
        for idx in 0..limit.min(BITMAP_SIZE) {
            let val = self.bitmap[idx].load(Ordering::Relaxed);
            count += val.count_zeros() as usize;
        }
        // Subtract unused bits in the last u64
        let remainder = self.total_frames % 64;
        if remainder != 0 {
            count -= 64 - remainder;
        }
        count
    }
}

/// Global physical frame allocator instance.
///
/// Initialized in `init_from_memory_map`. Accesses after init are safe
/// because the bitmap uses atomic operations.
static mut FRAME_ALLOCATOR: BitmapAllocator = BitmapAllocator::new();

/// Initialize the physical frame allocator from the boot memory map.
///
/// # Safety
/// Must be called exactly once during early kernel init, before any
/// other code calls `alloc_frame`.
pub unsafe fn init_from_memory_map(entries: *const crate::boot::MemoryMapEntry, count: u64) {
    let allocator = &mut *core::ptr::addr_of_mut!(FRAME_ALLOCATOR);

    for i in 0..count as usize {
        // SAFETY: entries pointer and count validated by bootloader
        let entry = &*entries.add(i);
        if entry.mem_type == crate::boot::MemoryType::Usable {
            allocator.mark_usable(entry.base, entry.size);
        }
    }

    crate::serial::serial_println!(
        "[  0.000120] RACORE: Physical allocator ready ({} usable frames, {} MiB)",
        allocator.usable_frame_count(),
        (allocator.usable_frame_count() * FRAME_SIZE) / (1024 * 1024)
    );
}

/// Reserve frames that the kernel or bootloader occupy.
///
/// # Safety
/// Must be called after `init_from_memory_map` and before general allocation.
pub unsafe fn reserve_range(base: u64, size: u64) {
    let allocator = &mut *core::ptr::addr_of_mut!(FRAME_ALLOCATOR);
    allocator.mark_reserved(base, size);
}

/// Allocate one physical frame.
pub fn alloc_frame() -> Result<PhysFrame, FrameError> {
    // SAFETY: FRAME_ALLOCATOR is initialized during early boot.
    // Atomic operations ensure correctness for concurrent access.
    unsafe { (*core::ptr::addr_of!(FRAME_ALLOCATOR)).alloc() }
}

/// Allocate `count` contiguous physical frames.
pub fn alloc_contiguous(count: usize) -> Result<PhysFrame, FrameError> {
    unsafe { (*core::ptr::addr_of!(FRAME_ALLOCATOR)).alloc_contiguous(count) }
}

/// Free a physical frame.
pub fn free_frame(frame: PhysFrame) -> Result<(), FrameError> {
    unsafe { (*core::ptr::addr_of!(FRAME_ALLOCATOR)).free(frame) }
}

/// Number of free frames.
pub fn free_count() -> usize {
    unsafe { (*core::ptr::addr_of!(FRAME_ALLOCATOR)).free_frame_count() }
}

/// Total number of usable frames.
pub fn total_count() -> usize {
    unsafe { (*core::ptr::addr_of!(FRAME_ALLOCATOR)).usable_frame_count() }
}
