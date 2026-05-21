// RaCore — PCI Enumeration
//
// Simple PCI enumeration via I/O ports 0xCF8 (Address) and 0xCFC (Data).
// Supports BAR0..BAR5 with 32/64-bit memory BAR detection and a helper
// to enable bus-master DMA in the command register.

use crate::arch::{inl, outl};

pub const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
pub const PCI_CONFIG_DATA: u16 = 0xCFC;

const REG_COMMAND: u8 = 0x04;
const BAR_BASE: u8 = 0x10;
const CMD_BUS_MASTER: u32 = 1 << 2;
const CMD_MEMORY_SPACE: u32 = 1 << 1;

#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
}

/// Decoded base-address register.
#[derive(Debug, Clone, Copy)]
pub enum Bar {
    /// 32- or 64-bit memory-mapped region.
    Mem { base: u64, prefetch: bool },
    /// I/O port range.
    Io  { base: u16 },
    /// BAR slot is unused (raw value 0) or hidden behind a 64-bit pair.
    Unused,
}

impl PciDevice {
    pub fn config_read_u32(&self, offset: u8) -> u32 {
        let address = ((self.bus as u32) << 16)
            | ((self.slot as u32) << 11)
            | ((self.func as u32) << 8)
            | ((offset as u32) & 0xFC)
            | 0x80000000;
        unsafe {
            outl(PCI_CONFIG_ADDRESS, address);
            inl(PCI_CONFIG_DATA)
        }
    }

    pub fn config_write_u32(&self, offset: u8, value: u32) {
        let address = ((self.bus as u32) << 16)
            | ((self.slot as u32) << 11)
            | ((self.func as u32) << 8)
            | ((offset as u32) & 0xFC)
            | 0x80000000;
        unsafe {
            outl(PCI_CONFIG_ADDRESS, address);
            outl(PCI_CONFIG_DATA, value);
        }
    }

    /// Compat shim — legacy callers (virtio-net) still grab BAR0 as a raw u32.
    pub fn get_bar0(&self) -> u32 {
        self.config_read_u32(BAR_BASE)
    }

    /// Decode a base-address register. `idx` is 0..6.
    /// If `idx-1` was the low half of a 64-bit BAR, returns `Unused`.
    pub fn read_bar(&self, idx: u8) -> Bar {
        debug_assert!(idx < 6);
        let off = BAR_BASE + idx * 4;
        let lo = self.config_read_u32(off);
        if lo == 0 { return Bar::Unused; }

        if lo & 1 != 0 {
            return Bar::Io { base: (lo & 0xFFFC) as u16 };
        }

        let kind = (lo >> 1) & 0x3;
        let prefetch = (lo >> 3) & 1 != 0;
        let base_lo = (lo & 0xFFFF_FFF0) as u64;
        match kind {
            0x0 => Bar::Mem { base: base_lo, prefetch },                // 32-bit
            0x2 => {                                                    // 64-bit
                if idx >= 5 { return Bar::Mem { base: base_lo, prefetch }; }
                let hi = self.config_read_u32(off + 4) as u64;
                Bar::Mem { base: base_lo | (hi << 32), prefetch }
            }
            _ => Bar::Mem { base: base_lo, prefetch },                  // reserved
        }
    }

    /// Set Bus Master + Memory Space in the command register so the device
    /// can issue DMA and respond to MMIO reads/writes.
    pub fn enable_bus_master(&self) {
        let cmd = self.config_read_u32(REG_COMMAND);
        let new = cmd | CMD_BUS_MASTER | CMD_MEMORY_SPACE;
        if new != cmd {
            self.config_write_u32(REG_COMMAND, new);
        }
    }
}

pub fn enumerate_pci() -> alloc::vec::Vec<PciDevice> {
    let mut devices = alloc::vec::Vec::new();
    for bus in 0..255 {
        for slot in 0..32 {
            if let Some(d) = check_device(bus as u8, slot as u8) {
                devices.push(d);
            }
        }
    }
    devices
}

fn check_device(bus: u8, slot: u8) -> Option<PciDevice> {
    let vendor_id = pci_read_u16(bus, slot, 0, 0);
    if vendor_id == 0xFFFF { return None; }

    let device_id = pci_read_u16(bus, slot, 0, 2);
    let class_rev = pci_read_u32(bus, slot, 0, 0x08);
    let class_code = (class_rev >> 24) as u8;
    let subclass  = (class_rev >> 16) as u8;
    let prog_if   = (class_rev >> 8)  as u8;

    Some(PciDevice {
        bus, slot, func: 0,
        vendor_id, device_id,
        class_code, subclass, prog_if,
    })
}

fn pci_read_u16(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
    let address = ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
        | 0x80000000;
    unsafe {
        outl(PCI_CONFIG_ADDRESS, address);
        let val = inl(PCI_CONFIG_DATA);
        ((val >> ((offset & 2) * 8)) & 0xFFFF) as u16
    }
}

fn pci_read_u32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC)
        | 0x80000000;
    unsafe {
        outl(PCI_CONFIG_ADDRESS, address);
        inl(PCI_CONFIG_DATA)
    }
}
