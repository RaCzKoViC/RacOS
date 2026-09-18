// RaCore — Split Virtqueue (VirtIO 0.9.5 / legacy layout)
//
// A virtqueue is the producer/consumer ring that a VirtIO device shares
// with the driver. Legacy I/O queue size is device-dictated (read-only), so
// the allocation has to follow the size reported by each device queue. QEMU
// versions in use report either 256 or 1024 entries. The split-ring layout is:
//
//     descriptor table: 16 * queue_size bytes
//     available ring:   4 + queue_size*2 + 2 bytes
//     used ring:        4 + queue_size*8 + 2 bytes, aligned to 4096
//
// We allocate enough contiguous 4 KiB frames for that layout and hand the PFN
// (page frame number = phys_addr >> 12) to the device via the queue-address
// I/O port. The device infers avail/used locations from the reported size.

use core::sync::atomic::{fence, Ordering};

use crate::mm::phys::{self, FRAME_SIZE};

/// Largest legacy split queue this driver is willing to allocate.
///
/// QEMU 8/9 commonly expose 256 entries and QEMU 10 can expose 1024. Keeping
/// a bound prevents a malformed device from forcing an excessive contiguous
/// physical allocation during boot.
pub const MAX_QUEUE_SIZE: usize = 1024;

const AVAIL_HEADER_BYTES: usize = 4;
const USED_HEADER_BYTES: usize = 4;
const EVENT_FIELD_BYTES: usize = 2;

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
}

#[derive(Clone, Copy)]
struct VirtqLayout {
    avail_offset: usize,
    used_offset: usize,
    frame_count: usize,
}

const fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

const fn layout_for(queue_size: u16) -> Option<VirtqLayout> {
    let size = queue_size as usize;
    if size == 0 || size > MAX_QUEUE_SIZE || size & (size - 1) != 0 {
        return None;
    }

    let avail_offset = core::mem::size_of::<VirtqDesc>() * size;
    let avail_bytes = AVAIL_HEADER_BYTES + core::mem::size_of::<u16>() * size + EVENT_FIELD_BYTES;
    let used_offset = align_up(avail_offset + avail_bytes, FRAME_SIZE);
    let used_bytes =
        USED_HEADER_BYTES + core::mem::size_of::<VirtqUsedElem>() * size + EVENT_FIELD_BYTES;
    let frame_count = align_up(used_offset + used_bytes, FRAME_SIZE) / FRAME_SIZE;

    Some(VirtqLayout {
        avail_offset,
        used_offset,
        frame_count,
    })
}

// Compile-time regression checks for the legacy layouts exposed by supported
// QEMU versions. These also guard against accidentally accepting unsafe sizes.
const _: () = {
    let q256 = layout_for(256).unwrap();
    assert!(q256.avail_offset == 4096);
    assert!(q256.used_offset == 8192);
    assert!(q256.frame_count == 3);

    let q1024 = layout_for(1024).unwrap();
    assert!(q1024.avail_offset == 16384);
    assert!(q1024.used_offset == 20480);
    assert!(q1024.frame_count == 8);

    assert!(layout_for(0).is_none());
    assert!(layout_for(255).is_none());
    assert!(layout_for(2048).is_none());
};

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
    InvalidSize,
    NoFreeDescriptors,
}

impl Virtqueue {
    /// Whether a device-reported legacy queue size is safe to use.
    pub const fn supports_size(queue_size: u16) -> bool {
        layout_for(queue_size).is_some()
    }

    /// Allocate a fresh virtqueue for a device-reported size.
    ///
    /// Returns ownership of contiguous frames; freeing is not implemented
    /// (queues live for the lifetime of the kernel).
    pub fn new(queue_size: u16) -> Result<Self, VqError> {
        let layout = layout_for(queue_size).ok_or(VqError::InvalidSize)?;
        let frame =
            phys::alloc_contiguous(layout.frame_count).map_err(|_| VqError::OutOfMemory)?;
        let base_phys = frame.addr();
        let avail_phys = base_phys + layout.avail_offset as u64;
        let used_phys = base_phys + layout.used_offset as u64;

        // Zero the entire device-visible allocation.
        // SAFETY: identity-mapped, exclusive owner.
        unsafe {
            core::ptr::write_bytes(
                base_phys as *mut u8,
                0,
                layout.frame_count * FRAME_SIZE,
            );
        }

        let desc = base_phys as *mut VirtqDesc;
        let avail = avail_phys as *mut VirtqAvail;
        let used = used_phys as *mut VirtqUsed;

        // Build the free-descriptor list: 0 → 1 → … → Q-1 → END.
        // SAFETY: desc points to queue_size valid slots zeroed above.
        unsafe {
            for i in 0..queue_size {
                (*desc.add(i as usize)).next = i + 1;
                (*desc.add(i as usize)).flags = VRING_DESC_F_NEXT;
            }
            // The tail entry has no successor.
            (*desc.add(queue_size as usize - 1)).flags = 0;
            (*desc.add(queue_size as usize - 1)).next = 0;
        }

        Ok(Virtqueue {
            base_phys,
            used_phys,
            desc,
            avail,
            used,
            size: queue_size,
            free_head: 0,
            num_free: queue_size,
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
            let ring = (self.avail as *mut u16).add(2);
            *ring.add(slot) = head;
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
        // SAFETY: the used-ring element array follows its two-u16 header and
        // contains `self.size` entries.
        let elem = unsafe {
            let ring = (self.used as *const u8)
                .add(USED_HEADER_BYTES)
                .cast::<VirtqUsedElem>();
            *ring.add(slot)
        };
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
