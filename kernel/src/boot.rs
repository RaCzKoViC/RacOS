// RaCore — Boot info structures
//
// Defines the BootInfo structure passed from the UEFI bootloader to the kernel.
// This is the contract between boot/kernel (see BOOT_FLOW.md).

/// Magic value: "RACOS_BI" as u64
pub const BOOT_INFO_MAGIC: u64 = 0x5241434F535F4249;

/// Boot information passed from bootloader to kernel.
#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    pub _padding: u32,
    pub memory_map: MemoryMapInfo,
    pub framebuffer: FramebufferInfo,
    pub initramfs_base: u64,
    pub initramfs_size: u64,
    pub rsdp_address: u64,
    pub boot_timestamp_ns: u64,
    pub kernel_physical_base: u64,
    pub kernel_virtual_base: u64,
}

#[repr(C)]
pub struct MemoryMapInfo {
    pub entries: *const MemoryMapEntry,
    pub entry_count: u64,
}

#[repr(C)]
pub struct MemoryMapEntry {
    pub base: u64,
    pub size: u64,
    pub mem_type: MemoryType,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryType {
    Usable = 0,
    Reserved = 1,
    AcpiReclaimable = 2,
    AcpiNvs = 3,
    BadMemory = 4,
    BootloaderReclaimable = 5,
    KernelAndModules = 6,
    Framebuffer = 7,
}

#[repr(C)]
pub struct FramebufferInfo {
    pub address: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u8,
    pub pixel_format: PixelFormat,
    pub _padding: [u8; 2],
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb = 0,
    Bgr = 1,
}

/// Validate BootInfo magic and version.
///
/// # Panics
/// Panics if magic is invalid or version is unsupported.
pub fn validate(info: &BootInfo) {
    if info.magic != BOOT_INFO_MAGIC {
        panic!("Invalid BootInfo magic: expected 0x{:016X}, got 0x{:016X}",
            BOOT_INFO_MAGIC, info.magic);
    }
    if info.version < 1 {
        panic!("Unsupported BootInfo version: {}", info.version);
    }
}

/// Count usable memory bytes from the memory map.
///
/// # Safety
/// Assumes memory_map.entries points to valid memory with entry_count entries.
pub fn count_usable_memory(info: &BootInfo) -> u64 {
    let mut total: u64 = 0;
    let count = info.memory_map.entry_count as usize;
    for i in 0..count {
        // SAFETY: entries pointer validated by bootloader, i is within bounds
        let entry = unsafe { &*info.memory_map.entries.add(i) };
        if entry.mem_type == MemoryType::Usable {
            total += entry.size;
        }
    }
    total
}
