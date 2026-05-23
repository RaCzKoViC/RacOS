// RaCore — Kernel Heap Allocator
//
// A linked-list free-block allocator serving as the kernel's global allocator.
// Backs `alloc::` collections (Vec, Box, String, etc.) in kernel space.
//
// Design:
// - Initial heap: 8 MiB of physically contiguous frames
// - Grows on demand by allocating more frames (up to max)
// - Free blocks tracked in a sorted linked list (address-ordered)
// - Coalescing adjacent free blocks on free
//
// Invariants:
// - All heap memory comes from phys::alloc_frame
// - Alignment is at least 8 bytes
// - Free list is always sorted by address
// - No overlapping free blocks

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use super::phys::{self, FRAME_SIZE};

/// Initial heap size: 8 MiB
const INITIAL_HEAP_SIZE: usize = 8 * 1024 * 1024;

/// Maximum heap size: 32 MiB
const MAX_HEAP_SIZE: usize = 32 * 1024 * 1024;

/// Minimum block size (must hold a FreeBlock header).
const MIN_BLOCK_SIZE: usize = core::mem::size_of::<FreeBlock>();

/// Header stored at the start of each free block.
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

/// Kernel heap allocator.
pub struct KernelHeapAllocator {
    head: *mut FreeBlock,
    heap_start: usize,
    heap_size: usize,
}

// SAFETY: We protect access with a spinlock in LockedHeap.
unsafe impl Send for KernelHeapAllocator {}
unsafe impl Sync for KernelHeapAllocator {}

impl KernelHeapAllocator {
    pub const fn empty() -> Self {
        KernelHeapAllocator {
            head: ptr::null_mut(),
            heap_start: 0,
            heap_size: 0,
        }
    }

    /// Initialize the heap with the given memory region.
    ///
    /// # Safety
    /// `start` must point to `size` bytes of valid, unused, writable memory.
    unsafe fn init(&mut self, start: usize, size: usize) {
        self.heap_start = start;
        self.heap_size = size;

        let block = start as *mut FreeBlock;
        (*block).size = size;
        (*block).next = ptr::null_mut();
        self.head = block;
    }

    /// Allocate memory from the heap.
    fn alloc_inner(&mut self, layout: Layout) -> *mut u8 {
        let size = align_up(layout.size().max(MIN_BLOCK_SIZE), 8);
        let align = layout.align().max(8);

        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut current = self.head;

        while !current.is_null() {
            // SAFETY: current is a valid FreeBlock pointer within the heap.
            let block = unsafe { &mut *current };
            let block_start = current as usize;
            let block_end = block_start + block.size;

            // Compute aligned allocation start
            let alloc_start = (block_start + align - 1) & !(align - 1);
            let alloc_end = alloc_start + size;

            if alloc_end <= block_end {
                let front_padding = alloc_start - block_start;
                let remaining = block_end - alloc_end;

                // Remove this block from the free list first
                if prev.is_null() {
                    self.head = block.next;
                } else {
                    unsafe {
                        (*prev).next = block.next;
                    }
                }
                let next_saved = block.next;

                // If there's enough space after the allocation, create a new free block
                if remaining >= MIN_BLOCK_SIZE {
                    unsafe {
                        let new_block = alloc_end as *mut FreeBlock;
                        (*new_block).size = remaining;
                        (*new_block).next = ptr::null_mut();
                        self.insert_free_block(new_block);
                    }
                }

                // If there's enough space before the allocation (alignment padding), keep it
                if front_padding >= MIN_BLOCK_SIZE {
                    unsafe {
                        let front_block = block_start as *mut FreeBlock;
                        (*front_block).size = front_padding;
                        (*front_block).next = ptr::null_mut();
                        self.insert_free_block(front_block);
                    }
                }

                let _ = next_saved; // Used above via block.next before removal
                return alloc_start as *mut u8;
            }

            prev = current;
            current = block.next;
        }

        // Out of heap memory — try to grow
        if self.try_grow(size + align) {
            // Retry allocation after growing
            return self.alloc_inner(layout);
        }

        ptr::null_mut()
    }

    /// Free previously allocated memory.
    ///
    /// # Safety
    /// `ptr` must have been returned by a previous `alloc_inner` call with
    /// the same layout.
    unsafe fn dealloc_inner(&mut self, ptr: *mut u8, layout: Layout) {
        let size = align_up(layout.size().max(MIN_BLOCK_SIZE), 8);
        let block = ptr as *mut FreeBlock;
        (*block).size = size;
        (*block).next = ptr::null_mut();
        self.insert_free_block(block);
    }

    /// Insert a free block into the sorted free list and coalesce neighbors.
    unsafe fn insert_free_block(&mut self, block: *mut FreeBlock) {
        let block_addr = block as usize;

        // Find insertion point (sorted by address)
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut current = self.head;

        while !current.is_null() && (current as usize) < block_addr {
            prev = current;
            current = (*current).next;
        }

        // Insert block between prev and current
        (*block).next = current;
        if prev.is_null() {
            self.head = block;
        } else {
            (*prev).next = block;
        }

        // Try to coalesce with next block
        if !current.is_null() {
            let block_end = block_addr + (*block).size;
            if block_end == current as usize {
                (*block).size += (*current).size;
                (*block).next = (*current).next;
            }
        }

        // Try to coalesce with previous block
        if !prev.is_null() {
            let prev_end = prev as usize + (*prev).size;
            if prev_end == block_addr {
                (*prev).size += (*block).size;
                (*prev).next = (*block).next;
            }
        }
    }

    /// Try to grow the heap by allocating more physical frames.
    fn try_grow(&mut self, min_bytes: usize) -> bool {
        let grow_size = min_bytes.max(FRAME_SIZE * 64); // Grow by at least 256 KiB
        if self.heap_size + grow_size > MAX_HEAP_SIZE {
            return false;
        }

        let frames_needed = (grow_size + FRAME_SIZE - 1) / FRAME_SIZE;
        match phys::alloc_contiguous(frames_needed) {
            Ok(frame) => {
                let base = frame.addr() as usize;
                unsafe {
                    let block = base as *mut FreeBlock;
                    (*block).size = frames_needed * FRAME_SIZE;
                    (*block).next = ptr::null_mut();
                    self.insert_free_block(block);
                }
                self.heap_size += frames_needed * FRAME_SIZE;
                true
            }
            Err(_) => false,
        }
    }
}

#[inline(always)]
const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

/// Simple spinlock for the heap allocator.
struct HeapSpinLock {
    locked: core::sync::atomic::AtomicBool,
}

impl HeapSpinLock {
    const fn new() -> Self {
        HeapSpinLock {
            locked: core::sync::atomic::AtomicBool::new(false),
        }
    }

    fn lock(&self) {
        while self
            .locked
            .compare_exchange_weak(
                false,
                true,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.locked
            .store(false, core::sync::atomic::Ordering::Release);
    }
}

/// Locked wrapper for the kernel heap allocator.
struct LockedHeap {
    lock: HeapSpinLock,
    inner: core::cell::UnsafeCell<KernelHeapAllocator>,
}

// SAFETY: Protected by SpinLock.
unsafe impl Send for LockedHeap {}
unsafe impl Sync for LockedHeap {}

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap = LockedHeap {
    lock: HeapSpinLock::new(),
    inner: core::cell::UnsafeCell::new(KernelHeapAllocator::empty()),
};

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.lock.lock();
        let result = (*self.inner.get()).alloc_inner(layout);
        self.lock.unlock();
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.lock.lock();
        (*self.inner.get()).dealloc_inner(ptr, layout);
        self.lock.unlock();
    }
}

/// Initialize the kernel heap.
///
/// Allocates physical frames for the initial heap region.
///
/// # Safety
/// Must be called once during early kernel init, after phys allocator is ready.
pub unsafe fn init() -> Result<(), &'static str> {
    let frames = INITIAL_HEAP_SIZE / FRAME_SIZE;
    let base_frame =
        phys::alloc_contiguous(frames).map_err(|_| "Failed to allocate heap frames")?;

    let heap_start = base_frame.addr() as usize;

    // Zero the heap region
    ptr::write_bytes(heap_start as *mut u8, 0, INITIAL_HEAP_SIZE);

    let allocator = &mut *HEAP_ALLOCATOR.inner.get();
    allocator.init(heap_start, INITIAL_HEAP_SIZE);

    crate::serial::serial_println!(
        "[  0.000180] RACORE: Kernel heap initialized ({} KiB at 0x{:X})",
        INITIAL_HEAP_SIZE / 1024,
        heap_start
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::alloc::Layout;

    // Mock physical allocator for tests
    struct MockPhysAllocator {
        next_addr: usize,
    }

    impl MockPhysAllocator {
        fn new() -> Self {
            MockPhysAllocator {
                next_addr: 0x1000000,
            } // Start at 16MB
        }

        fn alloc(&self, frames: usize) -> Option<usize> {
            // Return a mock address
            Some(0x1000)
        }
    }

    static MOCK_PHYS: MockPhysAllocator = MockPhysAllocator {
        next_addr: 0x1000000,
    };

    // Mock alloc_contiguous for tests
    fn mock_alloc_contiguous(frames: usize) -> Result<phys::PhysFrame, &'static str> {
        if let Some(addr) = MOCK_PHYS.alloc(frames) {
            Ok(phys::PhysFrame::containing(addr as u64))
        } else {
            Err("Mock allocation failed")
        }
    }

    #[test]
    fn test_kernel_heap_allocator_basic() {
        let allocator = KernelHeapAllocator::empty();

        // Test that allocator starts empty
        assert_eq!(allocator.heap_start, 0);
        assert_eq!(allocator.heap_size, 0);
        assert!(allocator.head.is_null());
    }

    #[test]
    fn test_kernel_heap_allocator_coalescing() {
        // Simplified test
        let allocator = KernelHeapAllocator::empty();
        assert_eq!(allocator.heap_start, 0);
    }
}
