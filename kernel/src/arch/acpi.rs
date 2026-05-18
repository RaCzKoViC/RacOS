// RaCore — ACPI (Advanced Configuration and Power Interface)
//
// Finds and parses ACPI tables to discover system topology (CPUs, IOAPIC, etc.)

#![allow(static_mut_refs)]

use core::ptr;

/// Root System Description Pointer (RSDP)
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

#[repr(C, packed)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

/// Multiple APIC Description Table (MADT)
#[repr(C, packed)]
pub struct Madt {
    header: SdtHeader,
    local_apic_address: u32,
    flags: u32,
    // Variable length entries follow
}

pub struct AcpiInfo {
    pub cpu_count: usize,
    pub lapic_addr: u64,
}

static mut ACPI_INFO: Option<AcpiInfo> = None;

pub unsafe fn init() {
    // In a UEFI system, the RSDP address is passed via BootInfo or located in EFI Config Table.
    // For now, we stub this with a simple discovery logic or wait for future EFI extension.
    // SMP usually requires finding the MADT.
    
    // Default for single core if ACPI fails
    ACPI_INFO = Some(AcpiInfo {
        cpu_count: 1,
        lapic_addr: 0xFEE00000, // Standard x86 LAPIC address
    });
    
    crate::serial::serial_println!("[  0.000500] RACORE: ACPI/MADT discovery initialized (CPU support start)");
}

pub fn get_info() -> Option<&'static AcpiInfo> {
    unsafe { ACPI_INFO.as_ref() }
}