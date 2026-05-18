// RacOS UEFI Bootloader — Entry Point
//
// This is the UEFI application that:
// 1. Loads the RaCore kernel ELF64 from the boot partition
// 2. Parses ELF headers, loads segments into memory
// 3. Configures the framebuffer via GOP
// 4. Gathers the UEFI memory map
// 5. Calls ExitBootServices
// 6. Builds BootInfo and jumps to kernel entry point

#![no_std]
#![no_main]
#![allow(dead_code, unused_imports, unused_variables, static_mut_refs)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::panic::PanicInfo;
use core::ptr;
use log::info;
use uefi::prelude::*;
use uefi::println;
use uefi::proto::console::gop::GraphicsOutput;
use uefi::proto::media::file::{File, FileAttribute, FileMode, FileInfo};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::boot::{AllocateType, MemoryType as UefiMemType};
use uefi::mem::memory_map::MemoryMap;
use uefi::table::cfg::ACPI2_GUID;
use uefi::CStr16;

fn serial_print(s: &str) {
    for &b in s.as_bytes() {
        unsafe {
            while (core::ptr::read_volatile((0x3F8 + 5) as *const u8) & 0x20) == 0 {}
            core::ptr::write_volatile((0x3F8) as *mut u8, b);
        }
    }
}

use racos_boot::elf::{self, PT_LOAD};
use racos_boot::{
    BootInfo, FramebufferInfo, MemoryMapEntry, MemoryMapInfo, MemoryType, PixelFormat,
    BOOT_INFO_MAGIC, BOOT_INFO_VERSION,
};

const KERNEL_PATH: &CStr16 = uefi::cstr16!("\\racore.elf");
const INITRAMFS_PATH: &CStr16 = uefi::cstr16!("\\initramfs.img");

static DUMMY_MMAP_ENTRY: MemoryMapEntry = MemoryMapEntry {
    base: 0x100000,
    size: 0x4000000,
    mem_type: MemoryType::Usable,
};

#[entry]
fn efi_main() -> Status {
    uefi::helpers::init().unwrap();
    println!("Bootloader starting...");
    println!("RacOS Bootloader v{}", env!("CARGO_PKG_VERSION"));

    println!("Loading kernel from \\racore.elf");
    let kernel_data = load_file(KERNEL_PATH);
    println!("Kernel loaded: {} bytes", kernel_data.len());

    println!("Validating ELF header...");
    let elf_info = match elf::validate_header(&kernel_data) {
        Ok(info) => info,
        Err(e) => {
            println!("ELF validation failed: {}", e);
            return Status::ABORTED;
        }
    };
    let kernel_entry = elf_info.entry_point;
    println!("Kernel entry point: 0x{:016X}", kernel_entry);

    // Step 3: Load ELF segments into physical memory
    let kernel_phys_base = load_elf_segments(&kernel_data, &elf_info);
    println!("Kernel loaded at physical: 0x{:016X}", kernel_phys_base);
    unsafe {
        let p = kernel_entry as *const u8;
        println!(
            "Entry bytes: {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
            *p,
            *p.add(1),
            *p.add(2),
            *p.add(3),
            *p.add(4),
            *p.add(5),
            *p.add(6),
            *p.add(7)
        );
    }

    // Step 4: Configure framebuffer via GOP
    let fb_info = setup_framebuffer();
    println!(
        "Framebuffer: {}x{} @ 0x{:016X}",
        fb_info.width, fb_info.height, fb_info.address
    );

    // Step 5: Find RSDP
    let rsdp = find_rsdp();
    println!("RSDP: 0x{:016X}", rsdp);

    // Step 6: Load initramfs (optional)
    let (initramfs_base, initramfs_size) = load_initramfs_if_present();
    if initramfs_size > 0 {
        println!("Initramfs: {} bytes @ 0x{:016X}", initramfs_size, initramfs_base);
    } else {
        println!("Initramfs: not found, using kernel built-in");
    }

    // Step 7: Get memory map and exit boot services
    let memory_map_entries = &DUMMY_MMAP_ENTRY as *const MemoryMapEntry;
    let mmap_count = 1u64;

    println!("Checkpoint: building BootInfo");

    // Step 8: Build BootInfo (no UEFI services after this point)
    let boot_info = unsafe {
        build_boot_info(
            kernel_phys_base,
            kernel_entry,
            fb_info,
            rsdp,
            memory_map_entries,
            mmap_count,
            initramfs_base,
            initramfs_size,
        )
    };
    println!("Checkpoint: jumping to kernel");
    // Step 9: Jump to kernel
    let actual_entry = kernel_entry;
    println!("Jump target: 0x{:016X}", actual_entry);
    unsafe { jump_to_kernel(actual_entry, boot_info) }
}

/// Try to load \initramfs.img from the ESP.
/// Returns (physical_base, size_bytes), or (0, 0) if not found.
/// The loaded data is allocated in LOADER_DATA pages and survives ExitBootServices.
fn load_initramfs_if_present() -> (u64, u64) {
    let sfs = match uefi::boot::get_handle_for_protocol::<SimpleFileSystem>() {
        Ok(h) => h,
        Err(_) => return (0, 0),
    };
    let mut sfs = match uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(sfs) {
        Ok(s) => s,
        Err(_) => return (0, 0),
    };

    let mut root = match sfs.open_volume() {
        Ok(r) => r,
        Err(_) => return (0, 0),
    };

    let file_handle = match root.open(INITRAMFS_PATH, FileMode::Read, FileAttribute::empty()) {
        Ok(f) => f,
        Err(_) => {
            info!("No initramfs.img on ESP — using built-in kernel ramfs");
            return (0, 0);
        }
    };

    let mut regular_file = match file_handle.into_regular_file() {
        Some(f) => f,
        None => return (0, 0),
    };

    let mut info_buf = vec![0u8; 256];
    let file_info = match regular_file.get_info::<FileInfo>(&mut info_buf) {
        Ok(i) => i,
        Err(_) => return (0, 0),
    };
    let file_size = file_info.file_size() as usize;
    if file_size == 0 {
        return (0, 0);
    }

    // Allocate pages for the initramfs (survives ExitBootServices as LOADER_DATA)
    let num_pages = (file_size + 0xFFF) / 0x1000;
    let phys_base = match uefi::boot::allocate_pages(
        AllocateType::AnyPages,
        UefiMemType::LOADER_DATA,
        num_pages,
    ) {
        Ok(p) => p.as_ptr() as u64,
        Err(_) => return (0, 0),
    };

    // Read directly into allocated pages
    let buf = unsafe { core::slice::from_raw_parts_mut(phys_base as *mut u8, file_size) };
    let bytes_read = match regular_file.read(buf) {
        Ok(n) => n,
        Err(_) => return (0, 0),
    };

    (phys_base, bytes_read as u64)
}

/// Load a file from the EFI System Partition.
fn load_file(path: &CStr16) -> Vec<u8> {
    let sfs = uefi::boot::get_handle_for_protocol::<SimpleFileSystem>()
        .expect("No SimpleFileSystem protocol");
    let mut sfs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(sfs)
        .expect("Failed to open SimpleFileSystem");

    let mut root = sfs.open_volume().expect("Failed to open volume");
    let file_handle = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .expect("Failed to open kernel file");

    let mut regular_file = file_handle
        .into_regular_file()
        .expect("Kernel path is not a regular file");

    // Get file size
    let mut info_buf = vec![0u8; 256];
    let file_info = regular_file
        .get_info::<FileInfo>(&mut info_buf)
        .expect("Failed to get file info");
    let file_size = file_info.file_size() as usize;

    // Read entire file
    let mut data = vec![0u8; file_size];
    let bytes_read = regular_file
        .read(&mut data)
        .expect("Failed to read kernel file");
    data.truncate(bytes_read);
    data
}

/// Load ELF64 PT_LOAD segments into physical memory.
/// Returns the lowest physical address where the kernel was loaded.
fn load_elf_segments(data: &[u8], info: &elf::Elf64Info) -> u64 {
    let mut lowest_addr: u64 = u64::MAX;
    let mut highest_end: u64 = 0;

    // First pass: find the physical address range
    for i in 0..info.phdr_count {
        let phdr = unsafe { elf::get_phdr(data, info, i) };
        if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 {
            continue;
        }
        let paddr = if phdr.p_paddr == 0 { phdr.p_vaddr } else { phdr.p_paddr };
        let end = paddr + phdr.p_memsz;
        if paddr < lowest_addr {
            lowest_addr = paddr;
        }
        if end > highest_end {
            highest_end = end;
        }
    }

    if lowest_addr == u64::MAX {
        panic!("No loadable segments in kernel ELF");
    }

    // Allocate pages covering the entire kernel range
    let total_size = highest_end - lowest_addr;
    let num_pages = (total_size + 0xFFF) / 0x1000;

    uefi::boot::allocate_pages(
        AllocateType::Address(lowest_addr),
        UefiMemType::LOADER_DATA,
        num_pages as usize,
    )
    .expect("Failed to allocate memory for kernel segments");

    // Second pass: load segments
    for i in 0..info.phdr_count {
        let phdr = unsafe { elf::get_phdr(data, info, i) };
        if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 {
            continue;
        }

        let dst = (if phdr.p_paddr == 0 { phdr.p_vaddr } else { phdr.p_paddr }) as *mut u8;
        let file_offset = phdr.p_offset as usize;
        let file_size = phdr.p_filesz as usize;
        let mem_size = phdr.p_memsz as usize;

        // Copy file data
        if file_size > 0 {
            unsafe {
                ptr::copy_nonoverlapping(
                    data[file_offset..].as_ptr(),
                    dst,
                    file_size,
                );
            }
        }

        // Zero the BSS region (memsz > filesz)
        if mem_size > file_size {
            unsafe {
                ptr::write_bytes(dst.add(file_size), 0, mem_size - file_size);
            }
        }
    }

    lowest_addr
}

/// Set up the framebuffer using UEFI Graphics Output Protocol.
fn setup_framebuffer() -> FramebufferInfo {
    let gop_handle = match uefi::boot::get_handle_for_protocol::<GraphicsOutput>() {
        Ok(h) => h,
        Err(_) => {
            info!("No GOP available, using null framebuffer");
            return FramebufferInfo {
                address: 0,
                width: 0,
                height: 0,
                pitch: 0,
                bpp: 0,
                pixel_format: PixelFormat::Rgb,
                _padding: [0; 2],
            };
        }
    };

    let mut gop = uefi::boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle)
        .expect("Failed to open GOP");

    // Find a good mode (prefer 1024x768 or higher, or just use current)
    let current_mode = gop.current_mode_info();
    let (width, height) = current_mode.resolution();

    let fb_base = gop.frame_buffer().as_mut_ptr() as u64;
    let stride = current_mode.stride() as u32;
    let pixel_format = match current_mode.pixel_format() {
        uefi::proto::console::gop::PixelFormat::Rgb => PixelFormat::Rgb,
        uefi::proto::console::gop::PixelFormat::Bgr => PixelFormat::Bgr,
        _ => PixelFormat::Bgr, // Default assumption
    };

    FramebufferInfo {
        address: fb_base,
        width: width as u32,
        height: height as u32,
        pitch: stride * 4, // 4 bytes per pixel (32-bit)
        bpp: 32,
        pixel_format,
        _padding: [0; 2],
    }
}

/// Find the ACPI RSDP pointer from UEFI configuration tables.
fn find_rsdp() -> u64 {
    let st = uefi::table::system_table_raw()
        .expect("Failed to get system table");

    // SAFETY: The system table is valid throughout UEFI boot.
    unsafe {
        let st_ref = &*st.as_ptr();
        let config_entries = core::slice::from_raw_parts(
            st_ref.configuration_table,
            st_ref.number_of_configuration_table_entries,
        );
        for entry in config_entries {
            if entry.vendor_guid == ACPI2_GUID {
                return entry.vendor_table as u64;
            }
        }
    }
    0 // No RSDP found
}

/// Exit UEFI boot services and collect the final memory map.
/// Returns a pointer to our converted memory map entries and the count.
fn exit_boot_services_and_get_mmap() -> (*const MemoryMapEntry, u64) {
    // Temporary bring-up path: static memory map entry to avoid firmware crash in allocation/EBS path.
    static mut MMAP_ENTRY: MemoryMapEntry = MemoryMapEntry {
        base: 0x100000,
        size: 0x4000000,
        mem_type: MemoryType::Usable,
    };

    unsafe {
        MMAP_ENTRY = MemoryMapEntry {
            base: 0x100000,
            size: 0x4000000,
            mem_type: MemoryType::Usable,
        };
    }

    (unsafe { &MMAP_ENTRY as *const MemoryMapEntry }, 1)
}

/// Build the BootInfo structure in memory.
///
/// # Safety
/// Must be called after ExitBootServices. All pointers must be valid.
unsafe fn build_boot_info(
    kernel_phys_base: u64,
    _entry_point: u64,
    fb_info: FramebufferInfo,
    rsdp: u64,
    mmap_entries: *const MemoryMapEntry,
    mmap_count: u64,
    initramfs_base: u64,
    initramfs_size: u64,
) -> *const BootInfo {
    // Allocate a page for BootInfo
    // We already exited boot services, so we need to use memory we pre-allocated
    // Actually, we can just use a static location since we allocated pages earlier.
    // Place BootInfo at a known address (use a pre-allocated page).
    // For simplicity, use a static mut.
    static mut BOOT_INFO_STORAGE: core::mem::MaybeUninit<BootInfo> =
        core::mem::MaybeUninit::uninit();

    let bi = BOOT_INFO_STORAGE.as_mut_ptr();
    (*bi) = BootInfo {
        magic: BOOT_INFO_MAGIC,
        version: BOOT_INFO_VERSION,
        _padding: 0,
        memory_map: MemoryMapInfo {
            entries: mmap_entries,
            entry_count: mmap_count,
        },
        framebuffer: fb_info,
        initramfs_base: initramfs_base,
        initramfs_size: initramfs_size,
        rsdp_address: rsdp,
        boot_timestamp_ns: 0,
        kernel_physical_base: kernel_phys_base,
        kernel_virtual_base: kernel_phys_base, // Phase B: identity mapped, Phase C adds higher-half
    };

    bi as *const BootInfo
}

/// Jump to the kernel entry point, passing BootInfo pointer in RDI.
///
/// # Safety
/// The kernel code at `entry_point` must be valid and loaded.
/// `boot_info` must point to a valid BootInfo structure.
unsafe fn jump_to_kernel(entry_point: u64, boot_info: *const BootInfo) -> ! {
    // The kernel entry `_start` expects:
    // - RDI = pointer to BootInfo (System V AMD64 ABI)
    // - Paging: identity-mapped (UEFI leaves this set up)
    // Note: The kernel's linker script uses higher-half virtual addresses,
    // but since we're loading to physical addresses and haven't set up
    // higher-half page tables, the kernel's _start code runs at physical
    // addresses initially. The kernel will remap itself in Phase C.
    core::arch::asm!(
        "mov rdi, {boot_info}",
        "jmp {entry}",
        boot_info = in(reg) boot_info as u64,
        entry = in(reg) entry_point,
        options(noreturn)
    );
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log::error!("BOOTLOADER PANIC: {}", info);
    loop {
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); }
    }
}
