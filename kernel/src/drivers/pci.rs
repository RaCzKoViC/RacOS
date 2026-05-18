// RaCore — PCI Enumeration
//
// Simple PCI enumeration via I/O ports 0xCF8 (Address) and 0xCFC (Data).

use crate::arch::{inl, outl};

pub const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
pub const PCI_CONFIG_DATA: u16 = 0xCFC;

#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
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

    pub fn get_bar0(&self) -> u32 {
        self.config_read_u32(0x10)
    }
}

pub fn enumerate_pci() -> alloc::vec::Vec<PciDevice> {
    let mut devices = alloc::vec::Vec::new();
    for bus in 0..255 {
        for slot in 0..32 {
            let dev = check_device(bus as u8, slot as u8);
            if let Some(d) = dev {
                devices.push(d);
            }
        }
    }
    devices
}

fn check_device(bus: u8, slot: u8) -> Option<PciDevice> {
    let vendor_id = pci_read_u16(bus, slot, 0, 0);
    if vendor_id == 0xFFFF {
        return None;
    }
    
    let device_id = pci_read_u16(bus, slot, 0, 2);
    let class_rev = pci_read_u16(bus, slot, 0, 8); // Class is in high byte
    let class_code = (class_rev >> 8) as u8;
    let subclass = (class_rev & 0xFF) as u8;

    Some(PciDevice {
        bus,
        slot,
        func: 0,
        vendor_id,
        device_id,
        class_code,
        subclass,
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
