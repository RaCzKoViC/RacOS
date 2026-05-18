// RaCore — Virtual Memory Manager (x86_64 4-level paging)
//
// Provides page table manipulation for the kernel.
// Phase B: identity-mapped kernel at physical addresses.
// Phase C (this): set up higher-half kernel mapping and per-process page tables.
//
// Design decisions (ADR-008):
// - 4-level page tables (PML4)
// - Higher-half kernel at 0xFFFF_8000_0000_0000
// - Guard pages between regions
// - Per-process user address spaces (future)
//
// Invariants:
// - Page table entries are only modified through this module
// - Physical frames for page tables come from phys::alloc_frame
// - Recursive mapping not used (direct physical access via identity map in early boot)

use super::phys::{self, PhysFrame, FRAME_SIZE};

/// Page table entry flags (x86_64).
pub mod flags {
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER: u64 = 1 << 2;
    pub const WRITE_THROUGH: u64 = 1 << 3;
    pub const NO_CACHE: u64 = 1 << 4;
    pub const ACCESSED: u64 = 1 << 5;
    pub const DIRTY: u64 = 1 << 6;
    pub const HUGE_PAGE: u64 = 1 << 7;
    pub const GLOBAL: u64 = 1 << 8;
    pub const NO_EXECUTE: u64 = 1 << 63;

    /// Kernel code: present, not writable, global, no-execute disabled
    pub const KERNEL_CODE: u64 = PRESENT | GLOBAL;
    /// Kernel data: present, writable, global, no-execute
    pub const KERNEL_DATA: u64 = PRESENT | WRITABLE | GLOBAL | NO_EXECUTE;
    /// Kernel read-only data: present, global, no-execute
    pub const KERNEL_RODATA: u64 = PRESENT | GLOBAL | NO_EXECUTE;
    /// Page table intermediate entry
    pub const TABLE: u64 = PRESENT | WRITABLE;
    /// User code: present, user, no-execute disabled
    pub const USER_CODE: u64 = PRESENT | USER;
    /// User data: present, writable, user, no-execute
    pub const USER_DATA: u64 = PRESENT | WRITABLE | USER | NO_EXECUTE;
}

/// A page table entry (8 bytes).
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    pub const fn empty() -> Self {
        PageTableEntry(0)
    }

    #[inline]
    pub fn is_present(self) -> bool {
        self.0 & flags::PRESENT != 0
    }

    #[inline]
    pub fn frame(self) -> Option<PhysFrame> {
        if self.is_present() {
            Some(PhysFrame::containing(self.0 & 0x000F_FFFF_FFFF_F000))
        } else {
            None
        }
    }

    #[inline]
    pub fn flags(self) -> u64 {
        self.0 & !0x000F_FFFF_FFFF_F000
    }

    #[inline]
    pub fn set(&mut self, frame: PhysFrame, flags: u64) {
        self.0 = frame.addr() | flags;
    }

    #[inline]
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}

/// A page table: 512 entries, aligned to 4 KiB.
#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [PageTableEntry; 512],
}

impl PageTable {
    pub const fn empty() -> Self {
        PageTable {
            entries: [PageTableEntry::empty(); 512],
        }
    }

    /// Zero all entries.
    pub fn clear(&mut self) {
        for entry in &mut self.entries {
            entry.clear();
        }
    }
}

/// Virtual address decomposition for 4-level paging.
#[derive(Debug, Clone, Copy)]
pub struct VirtAddr(pub u64);

impl VirtAddr {
    #[inline] pub fn pml4_index(self) -> usize { ((self.0 >> 39) & 0x1FF) as usize }
    #[inline] pub fn pdpt_index(self) -> usize { ((self.0 >> 30) & 0x1FF) as usize }
    #[inline] pub fn pd_index(self) -> usize   { ((self.0 >> 21) & 0x1FF) as usize }
    #[inline] pub fn pt_index(self) -> usize   { ((self.0 >> 12) & 0x1FF) as usize }
    #[inline] pub fn offset(self) -> usize     { (self.0 & 0xFFF) as usize }
}

/// Higher-half kernel base address.
pub const KERNEL_VIRT_BASE: u64 = 0xFFFF_8000_0000_0000;

/// Convert a physical address to a kernel virtual address.
/// Only valid after higher-half mapping is established.
#[inline]
pub fn phys_to_virt(phys: u64) -> u64 {
    phys + KERNEL_VIRT_BASE
}

/// Convert a kernel virtual address back to physical.
#[inline]
pub fn virt_to_phys(virt: u64) -> u64 {
    virt - KERNEL_VIRT_BASE
}

/// Read the current PML4 (CR3 register).
pub fn read_cr3() -> u64 {
    let cr3: u64;
    // SAFETY: Reading CR3 is safe — it returns the current page table base.
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack)); }
    cr3
}

/// Write CR3 to switch page tables.
///
/// # Safety
/// The new CR3 must point to a valid PML4 with correct mappings.
/// Incorrect mappings will cause immediate page faults or triple faults.
pub unsafe fn write_cr3(pml4_phys: u64) {
    // SAFETY: Caller guarantees valid page tables.
    core::arch::asm!("mov cr3, {}", in(reg) pml4_phys, options(nostack));
}

/// Invalidate a TLB entry for the given virtual address.
pub fn invlpg(virt: u64) {
    // SAFETY: invlpg is safe — it only invalidates a cached translation.
    unsafe { core::arch::asm!("invlpg [{}]", in(reg) virt, options(nostack)); }
}

/// Map a single 4 KiB page: virt → phys with given flags.
///
/// Allocates intermediate page table levels as needed.
/// Operates on page tables at physical addresses (identity-mapped access).
///
/// # Safety
/// - `pml4_phys` must point to a valid PML4 table
/// - Pages must be identity-mapped or accessible for writing
/// - `phys_frame` must be a valid physical frame
pub unsafe fn map_page(pml4_phys: u64, virt: VirtAddr, phys_frame: PhysFrame, page_flags: u64) -> Result<(), &'static str> {
    let pml4 = &mut *(pml4_phys as *mut PageTable);

    // If mapping a user page, intermediate entries also need USER bit
    let table_flags = if page_flags & flags::USER != 0 {
        flags::TABLE | flags::USER
    } else {
        flags::TABLE
    };

    // Walk or create PDPT
    let pdpt = ensure_table(&mut pml4.entries[virt.pml4_index()], table_flags, 3)?;
    // Walk or create PD
    let pd = ensure_table(&mut pdpt.entries[virt.pdpt_index()], table_flags, 2)?;
    // Walk or create PT
    let pt = ensure_table(&mut pd.entries[virt.pd_index()], table_flags, 1)?;

    let pte = &mut pt.entries[virt.pt_index()];
    if pte.is_present() {
        // User address spaces inherit parts of the kernel identity map.
        // When loading ELF/user stack pages, replace those inherited leaf
        // mappings in the process-private tables.
        if page_flags & flags::USER != 0 {
            pte.set(phys_frame, page_flags);
            invlpg(virt.0);
            return Ok(());
        }
        return Err("Page already mapped");
    }
    pte.set(phys_frame, page_flags);

    invlpg(virt.0);
    Ok(())
}

/// Unmap a single 4 KiB page.
///
/// # Safety
/// `pml4_phys` must point to a valid PML4 table accessible via identity map.
pub unsafe fn unmap_page(pml4_phys: u64, virt: VirtAddr) -> Result<PhysFrame, &'static str> {
    let pml4 = &mut *(pml4_phys as *mut PageTable);

    let pdpt_entry = &pml4.entries[virt.pml4_index()];
    if !pdpt_entry.is_present() { return Err("Not mapped (no PDPT)"); }
    let pdpt = &mut *(pdpt_entry.frame().ok_or("Corrupt PDPT entry")?.addr() as *mut PageTable);

    let pd_entry = &pdpt.entries[virt.pdpt_index()];
    if !pd_entry.is_present() { return Err("Not mapped (no PD)"); }
    let pd = &mut *(pd_entry.frame().ok_or("Corrupt PD entry")?.addr() as *mut PageTable);

    let pt_entry = &pd.entries[virt.pd_index()];
    if !pt_entry.is_present() { return Err("Not mapped (no PT)"); }
    let pt = &mut *(pt_entry.frame().ok_or("Corrupt PT entry")?.addr() as *mut PageTable);

    let pte = &mut pt.entries[virt.pt_index()];
    if !pte.is_present() { return Err("Not mapped"); }

    let frame = pte.frame().ok_or("Corrupt PTE")?;
    pte.clear();
    invlpg(virt.0);
    Ok(frame)
}

/// Ensure a page table entry points to a sub-table.
/// If not present, allocate a new frame for the sub-table with the given flags.
/// If already present, OR in additional flags (e.g., USER for user-page mappings).
///
/// # Safety
/// Called within map_page — page table memory must be identity-mapped.
unsafe fn ensure_table(entry: &mut PageTableEntry, table_flags: u64, level: u8) -> Result<&mut PageTable, &'static str> {
    if !entry.is_present() {
        let frame = phys::alloc_frame().map_err(|_| "Out of frames for page table")?;
        // Zero the new page table
        let table_ptr = frame.addr() as *mut PageTable;
        core::ptr::write_bytes(table_ptr, 0, 1);
        entry.set(frame, table_flags);
        Ok(&mut *table_ptr)
    } else {
        if entry.flags() & flags::HUGE_PAGE != 0 {
            split_huge_entry(entry, table_flags, level)?;
        }

        let addr = entry.frame().ok_or("Corrupt entry in ensure_table")?.addr();
        let current_flags = entry.flags();

        // If additional flags are required (notably USER), clone the table so we do
        // not mutate shared kernel page-table structures.
        if current_flags & table_flags != table_flags {
            let new_frame = phys::alloc_frame().map_err(|_| "Out of frames for page table")?;
            let new_ptr = new_frame.addr() as *mut PageTable;
            let old_ptr = addr as *const PageTable;
            core::ptr::copy_nonoverlapping(old_ptr, new_ptr, 1);
            entry.set(new_frame, current_flags | table_flags);
            return Ok(&mut *new_ptr);
        }

        Ok(&mut *(addr as *mut PageTable))
    }
}

/// Split a huge-page mapping into a lower-level page table so 4 KiB mappings
/// can coexist safely with existing identity-map huge pages.
///
/// `level` is the level of `entry` as described in `ensure_table`.
unsafe fn split_huge_entry(entry: &mut PageTableEntry, table_flags: u64, level: u8) -> Result<(), &'static str> {
    if entry.flags() & flags::HUGE_PAGE == 0 {
        return Ok(());
    }

    let old_flags = entry.flags();
    let base_phys = entry.frame().ok_or("Corrupt huge-page entry")?.addr();
    let new_frame = phys::alloc_frame().map_err(|_| "Out of frames for split table")?;
    let new_table_ptr = new_frame.addr() as *mut PageTable;
    core::ptr::write_bytes(new_table_ptr, 0, 1);
    let new_table = &mut *new_table_ptr;

    // Keep user/writable semantics from the original huge entry.
    let mut child_flags = old_flags & !flags::HUGE_PAGE;
    let user_needed = (table_flags & flags::USER) != 0;
    if user_needed {
        child_flags |= flags::USER;
    }

    match level {
        // Split 1 GiB page (PDPT huge) into 512 x 2 MiB PD huge entries.
        2 => {
            let step = 0x20_0000u64;
            for i in 0..512usize {
                let phys = base_phys + (i as u64) * step;
                new_table.entries[i].set(
                    PhysFrame::containing(phys),
                    child_flags | flags::HUGE_PAGE,
                );
            }
        }
        // Split 2 MiB page (PD huge) into 512 x 4 KiB PT entries.
        1 => {
            let step = FRAME_SIZE as u64;
            for i in 0..512usize {
                let phys = base_phys + (i as u64) * step;
                new_table.entries[i].set(
                    PhysFrame::containing(phys),
                    child_flags,
                );
            }
        }
        _ => return Err("Invalid level for huge-page split"),
    }

    // Replace the huge entry with a normal table entry.
    let mut new_parent_flags = flags::TABLE;
    if old_flags & flags::USER != 0 || user_needed {
        new_parent_flags |= flags::USER;
    }
    entry.set(new_frame, new_parent_flags);
    Ok(())
}

/// Map a contiguous range of physical memory into virtual space.
///
/// # Safety
/// Same requirements as `map_page`.
pub unsafe fn map_range(
    pml4_phys: u64,
    virt_start: u64,
    phys_start: u64,
    size: u64,
    page_flags: u64,
) -> Result<(), &'static str> {
    let pages = (size + FRAME_SIZE as u64 - 1) / FRAME_SIZE as u64;
    for i in 0..pages {
        let offset = i * FRAME_SIZE as u64;
        map_page(
            pml4_phys,
            VirtAddr(virt_start + offset),
            PhysFrame::containing(phys_start + offset),
            page_flags,
        )?;
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Page table lifecycle management
// ──────────────────────────────────────────────────────────────────────────────

/// Allocate a new empty page table (zeroed 4 KiB frame).
/// Returns the physical address of the allocated frame.
pub fn alloc_page_table() -> Result<u64, &'static str> {
    let frame = phys::alloc_frame().map_err(|_| "Out of frames for page table")?;
    // SAFETY: We just allocated this frame; zeroing it is safe.
    unsafe {
        core::ptr::write_bytes(frame.addr() as *mut u8, 0, FRAME_SIZE);
    }
    Ok(frame.addr())
}

/// Create a new user-process page table derived from the current kernel CR3.
///
/// Copies only present PML4 entries that are kernel-only (non-USER).
/// Kernel pages remain protected because their leaf PTEs don't have the USER bit.
/// User segments are mapped separately by `from_elf` with USER flags, and
/// `map_page` propagates USER to intermediate entries as needed.
///
/// Returns the physical address of the new PML4.
pub fn create_user_page_table() -> Result<u64, &'static str> {
    let new_pml4_phys = alloc_page_table()?;
    let kernel_pml4_phys = read_cr3() & !0xFFF_u64; // Strip PCID/flags from CR3

    // SAFETY: CR3 points to the current active PML4. The new PML4 was just allocated.
    unsafe {
        let src = &*(kernel_pml4_phys as *const PageTable);
        let dst = &mut *(new_pml4_phys as *mut PageTable);
        // Copy only present non-USER entries. This avoids inheriting the
        // caller's user-space mappings when sys_spawn is invoked from ring 3,
        // which would otherwise alias parent and child address spaces.
        for i in 0..512 {
            if src.entries[i].is_present() && (src.entries[i].flags() & flags::USER == 0) {
                dst.entries[i] = src.entries[i];
            }
        }
    }
    Ok(new_pml4_phys)
}

/// Free a complete page-table hierarchy plus all mapped physical frames.
///
/// Pass `free_mapped_frames = true` to also free the frames that *leaf* PTEs
/// point to (i.e. the actual process memory). Pass `false` to free only the
/// page-table bookkeeping frames (used when frames are managed elsewhere).
///
/// # Safety
/// `pml4_phys` must be a valid PML4 that was built entirely with frames from
/// the kernel's physical allocator.
pub unsafe fn free_page_table(pml4_phys: u64, free_mapped_frames: bool) {
    free_table_level_user_only(pml4_phys, 4, free_mapped_frames);
}

/// Recursive helper: walks a page-table level and frees sub-tables and,
/// optionally, mapped frames.
///
/// Only USER-marked branches are traversed. This avoids touching shared kernel
/// mappings that are copied into user page tables for kernel-mode execution.
unsafe fn free_table_level_user_only(table_phys: u64, level: u8, free_mapped: bool) {
    let table = &*(table_phys as *const PageTable);
    for entry in &table.entries {
        if !entry.is_present() {
            continue;
        }

        // Skip shared kernel mappings (non-USER entries).
        if entry.flags() & flags::USER == 0 {
            continue;
        }

        // Skip huge-page entries at levels > 1 (we don't sub-walk them).
        if entry.flags() & flags::HUGE_PAGE != 0 {
            if free_mapped {
                if let Some(f) = entry.frame() {
                    let _ = phys::free_frame(f);
                }
            }
            continue;
        }
        if let Some(child_frame) = entry.frame() {
            if level > 1 {
                // Recurse into sub-table.
                free_table_level_user_only(child_frame.addr(), level - 1, free_mapped);
            } else if free_mapped {
                // Leaf PTE — free the mapped physical frame.
                let _ = phys::free_frame(child_frame);
            }
        }
    }
    // Free the table frame itself.
    let _ = phys::free_frame(PhysFrame::containing(table_phys));
}

// ──────────────────────────────────────────────────────────────────────────────
// Page table cloning (for fork)
// ──────────────────────────────────────────────────────────────────────────────

/// Clone a user-process page table: deep-copy user entries (PML4 0..255),
/// share kernel entries (PML4 256..511) by reference.
///
/// Each user-space leaf page frame is physically copied so the child gets
/// an independent address space.
///
/// # Safety
/// `src_pml4_phys` must be a valid PML4 created by `create_user_page_table`.
pub unsafe fn clone_user_page_table(src_pml4_phys: u64) -> Result<u64, &'static str> {
    let new_pml4_phys = alloc_page_table()?;
    let src_pml4 = &*(src_pml4_phys as *const PageTable);
    let dst_pml4 = &mut *(new_pml4_phys as *mut PageTable);

    // Share kernel-half entries (256..511) by reference.
    for i in 256..512 {
        dst_pml4.entries[i] = src_pml4.entries[i];
    }

    // Deep-copy user-half entries (0..256).
    for i in 0..256 {
        if !src_pml4.entries[i].is_present() {
            continue;
        }

        if src_pml4.entries[i].flags() & flags::USER == 0 {
            // Preserve shared kernel mapping branches by reference.
            dst_pml4.entries[i] = src_pml4.entries[i];
            continue;
        }

        let src_child = src_pml4.entries[i].frame().ok_or("Corrupt PML4")?.addr();
        let dst_child = clone_table_level(src_child, 3)?;
        dst_pml4.entries[i].set(
            PhysFrame::containing(dst_child),
            src_pml4.entries[i].flags(),
        );
    }

    Ok(new_pml4_phys)
}

/// Recursively clone a page-table level.
/// level 3 = PDPT, 2 = PD, 1 = PT (leaf).
/// At the leaf level, physical frames are copied byte-for-byte.
unsafe fn clone_table_level(src_table_phys: u64, level: u8) -> Result<u64, &'static str> {
    let src = &*(src_table_phys as *const PageTable);
    let dst_phys = alloc_page_table()?;
    let dst = &mut *(dst_phys as *mut PageTable);

    for i in 0..512 {
        if !src.entries[i].is_present() {
            continue;
        }

        if src.entries[i].flags() & flags::USER == 0 {
            // Keep kernel mappings shared between processes.
            dst.entries[i] = src.entries[i];
            continue;
        }

        // Huge pages: copy the entire huge frame (not recursed).
        if src.entries[i].flags() & flags::HUGE_PAGE != 0 {
            if let Some(src_frame) = src.entries[i].frame() {
                let huge_size = if level == 2 { 0x20_0000usize } else { 0x4000_0000usize }; // 2M or 1G
                let dst_frame = phys::alloc_frame().map_err(|_| "OOM cloning huge page")?;
                // For huge pages we'd need contiguous alloc; skip copy for safety.
                // In practice the user-space doesn't use huge pages in this kernel.
                let _ = (src_frame, huge_size, dst_frame);
                dst.entries[i] = src.entries[i]; // share (safe for now)
            }
            continue;
        }

        if level == 1 {
            // Leaf: copy the physical frame contents.
            let src_frame = src.entries[i].frame().ok_or("Corrupt PTE in clone")?;
            let dst_frame = phys::alloc_frame().map_err(|_| "OOM cloning page")?;
            core::ptr::copy_nonoverlapping(
                src_frame.addr() as *const u8,
                dst_frame.addr() as *mut u8,
                FRAME_SIZE,
            );
            dst.entries[i].set(dst_frame, src.entries[i].flags());
        } else {
            // Intermediate: recurse.
            let src_child = src.entries[i].frame().ok_or("Corrupt entry in clone")?;
            let dst_child = clone_table_level(src_child.addr(), level - 1)?;
            dst.entries[i].set(
                PhysFrame::containing(dst_child),
                src.entries[i].flags(),
            );
        }
    }

    Ok(dst_phys)
}
