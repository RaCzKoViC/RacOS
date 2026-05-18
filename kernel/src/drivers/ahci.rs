// RaCore — AHCI/SATA Disk Driver
//
// ADR-010: Device Model — Mass Storage via AHCI.
// Implements the BlockDevice trait for hard disks connected via SATA.

use alloc::sync::Arc;
use crate::drivers::block::{BlockDevice, SECTOR_SIZE, BlockResult};
use crate::sync::SpinLock;

/// AHCI Controller identification
const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
const PCI_SUBCLASS_SATA: u8 = 0x06;
const PCI_PROG_IF_AHCI: u8 = 0x01;

/// Port types for AHCI
#[derive(Debug, PartialEq)]
pub(crate) enum AHCIInstanceType {
    SATA,
    SATAPI,
    PM,
    SEMB,
    None,
}

/// AHCI Port Registers (Subset)
#[repr(C)]
pub(crate) struct HbaPort {
    pub(crate) clb: u32,       // 0x00, command list base address, 1K-byte aligned
    pub(crate) clbu: u32,      // 0x04, command list base address upper 32 bits
    pub(crate) fb: u32,        // 0x08, FIS base address, 256-byte aligned
    pub(crate) fbu: u32,       // 0x0C, FIS base address upper 32 bits
    pub(crate) is: u32,        // 0x10, interrupt status
    pub(crate) ie: u32,        // 0x14, interrupt enable
    pub(crate) cmd: u32,       // 0x18, command and status
    pub(crate) rsv0: u32,      // 0x1C, reserved
    pub(crate) tfd: u32,       // 0x20, task file data
    pub(crate) sig: u32,       // 0x24, signature
    pub(crate) ssts: u32,      // 0x28, SATA status (SCR0:SStatus)
    pub(crate) sctl: u32,      // 0x2C, SATA control (SCR2:SControl)
    pub(crate) serr: u32,      // 0x30, SATA error (SCR1:SError)
    pub(crate) sact: u32,      // 0x34, SATA active (SCR3:SActive)
    pub(crate) ci: u32,        // 0x38, command issue
    pub(crate) sntf: u32,      // 0x3C, SATA notification (SCR4:SNotification)
    pub(crate) fbs: u32,       // 0x40, FIS-based switching control
    pub(crate) rsv1: [u32; 11], // 0x44 ~ 0x6F, reserved
    pub(crate) vendor: [u32; 4], // 0x70 ~ 0x7F, vendor specific
}

/// Implementation of SATA Disk via AHCI
pub struct SataDisk {
    port: *mut HbaPort,
    sector_count: u64,
}

// SAFETY: HbaPort pointer access is synchronized via SpinLock when used in the driver
unsafe impl Send for SataDisk {}
unsafe impl Sync for SataDisk {}

impl SataDisk {
    /// Create a new SATA disk instance from a detected AHCI port
    pub unsafe fn new(port: *mut HbaPort) -> Self {
        Self {
            port,
            sector_count: 0, // Should be probed via IDENTIFY DEVICE
        }
    }
}

impl BlockDevice for SataDisk {
    fn name(&self) -> &str {
        "sata"
    }

    fn read_sector(&self, lba: u64, buf: &mut [u8]) -> BlockResult<()> {
        // [MVP] Stub for AHCI I/O. Real implementation requires:
        // 1. Finding a free Command Slot in the Command List
        // 2. Setting up the Command Header and Command Table (PRDT)
        // 3. Setting the Command Issue bit for the port
        // 4. Waiting for complete (CI clear + IS check)
        
        crate::serial::serial_println!("[ AHCI ] Stub read at LBA {}", lba);
        Ok(())
    }

    fn write_sector(&self, lba: u64, buf: &[u8]) -> BlockResult<()> {
        crate::serial::serial_println!("[ AHCI ] Stub write at LBA {}", lba);
        Ok(())
    }

    fn sector_count(&self) -> u64 {
        self.sector_count
    }
}
