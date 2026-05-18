// RacOS Bootloader — Minimal ELF64 parser
//
// Parses the kernel ELF64 binary to extract loadable segments
// and the entry point address.

/// ELF64 magic bytes
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// ELF class: 64-bit
const ELFCLASS64: u8 = 2;
/// ELF data: little-endian
const ELFDATA2LSB: u8 = 1;
/// ELF type: executable
const ET_EXEC: u16 = 2;
/// ELF type: shared object (dynamic)
const ET_DYN: u16 = 3;
/// ELF machine: x86_64
const EM_X86_64: u16 = 62;
/// Program header type: loadable segment
pub const PT_LOAD: u32 = 1;

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

/// Validated ELF64 info extracted from a kernel image.
pub struct Elf64Info {
    pub entry_point: u64,
    pub phdr_offset: u64,
    pub phdr_count: u16,
    pub phdr_entry_size: u16,
}

/// Validate an ELF64 header from a byte buffer.
///
/// Returns `Err` with a description if the ELF is invalid.
pub fn validate_header(data: &[u8]) -> Result<Elf64Info, &'static str> {
    if data.len() < core::mem::size_of::<Elf64Header>() {
        return Err("File too small for ELF header");
    }

    // SAFETY: We checked the buffer length above. The header is repr(C, packed).
    let hdr = unsafe { &*(data.as_ptr() as *const Elf64Header) };

    if hdr.e_ident[0..4] != ELF_MAGIC {
        return Err("Invalid ELF magic");
    }
    if hdr.e_ident[4] != ELFCLASS64 {
        return Err("Not ELF64 (expected 64-bit)");
    }
    if hdr.e_ident[5] != ELFDATA2LSB {
        return Err("Not little-endian");
    }
    if hdr.e_type != ET_EXEC && hdr.e_type != ET_DYN {
        return Err("Not an executable ELF (expected ET_EXEC or ET_DYN)");
    }
    if hdr.e_machine != EM_X86_64 {
        return Err("Not x86_64 architecture");
    }
    if hdr.e_entry == 0 {
        return Err("Entry point is zero");
    }

    let phdr_end = hdr.e_phoff as usize
        + (hdr.e_phnum as usize) * (hdr.e_phentsize as usize);
    if phdr_end > data.len() {
        return Err("Program headers extend beyond file");
    }

    Ok(Elf64Info {
        entry_point: hdr.e_entry,
        phdr_offset: hdr.e_phoff,
        phdr_count: hdr.e_phnum,
        phdr_entry_size: hdr.e_phentsize,
    })
}

/// Get a program header from the ELF data by index.
///
/// # Safety
/// The caller must ensure `index < info.phdr_count` and that the
/// data slice is large enough to contain all program headers.
pub unsafe fn get_phdr(data: &[u8], info: &Elf64Info, index: u16) -> Elf64Phdr {
    let offset = info.phdr_offset as usize
        + (index as usize) * (info.phdr_entry_size as usize);
    let ptr = data.as_ptr().add(offset) as *const Elf64Phdr;
    core::ptr::read_unaligned(ptr)
}
