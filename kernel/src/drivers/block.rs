// RaCore — Block device subsystem (Phase B MVP)
//
// Provides a minimal in-kernel block device abstraction plus a simple
// RAM-backed disk used to bootstrap racfs work before real hardware drivers.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

/// Logical sector size used by MVP block devices.
pub const SECTOR_SIZE: usize = 512;

#[derive(Debug)]
pub enum BlockError {
    OutOfRange,
    InvalidBuffer,
    Io,
}

pub type BlockResult<T> = Result<T, BlockError>;

/// Minimal block device interface.
pub trait BlockDevice: Send + Sync {
    fn name(&self) -> &str;
    fn sector_count(&self) -> u64;
    fn read_sector(&self, lba: u64, out: &mut [u8]) -> BlockResult<()>;
    fn write_sector(&self, lba: u64, input: &[u8]) -> BlockResult<()>;
}

/// Simple RAM-backed block device.
pub struct RamBlockDevice {
    name: &'static str,
    storage: UnsafeCell<Vec<u8>>,
    sectors: u64,
}

// SAFETY: Access is serialized by scheduler/CLI-STI in MVP single-core mode.
unsafe impl Send for RamBlockDevice {}
unsafe impl Sync for RamBlockDevice {}

impl RamBlockDevice {
    pub fn new(name: &'static str, sectors: u64) -> Self {
        let bytes = (sectors as usize) * SECTOR_SIZE;
        RamBlockDevice {
            name,
            storage: UnsafeCell::new(alloc::vec![0u8; bytes]),
            sectors,
        }
    }
}

impl BlockDevice for RamBlockDevice {
    fn name(&self) -> &str {
        self.name
    }

    fn sector_count(&self) -> u64 {
        self.sectors
    }

    fn read_sector(&self, lba: u64, out: &mut [u8]) -> BlockResult<()> {
        if out.len() != SECTOR_SIZE {
            return Err(BlockError::InvalidBuffer);
        }
        if lba >= self.sectors {
            return Err(BlockError::OutOfRange);
        }
        let start = (lba as usize) * SECTOR_SIZE;
        let end = start + SECTOR_SIZE;
        // SAFETY: start/end are range-checked against allocated storage.
        let storage = unsafe { &*self.storage.get() };
        out.copy_from_slice(&storage[start..end]);
        Ok(())
    }

    fn write_sector(&self, lba: u64, input: &[u8]) -> BlockResult<()> {
        if input.len() != SECTOR_SIZE {
            return Err(BlockError::InvalidBuffer);
        }
        if lba >= self.sectors {
            return Err(BlockError::OutOfRange);
        }
        let start = (lba as usize) * SECTOR_SIZE;
        let end = start + SECTOR_SIZE;
        // SAFETY: start/end are range-checked against allocated storage.
        let storage = unsafe { &mut *self.storage.get() };
        storage[start..end].copy_from_slice(input);
        Ok(())
    }
}

/// Registered block devices.
static mut DEVICES: Option<Vec<Arc<dyn BlockDevice>>> = None;

/// # Safety
/// Must be called once after heap init and before concurrent access.
pub unsafe fn init() {
    let devices = &mut *core::ptr::addr_of_mut!(DEVICES);
    *devices = Some(Vec::new());
}

/// # Safety
/// Must be called with scheduler serialization (MVP: CLI/STI around call sites).
pub unsafe fn register(device: Arc<dyn BlockDevice>) {
    if let Some(list) = (*core::ptr::addr_of_mut!(DEVICES)).as_mut() {
        crate::serial::serial_println!(
            "[ DRV-BLK ] Registered {} ({} sectors)",
            device.name(),
            device.sector_count()
        );
        list.push(device);
    }
}

pub fn count() -> usize {
    unsafe {
        (*core::ptr::addr_of!(DEVICES))
            .as_ref()
            .map(|d| d.len())
            .unwrap_or(0)
    }
}

/// Get a block device by index.
pub fn get(index: usize) -> Option<Arc<dyn BlockDevice>> {
    unsafe {
        (*core::ptr::addr_of!(DEVICES))
            .as_ref()?
            .get(index)
            .cloned()
    }
}

/// Find a block device by name.
pub fn find(name: &str) -> Option<Arc<dyn BlockDevice>> {
    let devs = unsafe { (*core::ptr::addr_of!(DEVICES)).as_ref()? };
    devs.iter().find(|d| d.name() == name).cloned()
}

/// Create and register a default 8 MiB ramdisk (ram0) plus a smaller 4 MiB
/// scratch ramdisk (ram1) used by the FAT32 mount at /fat.
///
/// # Safety
/// Must be called after `init`.
pub unsafe fn init_default_ramdisk() {
    let ram0_sectors = (8 * 1024 * 1024 / SECTOR_SIZE) as u64;
    let ram0 = Arc::new(RamBlockDevice::new("ram0", ram0_sectors)) as Arc<dyn BlockDevice>;
    register(ram0);

    let ram1_sectors = (4 * 1024 * 1024 / SECTOR_SIZE) as u64;
    let ram1 = Arc::new(RamBlockDevice::new("ram1", ram1_sectors)) as Arc<dyn BlockDevice>;
    register(ram1);
}
