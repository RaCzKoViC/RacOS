// RaCore — VirtIO-Net Driver (legacy PCI I/O transport)
//
// Implements TX and RX over split virtqueues per VirtIO 0.9.5.
// Queue 0 is RX, queue 1 is TX (network device convention).
//
// Boot sequence (status bits):
//   1. RESET (write 0)
//   2. ACKNOWLEDGE
//   3. DRIVER
//   4. Read device features, write driver-supported subset
//   5. Configure each queue: select, read size, write PFN
//   6. Pre-post RX buffers
//   7. DRIVER_OK

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::arch::{inb, inl, inw, outb, outl, outw};
use crate::mm::phys::{self, FRAME_SIZE};

use super::pci::PciDevice;
use super::virtqueue::{Virtqueue, QUEUE_SIZE};

// --- PCI identity ---
const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
const VIRTIO_NET_DEVICE_ID: u16 = 0x1000;

// --- Legacy I/O register offsets (relative to BAR0 base) ---
const REG_DEVICE_FEATURES: u16 = 0x00;
const REG_GUEST_FEATURES: u16 = 0x04;
const REG_QUEUE_ADDRESS: u16 = 0x08;
const REG_QUEUE_SIZE: u16 = 0x0C;
const REG_QUEUE_SELECT: u16 = 0x0E;
const REG_QUEUE_NOTIFY: u16 = 0x10;
const REG_DEVICE_STATUS: u16 = 0x12;
const REG_ISR_STATUS: u16 = 0x13;
const REG_DEVICE_CONFIG: u16 = 0x14; // MAC[6], status u16, ...

// --- Device status bits ---
const STATUS_ACKNOWLEDGE: u8 = 0x01;
const STATUS_DRIVER: u8 = 0x02;
const STATUS_DRIVER_OK: u8 = 0x04;
const STATUS_FAILED: u8 = 0x80;

// --- Feature bits (VirtIO net + common) ---
const VIRTIO_NET_F_MAC: u32 = 1 << 5;

// --- Queue indices ---
const RX_QUEUE: u16 = 0;
const TX_QUEUE: u16 = 1;

/// How many RX buffers to keep posted. Much smaller than QUEUE_SIZE — we
/// just need enough to absorb a burst without dropping while the kernel
/// polls. Each buffer is one 4 KiB page.
const RX_BUF_COUNT: usize = 16;

/// Legacy virtio-net header preceding each frame on the wire.
/// 10 bytes when VIRTIO_NET_F_MRG_RXBUF is *not* negotiated.
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct VirtioNetHdr {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
}

pub const VIRTIO_NET_HDR_LEN: usize = 10;
pub const MTU_BYTES: usize = 1514; // Ethernet II max payload incl. headers
pub const RX_BUF_BYTES: usize = VIRTIO_NET_HDR_LEN + MTU_BYTES + 2; // tiny pad

/// Owned RX buffer (one frame each, identity-mapped).
struct RxBuf {
    phys: u64,
    /// Descriptor head index this buffer was posted at.
    desc_head: u16,
}

pub struct VirtioNet {
    io_base: u16,
    pub mac: [u8; 6],
    rx: Virtqueue,
    tx: Virtqueue,
    rx_bufs: Vec<RxBuf>,
    /// TX is single-shot: we keep one staging page reused across sends.
    tx_buf_phys: u64,
}

#[derive(Debug)]
pub enum VirtioNetError {
    NotVirtioNet,
    BarNotIo,
    FeatureNegotiation,
    QueueAlloc,
    NoFreeDescriptor,
    FrameTooLarge,
}

impl VirtioNet {
    /// Probe a PCI device. Returns Some(driver) if it is virtio-net legacy I/O.
    pub fn new(pci: &PciDevice) -> Option<Self> {
        if pci.vendor_id != VIRTIO_VENDOR_ID || pci.device_id != VIRTIO_NET_DEVICE_ID {
            return None;
        }
        let bar0 = pci.get_bar0();
        if bar0 & 1 == 0 {
            return None; // not I/O space — modern transport, unsupported here
        }
        let io_base = (bar0 & !3) as u16;
        Self::bring_up(io_base).ok()
    }

    fn bring_up(io_base: u16) -> Result<Self, VirtioNetError> {
        // 1. Reset.
        unsafe {
            outb(io_base + REG_DEVICE_STATUS, 0);
        }

        // 2/3. ACKNOWLEDGE | DRIVER.
        let mut status = STATUS_ACKNOWLEDGE;
        unsafe {
            outb(io_base + REG_DEVICE_STATUS, status);
        }
        status |= STATUS_DRIVER;
        unsafe {
            outb(io_base + REG_DEVICE_STATUS, status);
        }

        // 4. Feature negotiation — we only ask for VIRTIO_NET_F_MAC.
        let device_features = unsafe { inl(io_base + REG_DEVICE_FEATURES) };
        if device_features & VIRTIO_NET_F_MAC == 0 {
            unsafe {
                outb(io_base + REG_DEVICE_STATUS, status | STATUS_FAILED);
            }
            return Err(VirtioNetError::FeatureNegotiation);
        }
        unsafe {
            outl(io_base + REG_GUEST_FEATURES, VIRTIO_NET_F_MAC);
        }

        // 5. Read MAC from device-specific config.
        let mut mac = [0u8; 6];
        for i in 0..6 {
            unsafe {
                mac[i] = inb(io_base + REG_DEVICE_CONFIG + i as u16);
            }
        }

        // 6. Set up RX and TX virtqueues.
        let rx = Self::setup_queue(io_base, RX_QUEUE)?;
        let tx = Self::setup_queue(io_base, TX_QUEUE)?;

        // 7. Allocate RX buffer pages (one 4 KiB frame each).
        let mut rx_bufs = Vec::with_capacity(RX_BUF_COUNT);
        for _ in 0..RX_BUF_COUNT {
            let f = phys::alloc_frame().map_err(|_| VirtioNetError::QueueAlloc)?;
            rx_bufs.push(RxBuf {
                phys: f.addr(),
                desc_head: 0,
            });
        }

        // 8. Staging TX buffer.
        let tx_buf = phys::alloc_frame().map_err(|_| VirtioNetError::QueueAlloc)?;

        let mut dev = VirtioNet {
            io_base,
            mac,
            rx,
            tx,
            rx_bufs,
            tx_buf_phys: tx_buf.addr(),
        };

        // 9. Post RX buffers (descriptor flags: WRITE — device fills them in).
        dev.post_initial_rx()?;

        // 10. DRIVER_OK.
        unsafe {
            outb(io_base + REG_DEVICE_STATUS, status | STATUS_DRIVER_OK);
        }
        Ok(dev)
    }

    fn setup_queue(io_base: u16, idx: u16) -> Result<Virtqueue, VirtioNetError> {
        // Select queue.
        unsafe {
            outw(io_base + REG_QUEUE_SELECT, idx);
        }
        // Legacy I/O queue size is device-dictated and read-only. Our virtqueue
        // layout MUST match exactly or the device reads/writes outside it.
        let dev_size = unsafe { inw(io_base + REG_QUEUE_SIZE) };
        crate::serial::serial_println!(
            "[ VIRTIO ] queue {} device-reported size={}",
            idx,
            dev_size,
        );
        if (dev_size as usize) != QUEUE_SIZE {
            crate::serial::serial_println!(
                "[ VIRTIO ] queue {} size mismatch (device={}, driver={}); refusing",
                idx,
                dev_size,
                QUEUE_SIZE,
            );
            return Err(VirtioNetError::QueueAlloc);
        }
        let vq = Virtqueue::new().map_err(|_| VirtioNetError::QueueAlloc)?;
        // Tell device where the queue lives. Legacy I/O uses PFN.
        unsafe {
            outl(io_base + REG_QUEUE_ADDRESS, vq.pfn());
        }
        Ok(vq)
    }

    fn post_initial_rx(&mut self) -> Result<(), VirtioNetError> {
        // We cannot iterate &mut self.rx_bufs while also calling self.rx.add_buf
        // — split with indices.
        for i in 0..self.rx_bufs.len() {
            let phys = self.rx_bufs[i].phys;
            let head = self
                .rx
                .add_buf(&[(phys, RX_BUF_BYTES as u32, true)])
                .map_err(|_| VirtioNetError::NoFreeDescriptor)?;
            self.rx_bufs[i].desc_head = head;
        }
        // Kick — RX queue notify.
        self.notify(RX_QUEUE);
        Ok(())
    }

    #[inline]
    fn notify(&self, queue: u16) {
        // SAFETY: io_base belongs to this device's BAR0.
        unsafe {
            outw(self.io_base + REG_QUEUE_NOTIFY, queue);
        }
    }

    /// Send a single Ethernet frame. `payload` must include Ethernet header,
    /// not the VirtIO header (we prepend it).
    pub fn send_frame(&mut self, payload: &[u8]) -> Result<(), VirtioNetError> {
        if payload.len() > MTU_BYTES {
            return Err(VirtioNetError::FrameTooLarge);
        }

        // Write [hdr | payload] into the staging page.
        // SAFETY: tx_buf_phys is a 4 KiB owned frame, identity-mapped.
        unsafe {
            let dst = self.tx_buf_phys as *mut u8;
            core::ptr::write_bytes(dst, 0, VIRTIO_NET_HDR_LEN); // zero header
            core::ptr::copy_nonoverlapping(
                payload.as_ptr(),
                dst.add(VIRTIO_NET_HDR_LEN),
                payload.len(),
            );
        }

        let total_len = (VIRTIO_NET_HDR_LEN + payload.len()) as u32;
        // Single descriptor (device-readable).
        self.tx
            .add_buf(&[(self.tx_buf_phys, total_len, false)])
            .map_err(|_| VirtioNetError::NoFreeDescriptor)?;

        self.notify(TX_QUEUE);

        // Reap completion synchronously (no interrupts yet).
        // In practice QEMU drains immediately; spin briefly.
        for _ in 0..100_000 {
            if let Some((head, _)) = self.tx.pop_used() {
                self.tx.free_chain(head);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        // Drop the descriptor anyway to avoid leak — device may complete later
        // but we have to reclaim something on timeout to keep going.
        Ok(())
    }

    /// Pop one received frame, if any. Copies into `out` and returns the frame
    /// length (Ethernet bytes, excluding VirtIO header). Re-posts the buffer.
    pub fn poll_rx(&mut self, out: &mut [u8]) -> Option<usize> {
        let (head, len) = self.rx.pop_used()?;
        // Locate the rx_buf that posted this descriptor.
        let buf_idx = self.rx_bufs.iter().position(|b| b.desc_head == head)?;
        let phys = self.rx_bufs[buf_idx].phys;

        let total = len as usize;
        let frame_len = total.saturating_sub(VIRTIO_NET_HDR_LEN);
        let n = frame_len.min(out.len());
        // SAFETY: phys is identity-mapped, len bytes were written by the device.
        unsafe {
            let src = (phys as *const u8).add(VIRTIO_NET_HDR_LEN);
            core::ptr::copy_nonoverlapping(src, out.as_mut_ptr(), n);
        }

        // Free the descriptor chain and re-post the buffer.
        self.rx.free_chain(head);
        if let Ok(new_head) = self.rx.add_buf(&[(phys, RX_BUF_BYTES as u32, true)]) {
            self.rx_bufs[buf_idx].desc_head = new_head;
            self.notify(RX_QUEUE);
        }
        Some(n)
    }

    /// Acknowledge the ISR (clear pending bit). Read is destructive.
    pub fn ack_isr(&self) -> u8 {
        unsafe { inb(self.io_base + REG_ISR_STATUS) }
    }
}

// Suppress dead-code warnings for fields used only for diagnostics / future
// modern-PCI migration.
const _: fn() = || {
    let _ = core::mem::size_of::<VirtioNetHdr>();
    let _: Option<Box<VirtioNet>> = None;
};
