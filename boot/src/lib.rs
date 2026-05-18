// RacOS UEFI Bootloader — shared types and helpers
//
// Common types used by both the UEFI bootloader binary and the kernel.

#![no_std]

pub mod elf;

/// Magic value: "RACOS_BI" as u64
pub const BOOT_INFO_MAGIC: u64 = 0x5241434F535F4249;
pub const BOOT_INFO_VERSION: u32 = 1;

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
#[derive(Clone, Copy)]
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
