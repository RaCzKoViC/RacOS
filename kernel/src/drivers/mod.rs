// RaCore — Driver subsystem

extern crate alloc;

use alloc::boxed::Box;

use crate::sync::SpinLock;

pub mod ahci;
pub mod block;
pub mod cache;
pub mod pci;
pub mod ps2_keyboard;
pub mod virtio_net;
pub mod virtqueue;

/// Global handle to the first probed virtio-net device.
/// `None` until `init()` runs and finds the device.
pub static NIC: SpinLock<Option<Box<virtio_net::VirtioNet>>> = SpinLock::new(None);

pub fn init() {
    // SAFETY: boot-once drivers init sequence; IRQs still off.
    unsafe {
        block::init();
        block::init_default_ramdisk();
    }

    let pci_devices = pci::enumerate_pci();

    // Bind the first virtio-net we find.
    for dev in &pci_devices {
        if let Some(net) = virtio_net::VirtioNet::new(dev) {
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
            break;
        }
    }

    if NIC.lock().is_none() {
        crate::serial::serial_println!("[  0.001220] DRIVERS: no VirtIO-Net device found");
    }

    // Bring up AHCI (Phase F step 1). Failure here is non-fatal — the system
    // still boots on the RAM disk.
    match ahci::init(&pci_devices) {
        Ok(()) => {
            crate::serial::serial_println!("[  0.001300] DRIVERS: AHCI initialized, sda registered")
        }
        Err(e) => {
            crate::serial::serial_println!("[  0.001300] DRIVERS: AHCI init skipped: {:?}", e)
        }
    }
}

// `ahci_self_test` used to live here. It wrote "RACOS-AHCI-PhaseF" into LBA 1
// on any boot where it did not already find it there, and read it back on the
// next boot to prove the SATA disk kept data across a reboot.
//
// It was written in Phase F, when nothing else lived on sda. Something does
// now, and LBA 1 has belonged to the filesystem ever since: first to the
// allocation bitmap, and since the metadata journal to its header. So every
// boot that did not find the marker overwrote 17 bytes of live filesystem
// metadata with ASCII text.
//
// That is not a hypothetical. `racos-disk-corrupt-evidence.img`, kept in this
// project as evidence of "genuine corruption" and cited in the CHANGELOG for
// its `leaked=52` diagnosis, holds "SACOS-AHCI-PhaseF" at LBA 1 - the marker
// with one extra bit set where the filesystem later allocated block 0. The
// marker sets 55 bits; fsck reported 52 leaked blocks. The corruption fsck
// was built to detect was being manufactured by this self-test.
//
// It is deleted rather than moved because what it proved is now proved
// properly: racfs's boot-counter and big-probe survive a reboot through the
// real filesystem, which is a stronger statement about AHCI persistence than
// a raw sector ever was, and they do it without writing outside their own
// filesystem.
/// Self-test invoked from kernel_main after init: sends a single broadcast
/// frame and reports the result. No-op if the NIC is absent.
pub fn nic_self_test() {
    let mut guard = NIC.lock();
    let Some(nic) = guard.as_mut() else {
        return;
    };

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
            "[  0.001230] DRIVERS: NIC self-test OK ({} bytes sent)",
            frame.len()
        ),
        Err(e) => {
            crate::serial::serial_println!("[  0.001230] DRIVERS: NIC self-test failed: {:?}", e)
        }
    }
}
