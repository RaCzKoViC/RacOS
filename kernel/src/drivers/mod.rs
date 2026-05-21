// RaCore — Driver subsystem

extern crate alloc;

use alloc::boxed::Box;

use crate::sync::SpinLock;

pub mod block;
pub mod cache;
pub mod ps2_keyboard;
pub mod ahci;
pub mod pci;
pub mod virtqueue;
pub mod virtio_net;

/// Global handle to the first probed virtio-net device.
/// `None` until `init()` runs and finds the device.
pub static NIC: SpinLock<Option<Box<virtio_net::VirtioNet>>> = SpinLock::new(None);

pub fn init() {
    unsafe {
        block::init();
        block::init_default_ramdisk();
    }

    // PCI enumeration; bind the first virtio-net we find.
    let pci_devices = pci::enumerate_pci();
    for dev in pci_devices {
        if let Some(net) = virtio_net::VirtioNet::new(&dev) {
            crate::serial::serial_println!(
                "[  0.001200] DRIVERS: VirtIO-Net @ PCI {:02x}:{:02x}.{:01x}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                dev.bus, dev.slot, dev.func,
                net.mac[0], net.mac[1], net.mac[2], net.mac[3], net.mac[4], net.mac[5]
            );
            crate::serial::serial_println!(
                "[  0.001210] DRIVERS: VirtIO-Net queues up (queue_size={}, RX buffers pre-posted)",
                virtqueue::QUEUE_SIZE
            );
            let mut slot = NIC.lock();
            *slot = Some(Box::new(net));
            // Only bind the first one for MVP.
            break;
        }
    }

    if NIC.lock().is_none() {
        crate::serial::serial_println!("[  0.001220] DRIVERS: no VirtIO-Net device found");
    }
}

/// Self-test invoked from kernel_main after init: sends a single broadcast
/// frame and reports the result. No-op if the NIC is absent.
pub fn nic_self_test() {
    let mut guard = NIC.lock();
    let Some(nic) = guard.as_mut() else { return; };

    // Minimal Ethernet II header: dst broadcast, src MAC, ethertype 0x88B5
    // (IEEE local experimental — won't confuse a real network stack on the host).
    let mut frame = [0u8; 60]; // pad to Ethernet minimum
    frame[0..6].copy_from_slice(&[0xFF; 6]);
    frame[6..12].copy_from_slice(&nic.mac);
    frame[12] = 0x88;
    frame[13] = 0xB5;
    // 46 bytes of marker payload "RACOS-NET-SELFTEST\0..." follow.
    let marker = b"RACOS-NET-SELFTEST";
    frame[14..14 + marker.len()].copy_from_slice(marker);

    match nic.send_frame(&frame) {
        Ok(()) => crate::serial::serial_println!(
            "[  0.001230] DRIVERS: NIC self-test OK ({} bytes sent)", frame.len()
        ),
        Err(e) => crate::serial::serial_println!(
            "[  0.001230] DRIVERS: NIC self-test failed: {:?}", e
        ),
    }
}
