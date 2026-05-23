// RaCore — ELF64 loader for user-space processes
//
// Loads ELF64 executables into process address space.
// Supports ET_EXEC and ET_DYN (PIE) binaries.
//
// Segments are loaded at their specified virtual addresses in user space.
// The loader validates that all segments target user space addresses.

use crate::mm::phys::{self, PhysFrame, FRAME_SIZE};

/// ELF64 magic bytes
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// ELF class: 64-bit
const ELFCLASS64: u8 = 2;
/// Little-endian
const ELFDATA2LSB: u8 = 1;
/// Executable
const ET_EXEC: u16 = 2;
/// Position-independent executable / shared object
const ET_DYN: u16 = 3;
/// x86_64
const EM_X86_64: u16 = 62;
/// Loadable segment
const PT_LOAD: u32 = 1;
/// Dynamic linking information
const PT_DYNAMIC: u32 = 2;

// Dynamic table tags (ELF64)
const DT_NULL: i64 = 0;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_RELAENT: i64 = 9;

// x86_64 relocation type
const R_X86_64_RELATIVE: u32 = 8;

/// Maximum user-space address
const USER_SPACE_MAX: u64 = 0x0000_7FFF_FFFF_FFFF;
/// Fixed load bias for ET_DYN binaries (no ASLR in MVP).
///
/// Keep this well above early identity-mapped kernel memory to avoid clashes
/// with low canonical mappings shared in bootstrap page tables.
const ET_DYN_LOAD_BIAS: u64 = 0x0000_0001_0000_0000;

/// Default user stack size: 2 MiB
pub const USER_STACK_SIZE: usize = 2 * 1024 * 1024;
/// Default user stack top address
pub const USER_STACK_TOP: u64 = 0x0000_7FFF_FFFF_0000;

/// ELF64 file header
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Elf64Header {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

/// ELF64 program header
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

/// ELF64 dynamic entry.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Elf64Dyn {
    d_tag: i64,
    d_val: u64,
}

/// ELF64 RELA relocation entry.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Elf64Rela {
    r_offset: u64,
    r_info: u64,
    r_addend: i64,
}

/// PF flags
const PF_X: u32 = 1; // Execute
const PF_W: u32 = 2; // Write
const PF_R: u32 = 4; // Read

/// A loaded ELF image ready for execution.
pub struct LoadedElf {
    /// Entry point virtual address.
    pub entry_point: u64,
    /// Segments loaded into physical memory (to be mapped into user page tables).
    pub segments: [LoadedSegment; 8],
    pub segment_count: usize,
    /// User stack allocation.
    pub stack_phys_base: u64,
    pub stack_virt_top: u64,
    pub stack_size: usize,
}

/// A loaded segment.
#[derive(Clone, Copy)]
pub struct LoadedSegment {
    /// Virtual address where this segment should be mapped.
    pub vaddr: u64,
    /// Physical address where data is loaded.
    pub paddr: u64,
    /// Size in memory (may be larger than file size — BSS).
    pub memsz: usize,
    /// Segment flags (PF_R, PF_W, PF_X).
    pub flags: u32,
}

impl LoadedSegment {
    const fn empty() -> Self {
        LoadedSegment {
            vaddr: 0,
            paddr: 0,
            memsz: 0,
            flags: 0,
        }
    }
}

#[derive(Debug)]
pub enum ElfError {
    TooSmall,
    BadMagic,
    NotElf64,
    NotLittleEndian,
    NotExecutable,
    NotX86_64,
    NoEntry,
    TooManySegments,
    SegmentOutOfBounds,
    SegmentNotInUserSpace,
    OutOfMemory,
    RelocationFailed,
}

fn resolve_loaded_ptr(
    segments: &[LoadedSegment; 8],
    seg_count: usize,
    vaddr: u64,
    size: usize,
) -> Result<*mut u8, ElfError> {
    let size_u64 = size as u64;
    for i in 0..seg_count {
        let seg = segments[i];
        let seg_start = seg.vaddr;
        let seg_end = seg_start
            .checked_add(seg.memsz as u64)
            .ok_or(ElfError::RelocationFailed)?;
        let req_end = vaddr
            .checked_add(size_u64)
            .ok_or(ElfError::RelocationFailed)?;
        if vaddr >= seg_start && req_end <= seg_end {
            let offset = vaddr - seg_start;
            return Ok((seg.paddr + offset) as *mut u8);
        }
    }
    Err(ElfError::RelocationFailed)
}

fn runtime_vaddr(addr: u64, load_bias: u64) -> Result<u64, ElfError> {
    if addr >= load_bias {
        Ok(addr)
    } else {
        addr.checked_add(load_bias)
            .ok_or(ElfError::RelocationFailed)
    }
}

fn apply_relocations(
    segments: &[LoadedSegment; 8],
    seg_count: usize,
    dynamic_vaddr: u64,
    dynamic_size: usize,
    load_bias: u64,
) -> Result<(), ElfError> {
    let dyn_ent_size = core::mem::size_of::<Elf64Dyn>();
    if dynamic_size < dyn_ent_size {
        return Ok(());
    }

    let dyn_ptr = resolve_loaded_ptr(segments, seg_count, dynamic_vaddr, dynamic_size)?;
    let dyn_count = dynamic_size / dyn_ent_size;

    let mut rela_vaddr: Option<u64> = None;
    let mut rela_size: usize = 0;
    let mut rela_ent_size: usize = core::mem::size_of::<Elf64Rela>();

    for i in 0..dyn_count {
        let ent_ptr = unsafe { dyn_ptr.add(i * dyn_ent_size) as *const Elf64Dyn };
        let ent: Elf64Dyn = unsafe { core::ptr::read_unaligned(ent_ptr) };
        let tag = ent.d_tag;
        let val = ent.d_val;
        if tag == DT_NULL {
            break;
        }
        match tag {
            DT_RELA => {
                rela_vaddr = Some(runtime_vaddr(val, load_bias)?);
            }
            DT_RELASZ => {
                rela_size = val as usize;
            }
            DT_RELAENT => {
                rela_ent_size = val as usize;
            }
            _ => {}
        }
    }

    let rela_base = match rela_vaddr {
        Some(v) if rela_size > 0 => v,
        _ => return Ok(()),
    };

    if rela_ent_size == 0 || rela_ent_size < core::mem::size_of::<Elf64Rela>() {
        return Err(ElfError::RelocationFailed);
    }

    let rela_ptr = resolve_loaded_ptr(segments, seg_count, rela_base, rela_size)?;
    let rela_count = rela_size / rela_ent_size;
    let mut applied = 0usize;

    for i in 0..rela_count {
        let ent_ptr = unsafe { rela_ptr.add(i * rela_ent_size) as *const Elf64Rela };
        let rela: Elf64Rela = unsafe { core::ptr::read_unaligned(ent_ptr) };

        let r_offset = rela.r_offset;
        let r_info = rela.r_info;
        let r_addend = rela.r_addend;
        let r_type = (r_info & 0xFFFF_FFFF) as u32;

        if r_type == R_X86_64_RELATIVE {
            let place_vaddr = runtime_vaddr(r_offset, load_bias)?;
            let place_ptr = resolve_loaded_ptr(segments, seg_count, place_vaddr, 8)?;
            let relocated = load_bias.wrapping_add(r_addend as u64);
            unsafe {
                core::ptr::write_unaligned(place_ptr as *mut u64, relocated);
            }
            applied += 1;
        }
    }

    crate::serial::serial_println!("[   ELF   ] Applied {} RELATIVE relocations", applied);
    Ok(())
}

/// Load an ELF64 binary from a byte slice into physical memory.
///
/// Returns a `LoadedElf` containing the entry point and loaded segments.
/// The caller is responsible for mapping these into the process page tables.
pub fn load_elf(data: &[u8]) -> Result<LoadedElf, ElfError> {
    if data.len() < core::mem::size_of::<Elf64Header>() {
        return Err(ElfError::TooSmall);
    }

    // SAFETY: We verified the buffer is large enough
    let hdr: Elf64Header =
        unsafe { core::ptr::read_unaligned(data.as_ptr() as *const Elf64Header) };

    // Copy fields from packed struct to avoid unaligned reference errors
    let e_ident = hdr.e_ident;
    let e_type = hdr.e_type;
    let e_machine = hdr.e_machine;
    let e_entry = hdr.e_entry;
    let e_phoff = hdr.e_phoff;
    let e_phentsize = hdr.e_phentsize;
    let e_phnum = hdr.e_phnum;

    // Validate header
    if e_ident[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if e_ident[4] != ELFCLASS64 {
        return Err(ElfError::NotElf64);
    }
    if e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::NotLittleEndian);
    }
    if e_type != ET_EXEC && e_type != ET_DYN {
        return Err(ElfError::NotExecutable);
    }
    if e_machine != EM_X86_64 {
        return Err(ElfError::NotX86_64);
    }
    if e_entry == 0 {
        return Err(ElfError::NoEntry);
    }

    let load_bias = if e_type == ET_DYN {
        ET_DYN_LOAD_BIAS
    } else {
        0
    };
    let entry_point = e_entry
        .checked_add(load_bias)
        .ok_or(ElfError::SegmentOutOfBounds)?;

    // Validate effective entry point is in user space
    if entry_point > USER_SPACE_MAX {
        return Err(ElfError::SegmentNotInUserSpace);
    }

    let mut segments = [LoadedSegment::empty(); 8];
    let mut seg_count = 0usize;
    let mut dynamic_vaddr: Option<u64> = None;
    let mut dynamic_size: usize = 0;

    // Load PT_LOAD segments
    for i in 0..e_phnum {
        let phdr_offset = e_phoff as usize + (i as usize) * (e_phentsize as usize);
        if phdr_offset + core::mem::size_of::<Elf64Phdr>() > data.len() {
            return Err(ElfError::SegmentOutOfBounds);
        }

        // SAFETY: Offset validated above
        let phdr: Elf64Phdr = unsafe {
            core::ptr::read_unaligned(data.as_ptr().add(phdr_offset) as *const Elf64Phdr)
        };

        if phdr.p_type == PT_DYNAMIC {
            let dyn_vaddr = phdr
                .p_vaddr
                .checked_add(load_bias)
                .ok_or(ElfError::SegmentOutOfBounds)?;
            dynamic_vaddr = Some(dyn_vaddr);
            dynamic_size = phdr.p_memsz as usize;
        }

        if phdr.p_type != PT_LOAD {
            continue;
        }

        if seg_count >= 8 {
            return Err(ElfError::TooManySegments);
        }

        // Copy fields from packed struct to avoid unaligned references
        let p_vaddr = phdr.p_vaddr;
        let p_memsz = phdr.p_memsz;
        let p_offset = phdr.p_offset;
        let p_filesz = phdr.p_filesz;
        let p_flags = phdr.p_flags;

        if p_filesz > p_memsz {
            return Err(ElfError::SegmentOutOfBounds);
        }

        let eff_vaddr = p_vaddr
            .checked_add(load_bias)
            .ok_or(ElfError::SegmentOutOfBounds)?;

        // Validate segment is in user space
        let seg_end = eff_vaddr
            .checked_add(p_memsz)
            .ok_or(ElfError::SegmentOutOfBounds)?;
        if seg_end > USER_SPACE_MAX {
            return Err(ElfError::SegmentNotInUserSpace);
        }

        // Validate file data is within bounds
        let file_end = (p_offset as usize)
            .checked_add(p_filesz as usize)
            .ok_or(ElfError::SegmentOutOfBounds)?;
        if file_end > data.len() {
            return Err(ElfError::SegmentOutOfBounds);
        }

        // Preserve the in-page offset so virtual->physical mapping of bytes
        // is exact even when p_vaddr is not page-aligned.
        let page_offset = (eff_vaddr as usize) & (FRAME_SIZE - 1);
        let map_vaddr = eff_vaddr - page_offset as u64;
        let total_mem = page_offset
            .checked_add(p_memsz as usize)
            .ok_or(ElfError::SegmentOutOfBounds)?;

        // Allocate physical frames for this segment (including prefix offset).
        let pages = (total_mem + FRAME_SIZE - 1) / FRAME_SIZE;
        let frame = phys::alloc_contiguous(pages).map_err(|_| ElfError::OutOfMemory)?;
        let phys_addr = frame.addr();

        // Zero the entire allocation (for BSS)
        unsafe {
            core::ptr::write_bytes(phys_addr as *mut u8, 0, pages * FRAME_SIZE);
        }

        // Copy file data into the physical allocation
        if p_filesz > 0 {
            let src = &data[p_offset as usize..file_end];
            unsafe {
                core::ptr::copy_nonoverlapping(
                    src.as_ptr(),
                    (phys_addr + page_offset as u64) as *mut u8,
                    p_filesz as usize,
                );
            }
        }

        segments[seg_count] = LoadedSegment {
            vaddr: map_vaddr,
            paddr: phys_addr,
            memsz: total_mem,
            flags: p_flags,
        };
        seg_count += 1;

        crate::serial::serial_println!(
            "[   ELF   ] Loaded segment: vaddr=0x{:X} paddr=0x{:X} memsz=0x{:X} flags={}{}{}",
            map_vaddr,
            phys_addr,
            total_mem,
            if p_flags & PF_R != 0 { "R" } else { "-" },
            if p_flags & PF_W != 0 { "W" } else { "-" },
            if p_flags & PF_X != 0 { "X" } else { "-" },
        );
    }

    if e_type == ET_DYN {
        if let Some(dyn_vaddr) = dynamic_vaddr {
            apply_relocations(&segments, seg_count, dyn_vaddr, dynamic_size, load_bias)?;
        }
    }

    // Allocate user stack
    let stack_pages = USER_STACK_SIZE / FRAME_SIZE;
    let stack_frame = phys::alloc_contiguous(stack_pages).map_err(|_| ElfError::OutOfMemory)?;
    let stack_phys = stack_frame.addr();

    // Zero the stack
    unsafe {
        core::ptr::write_bytes(stack_phys as *mut u8, 0, USER_STACK_SIZE);
    }

    crate::serial::serial_println!(
        "[   ELF   ] Entry=0x{:X}, {} segments, stack at phys=0x{:X}",
        entry_point,
        seg_count,
        stack_phys
    );

    Ok(LoadedElf {
        entry_point,
        segments,
        segment_count: seg_count,
        stack_phys_base: stack_phys,
        stack_virt_top: USER_STACK_TOP,
        stack_size: USER_STACK_SIZE,
    })
}
