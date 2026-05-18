// RaCore — VirtIO-Net Driver (Legacy I/O)
//
// Minimal implementation of a VirtIO network device using legacy I/O registers.

use crate::arch::{inb, outb, inw, outw};
use super::pci::PciDevice;

const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
const VIRTIO_NET_DEVICE_ID: u16 = 0x1000;

pub struct VirtioNet {
    io_base: u16,
    pub mac: [u8; 6],
}

impl VirtioNet {
    pub fn new(pci: &PciDevice) -> Option<Self> {
        if pci.vendor_id != VIRTIO_VENDOR_ID || pci.device_id != VIRTIO_NET_DEVICE_ID {
            return None;
        }

        let bar0 = pci.get_bar0();
        if bar0 & 1 == 0 {
            // Not I/O space
            return None;
        }
        let io_base = (bar0 & !3) as u16;

        let mut dev = VirtioNet {
            io_base,
            mac: [0; 6],
        };

        dev.init();
        Some(dev)
    }

    fn init(&mut self) {
        // 1. Reset device
        unsafe { outb(self.io_base + 18, 0); }

        // 2. Set ACKNOWLEDGE status bit
        let mut status = 1;
        unsafe { outb(self.io_base + 18, status); }

        // 3. Set DRIVER status bit
        status |= 2;
        unsafe { outb(self.io_base + 18, status); }

        // 4. Read MAC address from device-specific config (offset 20 in legacy I/O)
        for i in 0..6 {
            unsafe {
                self.mac[i] = inb(self.io_base + 20 + i as u16);
            }
        }

        // 5. Finalize: Set DRIVER_OK status bit
        status |= 4;
        unsafe { outb(self.io_base + 18, status); }
    }
}
