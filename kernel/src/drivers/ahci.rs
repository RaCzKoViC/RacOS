// RaCore — AHCI/SATA disk driver (ADR-010)
//
// Single-port, single-command-slot, polling-based MVP. Sufficient to back
// racfs on a QEMU `ich9-ahci` controller. Sequential reads/writes only; no
// NCQ, no interrupts, no LPM. The HBA ABAR is identity-mapped — for the
// addresses QEMU uses (low 4 GiB) this matches the kernel's identity map
// established at boot.

extern crate alloc;

use alloc::sync::Arc;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{compiler_fence, Ordering};

use crate::drivers::block::{BlockDevice, BlockError, BlockResult, SECTOR_SIZE};
use crate::drivers::pci::{Bar, PciDevice};
use crate::mm::phys::{self, FRAME_SIZE};
use crate::sync::SpinLock;

const PCI_CLASS_MASS_STORAGE: u8 = 0x01;
const PCI_SUBCLASS_SATA: u8 = 0x06;
const PCI_PROG_IF_AHCI: u8 = 0x01;

// --- HBA memory register offsets ---
const HBA_CAP: usize = 0x00;
const HBA_GHC: usize = 0x04;
const HBA_PI: usize = 0x0C;
const HBA_PORT_BASE: usize = 0x100;
const HBA_PORT_STRIDE: usize = 0x80;

const GHC_AE: u32 = 1 << 31; // AHCI enable
const GHC_HR: u32 = 1 << 0; // HBA reset

// --- Per-port register offsets (relative to port base) ---
const P_CLB: usize = 0x00;
const P_CLBU: usize = 0x04;
const P_FB: usize = 0x08;
const P_FBU: usize = 0x0C;
const P_IS: usize = 0x10;
const P_IE: usize = 0x14;
const P_CMD: usize = 0x18;
const P_TFD: usize = 0x20;
const P_SIG: usize = 0x24;
const P_SSTS: usize = 0x28;
const P_SERR: usize = 0x30;
const P_CI: usize = 0x38;

const PORT_CMD_ST: u32 = 1 << 0;
const PORT_CMD_FRE: u32 = 1 << 4;
const PORT_CMD_FR: u32 = 1 << 14;
const PORT_CMD_CR: u32 = 1 << 15;

const SIG_ATA: u32 = 0x0000_0101;

const TFD_ERR: u32 = 1 << 0;
const TFD_DRQ: u32 = 1 << 3;
const TFD_BSY: u32 = 1 << 7;

// --- ATA commands ---
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;
const ATA_CMD_IDENTIFY: u8 = 0xEC;

// --- FIS types ---
const FIS_TYPE_REG_H2D: u8 = 0x27;

// --- Command header / table layout (per AHCI 1.3 spec) ---

#[repr(C)]
struct CmdHeader {
    /// CFL (5), A, W, P, R, B, C, rsv (1), PMP (4) packed in 16 bits.
    flags: u16,
    prdtl: u16,
    prdbc: u32,
    ctba: u32,
    ctbau: u32,
    _rsv: [u32; 4],
}

#[repr(C)]
struct PrdtEntry {
    dba: u32,
    dbau: u32,
    _rsv: u32,
    /// bits[21:0] = byte count - 1, bit 31 = interrupt-on-completion
    dbc_i: u32,
}

#[repr(C)]
struct CmdTable {
    cfis: [u8; 64],
    acmd: [u8; 16],
    _rsv: [u8; 48],
    prdt: [PrdtEntry; 1],
}

// --- HBA / port handle ---

struct Port {
    /// Pointer to the start of this port's register block.
    regs: *mut u8,
    /// Owned DMA pages, kept around for the lifetime of the driver.
    cmd_list_phys: u64,
    fis_phys: u64,
    cmd_table_phys: u64,
    /// Identify result.
    sector_count: u64,
}

// SAFETY: registers are accessed only through volatile read/write;
// internal mutability is fenced by SpinLock<Ahci>.
unsafe impl Send for Port {}

struct Ahci {
    abar: *mut u8,
    port: Option<Port>,
}

unsafe impl Send for Ahci {}

static AHCI: SpinLock<Option<Ahci>> = SpinLock::new(None);

// --- Volatile MMIO helpers ---

#[inline]
unsafe fn mmio_r32(base: *mut u8, off: usize) -> u32 {
    read_volatile(base.add(off) as *const u32)
}
#[inline]
unsafe fn mmio_w32(base: *mut u8, off: usize, val: u32) {
    write_volatile(base.add(off) as *mut u32, val);
    compiler_fence(Ordering::SeqCst);
}

// ─────────────────────────────────────────────────────────────────────────
// Public driver entry: scan PCI, init controller, register block device.
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum AhciError {
    NoController,
    NoPort,
    Identify,
    NotIdle,
    Io,
}

/// Locate an AHCI controller, initialize it, and register the first attached
/// SATA disk under the block subsystem as "sda".
pub fn init(pci_devices: &[PciDevice]) -> Result<(), AhciError> {
    let pci = pci_devices
        .iter()
        .find(|d| {
            d.class_code == PCI_CLASS_MASS_STORAGE
                && d.subclass == PCI_SUBCLASS_SATA
                && d.prog_if == PCI_PROG_IF_AHCI
        })
        .ok_or(AhciError::NoController)?;

    pci.enable_bus_master();

    let abar = match pci.read_bar(5) {
        Bar::Mem { base, .. } if base != 0 => base as *mut u8,
        _ => return Err(AhciError::NoController),
    };

    crate::serial::serial_println!(
        "[ AHCI ] controller at PCI {:02x}:{:02x}.{:01x} ABAR=0x{:08X}",
        pci.bus,
        pci.slot,
        pci.func,
        abar as u64,
    );

    // Bring HBA into AHCI mode.
    unsafe {
        let ghc = mmio_r32(abar, HBA_GHC);
        mmio_w32(abar, HBA_GHC, ghc | GHC_AE);
    }

    // Pick the first implemented + present port.
    let pi = unsafe { mmio_r32(abar, HBA_PI) };
    let mut chosen: Option<usize> = None;
    for i in 0..32 {
        if pi & (1 << i) == 0 {
            continue;
        }
        let port = port_regs(abar, i);
        let ssts = unsafe { mmio_r32(port, P_SSTS) };
        let det = ssts & 0x0F;
        let ipm = (ssts >> 8) & 0x0F;
        if det == 3 && ipm == 1 {
            let sig = unsafe { mmio_r32(port, P_SIG) };
            if sig == SIG_ATA {
                chosen = Some(i);
                break;
            }
        }
    }
    let port_idx = chosen.ok_or(AhciError::NoPort)?;
    crate::serial::serial_println!("[ AHCI ] using port {}", port_idx);

    let port = init_port(abar, port_idx)?;
    let sectors = port.sector_count;

    let mut slot = AHCI.lock();
    *slot = Some(Ahci {
        abar,
        port: Some(port),
    });
    drop(slot);

    let dev: Arc<dyn BlockDevice> = Arc::new(SataDisk { sectors });
    // SAFETY: drivers::init holds CLI-STI in MVP and only calls us once.
    unsafe {
        crate::drivers::block::register(dev);
    }
    Ok(())
}

#[inline]
fn port_regs(abar: *mut u8, idx: usize) -> *mut u8 {
    unsafe { abar.add(HBA_PORT_BASE + idx * HBA_PORT_STRIDE) }
}

fn init_port(abar: *mut u8, idx: usize) -> Result<Port, AhciError> {
    let regs = port_regs(abar, idx);

    // Stop the port (CMD.ST=0, then wait CR=0; CMD.FRE=0, then wait FR=0).
    unsafe {
        let cmd = mmio_r32(regs, P_CMD);
        mmio_w32(regs, P_CMD, cmd & !PORT_CMD_ST);
        wait_clear(regs, P_CMD, PORT_CMD_CR, 500_000)?;
        let cmd = mmio_r32(regs, P_CMD);
        mmio_w32(regs, P_CMD, cmd & !PORT_CMD_FRE);
        wait_clear(regs, P_CMD, PORT_CMD_FR, 500_000)?;
    }

    // Allocate DMA-coherent pages. We use one 4 KiB page each for the
    // command list and the FIS structures (alignment requirements: CL
    // 1024 B, FIS 256 B — frame alignment exceeds both). The command
    // table fits in another page (128 B + PRDT).
    let cmd_list_phys = phys::alloc_frame().map_err(|_| AhciError::Io)?.addr();
    let fis_phys = phys::alloc_frame().map_err(|_| AhciError::Io)?.addr();
    let cmd_table_phys = phys::alloc_frame().map_err(|_| AhciError::Io)?.addr();
    // SAFETY: identity-mapped pages, exclusively owned.
    unsafe {
        core::ptr::write_bytes(cmd_list_phys as *mut u8, 0, FRAME_SIZE);
        core::ptr::write_bytes(fis_phys as *mut u8, 0, FRAME_SIZE);
        core::ptr::write_bytes(cmd_table_phys as *mut u8, 0, FRAME_SIZE);
    }

    // Link slot 0's command header to the command table.
    unsafe {
        let header = cmd_list_phys as *mut CmdHeader;
        write_volatile(&mut (*header).ctba, cmd_table_phys as u32);
        write_volatile(&mut (*header).ctbau, (cmd_table_phys >> 32) as u32);
    }

    // Program port base addresses and re-enable.
    unsafe {
        mmio_w32(regs, P_CLB, cmd_list_phys as u32);
        mmio_w32(regs, P_CLBU, (cmd_list_phys >> 32) as u32);
        mmio_w32(regs, P_FB, fis_phys as u32);
        mmio_w32(regs, P_FBU, (fis_phys >> 32) as u32);
        mmio_w32(regs, P_SERR, 0xFFFF_FFFF); // clear all errors
        let cmd = mmio_r32(regs, P_CMD);
        mmio_w32(regs, P_CMD, cmd | PORT_CMD_FRE);
        let cmd = mmio_r32(regs, P_CMD);
        mmio_w32(regs, P_CMD, cmd | PORT_CMD_ST);
    }

    // Run IDENTIFY DEVICE to obtain sector count.
    let identify_phys = phys::alloc_frame().map_err(|_| AhciError::Io)?.addr();
    unsafe {
        core::ptr::write_bytes(identify_phys as *mut u8, 0, 512);
    }
    submit_command(
        regs,
        cmd_list_phys,
        cmd_table_phys,
        ATA_CMD_IDENTIFY,
        /*lba=*/ 0,
        /*count=*/ 0,
        identify_phys,
        512,
        /*write=*/ false,
    )?;

    let sector_count = read_identify_lba(identify_phys);
    let _ = phys::free_frame(phys::PhysFrame::containing(identify_phys));

    crate::serial::serial_println!(
        "[ AHCI ] port {} ready: {} sectors ({} MiB)",
        idx,
        sector_count,
        (sector_count * SECTOR_SIZE as u64) / (1024 * 1024),
    );

    Ok(Port {
        regs,
        cmd_list_phys,
        fis_phys,
        cmd_table_phys,
        sector_count,
    })
}

fn read_identify_lba(buf_phys: u64) -> u64 {
    // ATA IDENTIFY: words 100..103 hold 48-bit LBA count (if word 83 bit 10 is set).
    unsafe {
        let p = buf_phys as *const u16;
        let w83 = read_volatile(p.add(83));
        if w83 & (1 << 10) != 0 {
            let lo = read_volatile(p.add(100)) as u64;
            let mi = read_volatile(p.add(101)) as u64;
            let hi = read_volatile(p.add(102)) as u64;
            let hh = read_volatile(p.add(103)) as u64;
            (hh << 48) | (hi << 32) | (mi << 16) | lo
        } else {
            let lo = read_volatile(p.add(60)) as u64;
            let hi = read_volatile(p.add(61)) as u64;
            (hi << 16) | lo
        }
    }
}

/// Fill command FIS, descriptor, then ring CI and poll.
fn submit_command(
    regs: *mut u8,
    cmd_list_phys: u64,
    cmd_table_phys: u64,
    cmd: u8,
    lba: u64,
    count: u16,
    buf_phys: u64,
    byte_count: u32,
    write: bool,
) -> Result<(), AhciError> {
    // SAFETY: pages identity-mapped, slot 0 reserved for synchronous I/O.
    unsafe {
        let header = cmd_list_phys as *mut CmdHeader;
        // CFL = 5 (Reg H2D FIS dwords), W bit (28) set for writes, no atapi.
        let mut flags: u16 = 5;
        if write {
            flags |= 1 << 6;
        }
        write_volatile(&mut (*header).flags, flags);
        write_volatile(&mut (*header).prdtl, 1);
        write_volatile(&mut (*header).prdbc, 0);

        let table = cmd_table_phys as *mut CmdTable;
        // Wipe CFIS area so stale bytes don't leak.
        core::ptr::write_bytes((*table).cfis.as_mut_ptr(), 0, 64);
        let cfis = (*table).cfis.as_mut_ptr();
        *cfis.add(0) = FIS_TYPE_REG_H2D;
        *cfis.add(1) = 1 << 7; // C: command register update
        *cfis.add(2) = cmd;
        *cfis.add(3) = 0; // feature low
        *cfis.add(4) = lba as u8;
        *cfis.add(5) = (lba >> 8) as u8;
        *cfis.add(6) = (lba >> 16) as u8;
        *cfis.add(7) = 1 << 6; // LBA mode
        *cfis.add(8) = (lba >> 24) as u8;
        *cfis.add(9) = (lba >> 32) as u8;
        *cfis.add(10) = (lba >> 40) as u8;
        *cfis.add(11) = 0; // feature high
        *cfis.add(12) = count as u8;
        *cfis.add(13) = (count >> 8) as u8;
        *cfis.add(14) = 0; // ICC
        *cfis.add(15) = 0; // control

        let prdt = &mut (*table).prdt[0];
        write_volatile(&mut prdt.dba, buf_phys as u32);
        write_volatile(&mut prdt.dbau, (buf_phys >> 32) as u32);
        let dbc = byte_count.saturating_sub(1) & 0x003F_FFFF;
        write_volatile(&mut prdt.dbc_i, dbc);
    }

    // Wait for BSY/DRQ to clear before issuing.
    wait_ready(regs, 1_000_000)?;

    // Clear interrupt status, then issue slot 0.
    unsafe {
        mmio_w32(regs, P_IS, 0xFFFF_FFFF);
        mmio_w32(regs, P_CI, 1);
    }

    // Poll CI until cleared (= command complete) or TFD error.
    for _ in 0..50_000_000u64 {
        let ci = unsafe { mmio_r32(regs, P_CI) };
        if ci & 1 == 0 {
            break;
        }
        let tfd = unsafe { mmio_r32(regs, P_TFD) };
        if tfd & TFD_ERR != 0 {
            return Err(AhciError::Io);
        }
        core::hint::spin_loop();
    }
    let ci = unsafe { mmio_r32(regs, P_CI) };
    if ci & 1 != 0 {
        return Err(AhciError::Io);
    }
    let tfd = unsafe { mmio_r32(regs, P_TFD) };
    if tfd & TFD_ERR != 0 {
        return Err(AhciError::Io);
    }
    Ok(())
}

fn wait_clear(regs: *mut u8, off: usize, mask: u32, spins: u64) -> Result<(), AhciError> {
    for _ in 0..spins {
        let v = unsafe { mmio_r32(regs, off) };
        if v & mask == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(AhciError::NotIdle)
}

fn wait_ready(regs: *mut u8, spins: u64) -> Result<(), AhciError> {
    for _ in 0..spins {
        let tfd = unsafe { mmio_r32(regs, P_TFD) };
        if tfd & (TFD_BSY | TFD_DRQ) == 0 {
            return Ok(());
        }
        core::hint::spin_loop();
    }
    Err(AhciError::NotIdle)
}

// ─────────────────────────────────────────────────────────────────────────
// BlockDevice impl — public face of the SATA disk.
// ─────────────────────────────────────────────────────────────────────────

pub struct SataDisk {
    sectors: u64,
}

impl BlockDevice for SataDisk {
    fn name(&self) -> &str {
        "sda"
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
        do_io(lba, out, /*write=*/ false)
    }

    fn write_sector(&self, lba: u64, input: &[u8]) -> BlockResult<()> {
        if input.len() != SECTOR_SIZE {
            return Err(BlockError::InvalidBuffer);
        }
        if lba >= self.sectors {
            return Err(BlockError::OutOfRange);
        }
        do_io(lba, input, /*write=*/ true)
    }
}

fn do_io(lba: u64, buf: &[u8], write: bool) -> BlockResult<()> {
    // Stage in/out through a 4 KiB DMA bounce buffer so callers may use stack
    // or arbitrary kernel-heap memory.
    let scratch = phys::alloc_frame().map_err(|_| BlockError::Io)?.addr();
    let result = {
        let guard = AHCI.lock();
        let ahci = guard.as_ref().ok_or(BlockError::Io)?;
        let port = ahci.port.as_ref().ok_or(BlockError::Io)?;

        if write {
            // SAFETY: scratch is owned, sized SECTOR_SIZE bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(buf.as_ptr(), scratch as *mut u8, SECTOR_SIZE);
            }
        }
        let cmd = if write {
            ATA_CMD_WRITE_DMA_EXT
        } else {
            ATA_CMD_READ_DMA_EXT
        };
        let res = submit_command(
            port.regs,
            ahci.cmd_list_phys_for_test(port),
            port.cmd_table_phys,
            cmd,
            lba,
            1,
            scratch,
            SECTOR_SIZE as u32,
            write,
        );
        if !write && res.is_ok() {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    scratch as *const u8,
                    buf.as_ptr() as *mut u8,
                    SECTOR_SIZE,
                );
            }
        }
        res
    };
    let _ = phys::free_frame(phys::PhysFrame::containing(scratch));
    result.map_err(|_| BlockError::Io)
}

// Tiny accessor (kept here so Ahci's fields stay private to this module).
impl Ahci {
    fn cmd_list_phys_for_test(&self, port: &Port) -> u64 {
        port.cmd_list_phys
    }
}
