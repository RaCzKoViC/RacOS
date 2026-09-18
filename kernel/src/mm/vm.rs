//! Per-process record of what is mapped where: the address space as the
//! process is allowed to see it.
//!
//! The page table says which pages exist; this says which ranges the
//! process owns, with what access, and where the next anonymous mapping
//! may go. Until 2026-09 there was no such record: `mmap` mapped a
//! caller-supplied address anywhere (over a live mapping, over the
//! identity-mapped kernel), handed out addresses from one global cursor
//! shared by every process and never reused, `munmap` freed the frames of
//! any present page - kernel pages included - and `mprotect` returned 0
//! without doing anything.
//!
//! Every user mapping the kernel creates - the ELF segments and the stack
//! at exec, anonymous mappings from mmap - is an area here, so `munmap` and
//! `mprotect` can be answered with "not yours" (EINVAL / ENOMEM) instead of
//! being obeyed. The record is per address space: a fork gets a copy, a
//! CLONE_VM thread shares its parent's through the `Arc`.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use crate::sync::with_irqs_off;

pub const PAGE_SIZE: u64 = 4096;

/// Lowest address a user mapping may start at. Everything below is the
/// identity-mapped physical memory the kernel lives in; user segments start
/// at 4 GiB and the stack sits just under the canonical limit.
pub const USER_FLOOR: u64 = 0x0000_0001_0000_0000;
/// One past the highest user address (canonical lower half).
pub const USER_CEILING: u64 = 0x0000_8000_0000_0000;
/// Where anonymous mappings are placed when the caller gives no address:
/// downwards from here, below the stack.
pub const MMAP_TOP: u64 = 0x0000_7FF0_0000_0000;

/// PROT_* bits as user space passes them.
pub const PROT_READ: u32 = 1;
pub const PROT_WRITE: u32 = 2;
pub const PROT_EXEC: u32 = 4;
pub const PROT_MASK: u32 = PROT_READ | PROT_WRITE | PROT_EXEC;

/// What a mapping is, for diagnostics and for the rules that differ by
/// kind (none yet beyond naming).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AreaKind {
    /// A loaded ELF segment.
    Elf,
    /// The initial user stack.
    Stack,
    /// An anonymous mmap.
    Anon,
}

/// One contiguous mapped range `[start, end)`, page-aligned.
#[derive(Clone, Copy, Debug)]
pub struct VmArea {
    pub start: u64,
    pub end: u64,
    pub prot: u32,
    pub kind: AreaKind,
}

impl VmArea {
    pub fn len(&self) -> u64 {
        self.end - self.start
    }
}

/// The areas of one address space, sorted by start, non-overlapping.
#[derive(Clone, Default)]
pub struct VmSpace {
    areas: Vec<VmArea>,
}

/// Why an area could not be added.
#[derive(Debug, PartialEq, Eq)]
pub enum VmError {
    /// Not page-aligned, zero-length, or outside the user range.
    Invalid,
    /// Overlaps an existing area.
    Overlap,
}

impl VmSpace {
    pub fn new() -> Self {
        VmSpace { areas: Vec::new() }
    }

    pub fn areas(&self) -> &[VmArea] {
        &self.areas
    }

    /// True if `[start, end)` is a page-aligned, non-empty range inside
    /// the user address space.
    pub fn range_is_valid(start: u64, end: u64) -> bool {
        start.is_multiple_of(PAGE_SIZE)
            && end.is_multiple_of(PAGE_SIZE)
            && start < end
            && start >= USER_FLOOR
            && end <= USER_CEILING
    }

    /// The area containing `addr`, if any.
    pub fn find(&self, addr: u64) -> Option<&VmArea> {
        self.areas.iter().find(|a| a.start <= addr && addr < a.end)
    }

    /// True if any area intersects `[start, end)`.
    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        self.areas.iter().any(|a| a.start < end && start < a.end)
    }

    /// True if every page of `[start, end)` lies inside some area.
    pub fn covers(&self, start: u64, end: u64) -> bool {
        let mut at = start;
        while at < end {
            match self.find(at) {
                Some(a) => at = a.end,
                None => return false,
            }
        }
        true
    }

    /// Record a new area. The range must be valid and free.
    pub fn insert(&mut self, area: VmArea) -> Result<(), VmError> {
        if !Self::range_is_valid(area.start, area.end) {
            return Err(VmError::Invalid);
        }
        if self.overlaps(area.start, area.end) {
            return Err(VmError::Overlap);
        }
        let pos = self
            .areas
            .iter()
            .position(|a| a.start > area.start)
            .unwrap_or(self.areas.len());
        self.areas.insert(pos, area);
        Ok(())
    }

    /// Record `area`, leaving out whatever part of it is already covered.
    /// For the ELF segments at exec: two segments may share a page at the
    /// boundary, and the page is the process's either way.
    pub fn insert_uncovered(&mut self, area: VmArea) -> Result<(), VmError> {
        if !Self::range_is_valid(area.start, area.end) {
            return Err(VmError::Invalid);
        }
        let mut at = area.start;
        while at < area.end {
            match self.find(at) {
                Some(a) => at = a.end,
                None => {
                    let next = self
                        .areas
                        .iter()
                        .filter(|a| a.start > at)
                        .map(|a| a.start)
                        .min()
                        .unwrap_or(area.end)
                        .min(area.end);
                    self.insert(VmArea {
                        start: at,
                        end: next,
                        prot: area.prot,
                        kind: area.kind,
                    })?;
                    at = next;
                }
            }
        }
        Ok(())
    }

    /// Highest-placed free gap of `len` bytes below `top`, or None. Areas
    /// are walked from the top down, so an unmapped range is found again
    /// by the next mapping of the same size - the space is reused, not
    /// consumed.
    pub fn find_free_below(&self, len: u64, top: u64) -> Option<u64> {
        let mut limit = top.min(USER_CEILING);
        for a in self.areas.iter().rev() {
            if a.end <= limit {
                if limit - a.end >= len && limit - len >= USER_FLOOR {
                    return Some(limit - len);
                }
                limit = a.start;
            } else if a.start < limit {
                // The area straddles the current limit; continue below it.
                limit = a.start;
            }
        }
        if limit >= USER_FLOOR + len {
            Some(limit - len)
        } else {
            None
        }
    }

    /// Cut `[start, end)` out of the areas that cover it, splitting an
    /// area when the range lands in its middle. The caller checked
    /// `covers` first; pages outside any area are simply not there.
    pub fn remove_range(&mut self, start: u64, end: u64) {
        let mut out: Vec<VmArea> = Vec::with_capacity(self.areas.len() + 1);
        for a in self.areas.iter() {
            if a.end <= start || a.start >= end {
                out.push(*a);
                continue;
            }
            if a.start < start {
                out.push(VmArea {
                    start: a.start,
                    end: start,
                    prot: a.prot,
                    kind: a.kind,
                });
            }
            if a.end > end {
                out.push(VmArea {
                    start: end,
                    end: a.end,
                    prot: a.prot,
                    kind: a.kind,
                });
            }
        }
        self.areas = out;
    }

    /// Give every page of `[start, end)` the access `prot`, splitting the
    /// areas at the boundaries. The caller checked `covers` first.
    pub fn set_prot(&mut self, start: u64, end: u64, prot: u32) {
        let mut out: Vec<VmArea> = Vec::with_capacity(self.areas.len() + 2);
        for a in self.areas.iter() {
            if a.end <= start || a.start >= end {
                out.push(*a);
                continue;
            }
            if a.start < start {
                out.push(VmArea {
                    start: a.start,
                    end: start,
                    prot: a.prot,
                    kind: a.kind,
                });
            }
            out.push(VmArea {
                start: a.start.max(start),
                end: a.end.min(end),
                prot,
                kind: a.kind,
            });
            if a.end > end {
                out.push(VmArea {
                    start: end,
                    end: a.end,
                    prot: a.prot,
                    kind: a.kind,
                });
            }
        }
        self.areas = out;
    }
}

/// The address-space record a task points at. Shared by the threads of one
/// process, owned by one process otherwise. Accessed only inside
/// `with`, which disables interrupts: the scheduler cannot interleave two
/// tasks' updates to one record on this single CPU.
pub struct VmSpaceCell(UnsafeCell<VmSpace>);

// SAFETY: every access goes through `with`, which runs with interrupts off
// on a single CPU; there is no concurrent access to the inner value.
unsafe impl Send for VmSpaceCell {}
// SAFETY: as above.
unsafe impl Sync for VmSpaceCell {}

impl VmSpaceCell {
    pub fn new(space: VmSpace) -> Arc<Self> {
        Arc::new(VmSpaceCell(UnsafeCell::new(space)))
    }

    /// Run `f` on the record with interrupts off.
    pub fn with<R>(&self, f: impl FnOnce(&mut VmSpace) -> R) -> R {
        with_irqs_off(|| {
            // SAFETY: interrupts are off and this kernel runs one CPU, so no
            // other task can be inside this cell at the same time.
            let space = unsafe { &mut *self.0.get() };
            f(space)
        })
    }

    /// A deep copy for a forked child.
    pub fn duplicate(&self) -> Arc<Self> {
        let copy = self.with(|s| s.clone());
        Self::new(copy)
    }
}
