// RaCore — Split Virtqueue (VirtIO 0.9.5 / legacy layout)
//
// A virtqueue is the producer/consumer ring that a VirtIO device shares
// with the driver. Legacy I/O queue size is device-dictated (read-only)
// and equals 256 for QEMU virtio-net. The layout is then:
//
//     page 0 (offset 0..4096):      Descriptor table  — 16 * 256 = 4096 B
//     page 1 (offset 4096..8192):   Available ring    — header 4 + 256*2 + 2
//     page 2 (offset 8192..12288):  Used ring         — header 4 + 256*8 + 2
//
// We allocate three contiguous 4 KiB frames per queue and hand the PFN
// (page frame number = phys_addr >> 12) to the device via the
// queue-address I/O port. The device infers avail/used locations from the
// fixed layout above.

use core::sync::atomic::{fence, Ordering};

use crate::mm::phys::{self, FRAME_SIZE};

/// Queue size for QEMU virtio-net legacy: device dictates 256 and the
/// driver cannot lower it through the legacy I/O register.
pub const QUEUE_SIZE: usize = 256;
const QUEUE_FRAMES: usize = 3;

/// Descriptor flags.
pub const VRING_DESC_F_NEXT: u16 = 1;
pub const VRING_DESC_F_WRITE: u16 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtqDesc {
    pub addr: u64, // guest physical address
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

#[repr(C)]
pub struct VirtqAvail {
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; QUEUE_SIZE],
    pub used_event: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtqUsedElem {
    pub id: u32,
    pub len: u32,
}

#[repr(C)]
pub struct VirtqUsed {
    pub flags: u16,
    pub idx: u16,
    pub ring: [VirtqUsedElem; QUEUE_SIZE],
    pub avail_event: u16,
}

/// In-memory handle to a virtqueue. Pointers are physical and identity-mapped.
pub struct Virtqueue {
    base_phys: u64, // 1st page (desc + avail)
    used_phys: u64, // 2nd page (used)
    desc: *mut VirtqDesc,
    avail: *mut VirtqAvail,
    used: *mut VirtqUsed,
    pub size: u16,
    pub free_head: u16, // head of the free-descriptor linked list
    pub num_free: u16,
    pub last_used_idx: u16,
}

unsafe impl Send for Virtqueue {}

#[derive(Debug)]
pub enum VqError {
    OutOfMemory,
    NoFreeDescriptors,
}

impl Virtqueue {
    /// Allocate a fresh virtqueue.
    ///
    /// Returns ownership of two contiguous frames; freeing is not implemented
    /// (queues live for the lifetime of the kernel).
    pub fn new() -> Result<Self, VqError> {
        // 3 contiguous frames: page 0 = desc (exactly 4 KiB),
        //                     page 1 = avail, page 2 = used.
        let frame = phys::alloc_contiguous(QUEUE_FRAMES).map_err(|_| VqError::OutOfMemory)?;
        let base_phys = frame.addr();
        let avail_phys = base_phys + FRAME_SIZE as u64;
        let used_phys = base_phys + (2 * FRAME_SIZE) as u64;

        // Zero the whole 12 KiB region.
        // SAFETY: identity-mapped, exclusive owner.
        unsafe {
            core::ptr::write_bytes(base_phys as *mut u8, 0, QUEUE_FRAMES * FRAME_SIZE);
        }

        let desc = base_phys as *mut VirtqDesc;
        let avail = avail_phys as *mut VirtqAvail;
        let used = used_phys as *mut VirtqUsed;

        // Build the free-descriptor list: 0 → 1 → … → Q-1 → END.
        // SAFETY: desc points to QUEUE_SIZE valid slots zeroed above.
        unsafe {
            for i in 0..(QUEUE_SIZE as u16) {
                (*desc.add(i as usize)).next = i + 1;
                (*desc.add(i as usize)).flags = VRING_DESC_F_NEXT;
            }
            // The tail entry has no successor.
            (*desc.add(QUEUE_SIZE - 1)).flags = 0;
            (*desc.add(QUEUE_SIZE - 1)).next = 0;
        }

        Ok(Virtqueue {
            base_phys,
            used_phys,
            desc,
            avail,
            used,
            size: QUEUE_SIZE as u16,
            free_head: 0,
            num_free: QUEUE_SIZE as u16,
            last_used_idx: 0,
        })
    }

    /// Physical frame number reported to the device (legacy I/O).
    #[inline]
    pub fn pfn(&self) -> u32 {
        (self.base_phys >> 12) as u32
    }

    /// Allocate one descriptor from the free list.
    fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }
        let head = self.free_head;
        // SAFETY: head < size; descriptor table size is QUEUE_SIZE.
        let next = unsafe { (*self.desc.add(head as usize)).next };
        self.free_head = next;
        self.num_free -= 1;
        Some(head)
    }

    /// Return a previously-allocated descriptor chain (single index) to the free list.
    fn free_desc(&mut self, idx: u16) {
        // SAFETY: idx originally came from alloc_desc.
        unsafe {
            let d = self.desc.add(idx as usize);
            (*d).flags = VRING_DESC_F_NEXT;
            (*d).next = self.free_head;
        }
        self.free_head = idx;
        self.num_free += 1;
    }

    /// Free an entire descriptor chain starting at `head`.
    pub fn free_chain(&mut self, head: u16) {
        let mut cur = head;
        loop {
            // SAFETY: chain integrity is maintained by add_buf.
            let (flags, next) = unsafe {
                let d = self.desc.add(cur as usize);
                ((*d).flags, (*d).next)
            };
            let last = (flags & VRING_DESC_F_NEXT) == 0;
            self.free_desc(cur);
            if last {
                break;
            }
            cur = next;
        }
    }

    /// Add a chain of buffers to the virtqueue.
    ///
    /// `bufs`: list of (phys_addr, len, write_only) tuples. The first len entries
    /// are read by the device, the remainder are written by the device. (We
    /// encode that per-buffer via the `write` flag — both layouts are legal.)
    ///
    /// Returns the head descriptor index.
    pub fn add_buf(&mut self, bufs: &[(u64, u32, bool)]) -> Result<u16, VqError> {
        const MAX_CHAIN: usize = 8;
        if bufs.is_empty() || bufs.len() > self.num_free as usize || bufs.len() > MAX_CHAIN {
            return Err(VqError::NoFreeDescriptors);
        }

        // Allocate all descriptors first.
        let mut indices = [0u16; MAX_CHAIN];
        for slot in indices.iter_mut().take(bufs.len()) {
            *slot = self.alloc_desc().ok_or(VqError::NoFreeDescriptors)?;
        }

        // Populate them.
        for (i, &(addr, len, write)) in bufs.iter().enumerate() {
            let last = i + 1 == bufs.len();
            let mut flags = 0;
            if write {
                flags |= VRING_DESC_F_WRITE;
            }
            if !last {
                flags |= VRING_DESC_F_NEXT;
            }
            // SAFETY: indices[i] is a freshly-allocated valid slot.
            unsafe {
                let d = self.desc.add(indices[i] as usize);
                (*d).addr = addr;
                (*d).len = len;
                (*d).flags = flags;
                (*d).next = if last { 0 } else { indices[i + 1] };
            }
        }

        let head = indices[0];

        // Publish the head in the available ring.
        // SAFETY: avail is a valid pointer to a zero-initialised VirtqAvail.
        unsafe {
            let avail = &mut *self.avail;
            let slot = (avail.idx as usize) & (self.size as usize - 1);
            avail.ring[slot] = head;
            // Ensure descriptor writes are visible before idx update.
            fence(Ordering::Release);
            avail.idx = avail.idx.wrapping_add(1);
        }

        Ok(head)
    }

    /// Pop one completed entry from the used ring.
    /// Returns (head_descriptor_index, bytes_written) on success.
    pub fn pop_used(&mut self) -> Option<(u16, u32)> {
        // SAFETY: used is a valid pointer.
        let used_idx = unsafe { core::ptr::read_volatile(&(*self.used).idx) };
        if used_idx == self.last_used_idx {
            return None;
        }
        fence(Ordering::Acquire);
        let slot = (self.last_used_idx as usize) & (self.size as usize - 1);
        // SAFETY: same.
        let elem = unsafe { (*self.used).ring[slot] };
        self.last_used_idx = self.last_used_idx.wrapping_add(1);
        Some((elem.id as u16, elem.len))
    }

    #[inline]
    pub fn base_phys(&self) -> u64 {
        self.base_phys
    }
    #[inline]
    pub fn used_phys(&self) -> u64 {
        self.used_phys
    }
}
