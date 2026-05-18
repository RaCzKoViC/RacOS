// RaCore — Driver subsystem

pub mod block;
pub mod cache;
pub mod ps2_keyboard;
pub mod ahci;
pub mod pci;
pub mod virtio_net;

pub fn init() {
    unsafe {
        block::init();
        block::init_default_ramdisk();
    }

    // PCI Enumeration and Network initialization
    let pci_devices = pci::enumerate_pci();
    for dev in pci_devices {
        if let Some(net) = virtio_net::VirtioNet::new(&dev) {
            crate::serial::serial_println!(
                "[  0.001200] DRIVERS: VirtIO-Net found at PCI {:02x}:{:02x}.{:01x}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                dev.bus, dev.slot, dev.func,
                net.mac[0], net.mac[1], net.mac[2], net.mac[3], net.mac[4], net.mac[5]
            );
        }
    }
}
