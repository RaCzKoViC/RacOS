# RacOS — Boot Flow Specification

> Version: 0.1.0 | Status: Draft | Component: RaCore + Bootloader

## 1. Boot Sequence Overview

```
┌──────────┐    ┌────────────┐    ┌────────────┐    ┌──────────┐    ┌─────────┐
│ UEFI FW  │───→│ Bootloader │───→│  RaCore    │───→│ RacInit  │───→│ Targets │
│          │    │ (EFI app)  │    │  (kernel)  │    │ (PID 1)  │    │         │
└──────────┘    └────────────┘    └────────────┘    └──────────┘    └─────────┘
```

## 2. Stage 0: UEFI Firmware

**Input**: Power-on / reset
**Output**: Control transferred to bootloader EFI application

1. Platform initialization (CPU, memory controller, firmware devices)
2. UEFI Boot Services available
3. Load bootloader from EFI System Partition (ESP): `\EFI\RACOS\bootx64.efi`
4. Transfer control to bootloader entry point

## 3. Stage 1: Bootloader

**Input**: UEFI Boot Services handle, SystemTable pointer
**Output**: Kernel loaded in memory, initramfs loaded, framebuffer configured

### 3.1 Responsibilities

1. Obtain memory map via `GetMemoryMap()`
2. Locate kernel ELF64 image on boot partition: `/racore.elf`
3. Locate initramfs image: `/initramfs.img`
4. Parse kernel ELF64 headers, load segments to correct physical addresses
5. Configure framebuffer via UEFI GOP (Graphics Output Protocol)
6. Gather boot info structure:
   - Memory map (type, base, size for each region)
   - Framebuffer info (address, width, height, pitch, pixel format)
   - initramfs location (base address, size)
   - RSDP pointer (for ACPI)
   - Boot timestamp
7. Call `ExitBootServices()` — no more UEFI runtime after this
8. Set up minimal identity-mapped page tables (if kernel expects paging on)
9. Jump to kernel entry point, passing boot info pointer in RDI

### 3.2 Boot Info Structure

```rust
#[repr(C)]
pub struct BootInfo {
    pub magic: u64,                    // 0x5241434F535F4249 ("RACOS_BI")
    pub version: u32,                  // Boot info version
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
    pub mem_type: MemoryType,          // Usable, Reserved, AcpiReclaimable, etc.
}

#[repr(C)]
pub struct FramebufferInfo {
    pub address: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u8,
    pub pixel_format: PixelFormat,     // Rgb, Bgr
}
```

### 3.3 Bootloader Implementation Language

Rust (UEFI target: `x86_64-unknown-uefi`), using `uefi` crate for UEFI protocol access.

## 4. Stage 2: Kernel Early Init

**Input**: BootInfo pointer in RDI
**Output**: Kernel subsystems initialized, scheduler running, idle loop or init handoff

### 4.1 Early Init Sequence (assembly entry → Rust)

```
1. _start (assembly):
   - Save boot info pointer
   - Set up initial stack (from .bss reserved space)
   - Call kernel_main(boot_info)

2. kernel_main (Rust):
   a. Validate BootInfo magic
   b. Initialize serial output (COM1, 115200 baud)
   c. Print boot banner + build number
   d. Initialize GDT (flat segments + TSS)
   e. Initialize IDT (exception handlers + IRQ stubs)
   f. Initialize physical memory manager from memory map
   g. Initialize kernel heap allocator
   h. Initialize virtual memory manager, remap kernel to higher-half
   i. Initialize PIT/HPET timer
   j. Initialize keyboard driver (PS/2 basic)
   k. Initialize task subsystem
   l. Initialize scheduler
   m. Create init task (PID 1) → load /sbin/racinit from initramfs
   n. Enable interrupts
   o. Enter idle loop (hlt in loop, scheduler preempts)
```

### 4.2 Boot Logging

All boot messages go to serial (COM1) with structured format:
```
[  0.000000] RACORE: Boot info validated (magic OK, version 1)
[  0.000012] RACORE: Memory detected: 256 MiB usable
[  0.000045] RACORE: GDT loaded (3 entries + TSS)
[  0.000067] RACORE: IDT loaded (256 entries)
[  0.000120] RACORE: Physical allocator ready (65536 frames)
[  0.000180] RACORE: Kernel heap initialized (1 MiB initial)
[  0.000250] RACORE: Higher-half remap complete
[  0.000300] RACORE: Timer initialized (PIT, 1000 Hz)
[  0.000350] RACORE: Scheduler ready (round-robin)
[  0.000400] RACORE: Loading /sbin/racinit from initramfs...
[  0.000500] RACORE: Init process created (PID 1)
[  0.000510] RACORE: Interrupts enabled, entering idle loop
```

### 4.3 Panic Handler

If any early init step fails:
1. Print panic message to serial
2. Print register dump if available
3. Halt all CPUs (`cli; hlt` loop)
4. No reboot — allow operator to inspect serial log

## 5. Stage 3: RacInit Handoff

**Input**: Control from kernel as PID 1 user process
**Output**: System services running, base.target reached

1. RacInit reads unit files from `/etc/racinit/`
2. Builds dependency graph
3. Starts services in dependency order toward `base.target`
4. Opens `/dev/console` for system logging
5. System is "booted" when base.target is reached

## 6. Boot Image Layout

### 6.1 ISO Image (bootable)

```
/EFI/RACOS/bootx64.efi    — UEFI bootloader
/racore.elf                — Kernel image
/initramfs.img             — Initial ramdisk
```

### 6.2 initramfs Contents

```
/sbin/racinit              — Init process binary
/etc/racinit/              — Unit files for early boot
/lib/libc-lite.so          — Minimal C library (if dynamic)
/dev/                      — Pre-created device nodes (or empty)
/tmp/
/proc/
/sys/
```

## 7. Exit Criteria

- [ ] QEMU boots kernel via UEFI bootloader
- [ ] Serial output shows boot banner + build number
- [ ] Memory map is parsed and usable memory reported
- [ ] Kernel reaches idle loop without crash/panic
- [ ] initramfs is loaded (base address + size reported)

## 8. Risks

| Risk | Mitigation |
|------|------------|
| UEFI protocol complexity | Use `uefi` crate, test in OVMF |
| Memory map edge cases | Validate all entries, reject overlapping regions |
| ELF loading errors | Strict ELF64 validation, reject malformed headers |
| Framebuffer not available | Fall back to serial-only mode |
| initramfs corruption | Validate checksum before mount |
