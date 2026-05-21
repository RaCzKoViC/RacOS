// RaCore - ACPI (Advanced Configuration and Power Interface)
//
// Walks RSDP -> RSDT/XSDT -> MADT to enumerate Local APICs (CPUs) and
// IOAPICs. The RSDP address comes from the UEFI bootloader via BootInfo.
//
// Validation: every table has its checksum verified before its contents are
// trusted. Unknown / malformed tables fall back to a single-CPU stub so the
// rest of bring-up can proceed.

#![allow(static_mut_refs)]

use core::ptr;
use core::slice;

extern crate alloc;
use alloc::vec::Vec;

const RSDP_SIG: &[u8; 8] = b"RSD PTR ";
const RSDT_SIG: &[u8; 4] = b"RSDT";
const XSDT_SIG: &[u8; 4] = b"XSDT";
const MADT_SIG: &[u8; 4] = b"APIC";

// ── Raw ACPI structures (all packed, little-endian) ───────────────────────

#[repr(C, packed)]
struct RsdpV1 {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

#[repr(C, packed)]
struct RsdpV2 {
    v1: RsdpV1,
    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    _reserved: [u8; 3],
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

#[repr(C, packed)]
struct MadtHeader {
    header: SdtHeader,
    local_apic_address: u32,
    flags: u32,
}

// MADT entry tags.
const MADT_LAPIC:        u8 = 0;
const MADT_IOAPIC:       u8 = 1;
const MADT_INT_OVERRIDE: u8 = 2;
const MADT_LAPIC_NMI:    u8 = 4;
const MADT_LAPIC_OVR:    u8 = 5;
const MADT_X2APIC:       u8 = 9;

#[repr(C, packed)]
struct MadtLapic {
    entry_type: u8,
    entry_len: u8,
    acpi_processor_id: u8,
    apic_id: u8,
    flags: u32,
}

#[repr(C, packed)]
struct MadtIoapic {
    entry_type: u8,
    entry_len: u8,
    id: u8,
    _reserved: u8,
    address: u32,
    gsi_base: u32,
}

#[repr(C, packed)]
struct MadtX2apic {
    entry_type: u8,
    entry_len: u8,
    _reserved: u16,
    x2apic_id: u32,
    flags: u32,
    acpi_processor_uid: u32,
}

// ── Parsed, kernel-side topology ──────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct CpuInfo {
    /// ACPI processor UID (for matching against _PR objects).
    pub acpi_uid: u32,
    /// LAPIC ID used to target IPIs.
    pub apic_id: u32,
    /// True when MADT advertises this CPU as usable (enabled OR
    /// online-capable). Disabled processors are still recorded but APs
    /// won't be started against them.
    pub enabled: bool,
    /// True for x2APIC entries (id > 255). Caller picks register encoding.
    pub is_x2apic: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct IoApicInfo {
    pub id: u8,
    pub address: u64,
    pub gsi_base: u32,
}

pub struct AcpiInfo {
    pub revision: u8,
    pub lapic_addr: u64,
    pub cpus: Vec<CpuInfo>,
    pub ioapics: Vec<IoApicInfo>,
}

impl AcpiInfo {
    fn fallback() -> Self {
        AcpiInfo {
            revision: 0,
            lapic_addr: 0xFEE0_0000,
            cpus: alloc::vec![CpuInfo {
                acpi_uid: 0,
                apic_id: 0,
                enabled: true,
                is_x2apic: false,
            }],
            ioapics: Vec::new(),
        }
    }

    pub fn cpu_count(&self) -> usize { self.cpus.len() }
    pub fn enabled_cpu_count(&self) -> usize {
        self.cpus.iter().filter(|c| c.enabled).count()
    }
}

static mut ACPI_INFO: Option<AcpiInfo> = None;

// ── Public entry ──────────────────────────────────────────────────────────

/// Initialise ACPI from the RSDP address the bootloader handed us. Falls
/// back to a single-CPU stub if anything fails to validate so kernel boot
/// continues even on machines we can't fully parse.
///
/// # Safety
/// `rsdp_addr` must be either 0 (no RSDP from bootloader) or a valid
/// physical address pointing at a RSDP-aligned structure.
pub unsafe fn init(rsdp_addr: u64) {
    let info = if rsdp_addr == 0 {
        crate::serial::serial_println!(
            "[  0.000500] RACORE: ACPI/MADT — no RSDP from bootloader, single-CPU fallback"
        );
        AcpiInfo::fallback()
    } else {
        match parse_rsdp(rsdp_addr) {
            Ok(info) => info,
            Err(reason) => {
                crate::serial::serial_println!(
                    "[  0.000500] RACORE: ACPI/MADT — parse failed: {}, single-CPU fallback",
                    reason,
                );
                AcpiInfo::fallback()
            }
        }
    };

    crate::serial::serial_println!(
        "[  0.000510] RACORE: ACPI rev {}, LAPIC @ 0x{:08X}, {} CPU(s) ({} enabled), {} IOAPIC(s)",
        info.revision,
        info.lapic_addr,
        info.cpu_count(),
        info.enabled_cpu_count(),
        info.ioapics.len(),
    );
    for cpu in &info.cpus {
        crate::serial::serial_println!(
            "[  0.000520] RACORE:   CPU acpi_uid={} apic_id={} enabled={} x2apic={}",
            cpu.acpi_uid, cpu.apic_id, cpu.enabled, cpu.is_x2apic,
        );
    }
    for io in &info.ioapics {
        crate::serial::serial_println!(
            "[  0.000530] RACORE:   IOAPIC id={} addr=0x{:08X} gsi_base={}",
            io.id, io.address, io.gsi_base,
        );
    }

    ACPI_INFO = Some(info);
}

pub fn get_info() -> Option<&'static AcpiInfo> {
    unsafe { ACPI_INFO.as_ref() }
}

// ── Parser internals ──────────────────────────────────────────────────────

unsafe fn parse_rsdp(addr: u64) -> Result<AcpiInfo, &'static str> {
    let rsdp = &*(addr as *const RsdpV1);
    if rsdp.signature != *RSDP_SIG {
        return Err("bad RSDP signature");
    }
    if !checksum_ok(addr as *const u8, core::mem::size_of::<RsdpV1>()) {
        return Err("bad RSDP checksum");
    }

    let revision = rsdp.revision;
    let mut cpus = Vec::new();
    let mut ioapics = Vec::new();
    let mut lapic_addr: u64 = 0xFEE0_0000;

    if revision >= 2 {
        let rsdp2 = &*(addr as *const RsdpV2);
        if !checksum_ok(addr as *const u8, rsdp2.length as usize) {
            return Err("bad XSDP checksum");
        }
        let xsdt_addr = rsdp2.xsdt_address;
        parse_sdt_array(
            xsdt_addr, /*is_xsdt=*/true,
            &mut cpus, &mut ioapics, &mut lapic_addr,
        )?;
    } else {
        let rsdt_addr = rsdp.rsdt_address as u64;
        parse_sdt_array(
            rsdt_addr, /*is_xsdt=*/false,
            &mut cpus, &mut ioapics, &mut lapic_addr,
        )?;
    }

    if cpus.is_empty() {
        // MADT absent or empty — synthesize the BSP so the rest of bring-up
        // has something to talk about.
        cpus.push(CpuInfo { acpi_uid: 0, apic_id: 0, enabled: true, is_x2apic: false });
    }

    Ok(AcpiInfo { revision, lapic_addr, cpus, ioapics })
}

unsafe fn parse_sdt_array(
    sdt_addr: u64, is_xsdt: bool,
    cpus: &mut Vec<CpuInfo>, ioapics: &mut Vec<IoApicInfo>, lapic_addr: &mut u64,
) -> Result<(), &'static str> {
    if sdt_addr == 0 {
        return Err("null RSDT/XSDT address");
    }
    let header = &*(sdt_addr as *const SdtHeader);
    let expect_sig = if is_xsdt { XSDT_SIG } else { RSDT_SIG };
    if &header.signature != expect_sig {
        return Err("bad RSDT/XSDT signature");
    }
    if !checksum_ok(sdt_addr as *const u8, header.length as usize) {
        return Err("bad RSDT/XSDT checksum");
    }

    let entries_bytes = header.length as usize - core::mem::size_of::<SdtHeader>();
    let entries_ptr = (sdt_addr as usize + core::mem::size_of::<SdtHeader>()) as *const u8;
    let stride = if is_xsdt { 8 } else { 4 };
    let count = entries_bytes / stride;

    for i in 0..count {
        let entry_addr: u64 = if is_xsdt {
            // ACPI tables can be unaligned — read via byte copy.
            let mut b = [0u8; 8];
            ptr::copy_nonoverlapping(entries_ptr.add(i * 8), b.as_mut_ptr(), 8);
            u64::from_le_bytes(b)
        } else {
            let mut b = [0u8; 4];
            ptr::copy_nonoverlapping(entries_ptr.add(i * 4), b.as_mut_ptr(), 4);
            u32::from_le_bytes(b) as u64
        };
        if entry_addr == 0 { continue; }

        let entry_hdr = &*(entry_addr as *const SdtHeader);
        if &entry_hdr.signature == MADT_SIG {
            if !checksum_ok(entry_addr as *const u8, entry_hdr.length as usize) {
                return Err("bad MADT checksum");
            }
            parse_madt(entry_addr, cpus, ioapics, lapic_addr);
        }
    }
    Ok(())
}

unsafe fn parse_madt(
    addr: u64,
    cpus: &mut Vec<CpuInfo>, ioapics: &mut Vec<IoApicInfo>, lapic_addr: &mut u64,
) {
    let madt = &*(addr as *const MadtHeader);
    let total_len = madt.header.length as usize;
    *lapic_addr = madt.local_apic_address as u64;

    let entries_start = addr as usize + core::mem::size_of::<MadtHeader>();
    let entries_end   = addr as usize + total_len;
    let mut p = entries_start;

    while p + 2 <= entries_end {
        let entry_type = *(p as *const u8);
        let entry_len  = *((p + 1) as *const u8) as usize;
        if entry_len < 2 || p + entry_len > entries_end {
            break;
        }
        match entry_type {
            MADT_LAPIC => {
                let e = &*(p as *const MadtLapic);
                // flags bit 0 = enabled, bit 1 = online-capable.
                let enabled = (e.flags & 0x3) != 0;
                cpus.push(CpuInfo {
                    acpi_uid: e.acpi_processor_id as u32,
                    apic_id:  e.apic_id as u32,
                    enabled,
                    is_x2apic: false,
                });
            }
            MADT_IOAPIC => {
                let e = &*(p as *const MadtIoapic);
                ioapics.push(IoApicInfo {
                    id: e.id,
                    address: e.address as u64,
                    gsi_base: e.gsi_base,
                });
            }
            MADT_X2APIC => {
                let e = &*(p as *const MadtX2apic);
                let enabled = (e.flags & 0x3) != 0;
                cpus.push(CpuInfo {
                    acpi_uid: e.acpi_processor_uid,
                    apic_id:  e.x2apic_id,
                    enabled,
                    is_x2apic: true,
                });
            }
            // Recognised-but-skipped types: int overrides, NMI sources, LAPIC overrides.
            // They matter for full IOAPIC routing but not for AP enumeration.
            MADT_INT_OVERRIDE | MADT_LAPIC_NMI | MADT_LAPIC_OVR => {}
            _ => {}
        }
        p += entry_len;
    }
}

/// Sum every byte in [base, base+len) modulo 256; ACPI requires zero.
unsafe fn checksum_ok(base: *const u8, len: usize) -> bool {
    let s = slice::from_raw_parts(base, len);
    let mut acc: u8 = 0;
    for &b in s {
        acc = acc.wrapping_add(b);
    }
    acc == 0
}
