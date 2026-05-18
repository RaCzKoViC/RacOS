// RaCore — Device filesystem (devfs)
//
// ADR-010: /dev is a special filesystem where device nodes appear
// automatically when drivers register devices.
//
// MVP devices:
// - /dev/null   (major 2): discards all writes, reads return EOF
// - /dev/zero   (major 3): reads return zeros, writes succeed silently
// - /dev/console (major 4): alias for serial output
// - /dev/serial0 (major 1): serial port

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use crate::sync::SpinLock;

use super::inode::{
    DirEntry, FileMode, FileType, InodeMetadata, InodeNum, InodeOps, VfsError, VfsResult,
};
use super::mount::Filesystem;

static PTY_MASTER: SpinLock<Option<crate::tty::pty::PtyMaster>> = SpinLock::new(None);

fn with_pty_master<R>(f: impl FnOnce(&mut crate::tty::pty::PtyMaster) -> R) -> R {
    let mut guard = PTY_MASTER.lock();
    if guard.is_none() {
        let (master, _slave) = crate::tty::pty::alloc_pty();
        crate::serial::serial_println!("[  DEVFS  ] Initialized PTY pair: /dev/ptmx <-> /dev/pts0");
        *guard = Some(master);
    }
    f(guard.as_mut().unwrap())
}

/// Device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Char,
    Block,
}

/// A registered device.
struct DeviceNode {
    name: String,
    ino: InodeNum,
    dev_type: DeviceType,
    major: u32,
    minor: u32,
    ops: Arc<dyn DeviceOps>,
}

/// Operations for a device.
pub trait DeviceOps: Send + Sync {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize>;
    fn write(&self, offset: u64, buf: &[u8]) -> VfsResult<usize>;
    fn ioctl(&self, _request: u64, _arg: u64) -> VfsResult<i64> {
        Err(VfsError::NotImplemented)
    }
}

/// /dev/null device — discards writes, EOF on read.
pub struct NullDevice;

impl DeviceOps for NullDevice {
    fn read(&self, _offset: u64, _buf: &mut [u8]) -> VfsResult<usize> {
        Ok(0) // EOF
    }
    fn write(&self, _offset: u64, buf: &[u8]) -> VfsResult<usize> {
        Ok(buf.len()) // Accept everything
    }
}

/// /dev/zero device — reads return zeros, writes succeed.
pub struct ZeroDevice;

impl DeviceOps for ZeroDevice {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        for byte in buf.iter_mut() {
            *byte = 0;
        }
        Ok(buf.len())
    }
    fn write(&self, _offset: u64, buf: &[u8]) -> VfsResult<usize> {
        Ok(buf.len())
    }
}

/// /dev/console and /dev/serial0 — outputs to serial port.
pub struct SerialDevice;

impl DeviceOps for SerialDevice {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        // Read from serial input buffer (filled by IRQ4)
        // Block until at least one byte is available
        loop {
            let n = crate::serial::read_input(buf);
            if n > 0 {
                return Ok(n);
            }
            // Yield to scheduler while waiting for input
            crate::task::scheduler::yield_now();
        }
    }
    fn write(&self, _offset: u64, buf: &[u8]) -> VfsResult<usize> {
        for &byte in buf {
            crate::serial::serial_print!("{}", byte as char);
        }

        // Mirror console output to the active virtual terminal so the
        // framebuffer view in QEMU shows the same session as /dev/console.
        if let Ok(s) = core::str::from_utf8(buf) {
            crate::tty::vt::vt_print(s);
        } else {
            for &b in buf {
                let ch = if b.is_ascii() { b } else { b'?' };
                let one = [ch];
                // SAFETY: `ch` is forced to ASCII, which is always valid UTF-8.
                let s = unsafe { core::str::from_utf8_unchecked(&one) };
                crate::tty::vt::vt_print(s);
            }
        }

        Ok(buf.len())
    }
}

/// /dev/urandom device — pseudo-random bytes.
pub struct UrandomDevice;

impl DeviceOps for UrandomDevice {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let mut seed = crate::interrupts::pit::uptime_ms();
        seed ^= crate::task::scheduler::current_pid() as u64;
        seed ^= buf.len() as u64;
        for byte in buf.iter_mut() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *byte = (seed >> 33) as u8;
        }
        Ok(buf.len())
    }
    fn write(&self, _offset: u64, buf: &[u8]) -> VfsResult<usize> {
        Ok(buf.len()) // Writes are accepted but ignored
    }
}

/// /dev/ptmx device — PTY master endpoint.
pub struct PtmxDevice;

impl DeviceOps for PtmxDevice {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        loop {
            let n = with_pty_master(|master| master.read_output(buf));
            if n > 0 {
                return Ok(n);
            }
            crate::task::scheduler::yield_now();
        }
    }

    fn write(&self, _offset: u64, buf: &[u8]) -> VfsResult<usize> {
        with_pty_master(|master| {
            let echo = master.write_input(buf);
            if !echo.is_empty() {
                let _ = master.slave_write(&echo);
            }
        });
        Ok(buf.len())
    }
}

/// /dev/pts0 device — PTY slave endpoint.
pub struct PtySlaveDevice;

impl DeviceOps for PtySlaveDevice {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        loop {
            let n = with_pty_master(|master| master.slave_read(buf));
            if n > 0 {
                return Ok(n);
            }
            crate::task::scheduler::yield_now();
        }
    }

    fn write(&self, _offset: u64, buf: &[u8]) -> VfsResult<usize> {
        let n = with_pty_master(|master| master.slave_write(buf));
        Ok(n)
    }
}

/// The devfs filesystem.
pub struct Devfs {
    devices: Vec<DeviceNode>,
}

/// Devfs inode wrapper.
struct DevfsInode {
    idx: usize,
    fs: Arc<Devfs>,
}

impl Devfs {
    pub fn new() -> Self {
        Devfs {
            devices: Vec::new(),
        }
    }

    /// Register a device. Returns the inode number.
    pub fn register(&mut self, name: &str, dev_type: DeviceType, major: u32, minor: u32, ops: Arc<dyn DeviceOps>) -> InodeNum {
        let ino = (self.devices.len() + 1) as InodeNum; // inode 0 = root dir
        self.devices.push(DeviceNode {
            name: String::from(name),
            ino,
            dev_type,
            major,
            minor,
            ops,
        });

        crate::serial::serial_println!(
            "[  DEVFS  ] Registered /dev/{} ({:?}, {}:{})",
            name, dev_type, major, minor
        );

        ino
    }

    /// Register the default MVP devices.
    pub fn register_defaults(&mut self) {
        self.register("serial0", DeviceType::Char, 1, 0, Arc::new(SerialDevice));
        self.register("null", DeviceType::Char, 2, 0, Arc::new(NullDevice));
        self.register("zero", DeviceType::Char, 3, 0, Arc::new(ZeroDevice));
        self.register("console", DeviceType::Char, 4, 0, Arc::new(SerialDevice));
        self.register("urandom", DeviceType::Char, 1, 9, Arc::new(UrandomDevice));
        self.register("random", DeviceType::Char, 1, 8, Arc::new(UrandomDevice));
        self.register("tty", DeviceType::Char, 5, 0, Arc::new(SerialDevice));
        self.register("ptmx", DeviceType::Char, 5, 2, Arc::new(PtmxDevice));
        self.register("pts0", DeviceType::Char, 136, 0, Arc::new(PtySlaveDevice));
        self.register("stdin", DeviceType::Char, 0, 0, Arc::new(SerialDevice));
        self.register("stdout", DeviceType::Char, 0, 1, Arc::new(SerialDevice));
        self.register("stderr", DeviceType::Char, 0, 2, Arc::new(SerialDevice));
        // Register block devices from the block driver subsystem.
        if let Some(ram0) = crate::drivers::block::find("ram0") {
            self.register("ram0", DeviceType::Block, 8, 0, Arc::new(RamBlockDevOps::new(ram0)));
        }
    }
}

/// /dev/ram0 — block device interface exposing the ramdisk to userspace.
pub struct RamBlockDevOps {
    dev: Arc<dyn crate::drivers::block::BlockDevice>,
}

impl RamBlockDevOps {
    pub fn new(dev: Arc<dyn crate::drivers::block::BlockDevice>) -> Self {
        RamBlockDevOps { dev }
    }
}

impl DeviceOps for RamBlockDevOps {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        use crate::drivers::block::SECTOR_SIZE;
        let total_size = self.dev.sector_count() * SECTOR_SIZE as u64;
        if offset >= total_size {
            return Ok(0); // EOF
        }
        let avail = (total_size - offset) as usize;
        let to_read = buf.len().min(avail);

        let mut done = 0usize;
        let mut pos = offset;
        let mut sector_buf = [0u8; SECTOR_SIZE];
        while done < to_read {
            let lba = pos / SECTOR_SIZE as u64;
            let off_in_sector = (pos % SECTOR_SIZE as u64) as usize;
            self.dev.read_sector(lba, &mut sector_buf).map_err(|_| VfsError::IoError)?;
            let chunk = (SECTOR_SIZE - off_in_sector).min(to_read - done);
            buf[done..done + chunk].copy_from_slice(&sector_buf[off_in_sector..off_in_sector + chunk]);
            done += chunk;
            pos += chunk as u64;
        }
        Ok(done)
    }

    fn write(&self, offset: u64, buf: &[u8]) -> VfsResult<usize> {
        use crate::drivers::block::SECTOR_SIZE;
        let total_size = self.dev.sector_count() * SECTOR_SIZE as u64;
        if offset >= total_size {
            return Err(VfsError::NoSpace);
        }
        let avail = (total_size - offset) as usize;
        let to_write = buf.len().min(avail);

        let mut done = 0usize;
        let mut pos = offset;
        let mut sector_buf = [0u8; SECTOR_SIZE];
        while done < to_write {
            let lba = pos / SECTOR_SIZE as u64;
            let off_in_sector = (pos % SECTOR_SIZE as u64) as usize;
            // Read-modify-write for partial sector writes.
            if off_in_sector != 0 || (to_write - done) < SECTOR_SIZE {
                self.dev.read_sector(lba, &mut sector_buf).map_err(|_| VfsError::IoError)?;
            }
            let chunk = (SECTOR_SIZE - off_in_sector).min(to_write - done);
            sector_buf[off_in_sector..off_in_sector + chunk].copy_from_slice(&buf[done..done + chunk]);
            self.dev.write_sector(lba, &sector_buf).map_err(|_| VfsError::IoError)?;
            done += chunk;
            pos += chunk as u64;
        }
        Ok(done)
    }
}

impl InodeOps for DevfsInode {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let dev = &self.fs.devices[self.idx];
        dev.ops.read(offset, buf)
    }

    fn write(&self, offset: u64, buf: &[u8]) -> VfsResult<usize> {
        let dev = &self.fs.devices[self.idx];
        dev.ops.write(offset, buf)
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let dev = &self.fs.devices[self.idx];
        let file_type = match dev.dev_type {
            DeviceType::Char => FileType::CharDevice,
            DeviceType::Block => FileType::BlockDevice,
        };
        let mut meta = InodeMetadata::new(dev.ino, file_type);
        meta.mode = FileMode::new(0o666);
        meta.dev_major = dev.major;
        meta.dev_minor = dev.minor;
        Ok(meta)
    }

    fn ioctl(&self, request: u64, arg: u64) -> VfsResult<i64> {
        let dev = &self.fs.devices[self.idx];
        dev.ops.ioctl(request, arg)
    }
}

/// Devfs root directory inode.
struct DevfsRootInode {
    fs: Arc<Devfs>,
}

impl InodeOps for DevfsRootInode {
    fn read(&self, _offset: u64, _buf: &mut [u8]) -> VfsResult<usize> {
        Err(VfsError::IsADirectory)
    }

    fn write(&self, _offset: u64, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::IsADirectory)
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let mut meta = InodeMetadata::new(0, FileType::Directory);
        meta.mode = FileMode::new(0o755);
        Ok(meta)
    }

    fn lookup(&self, name: &str) -> VfsResult<InodeNum> {
        for dev in &self.fs.devices {
            if dev.name == name {
                return Ok(dev.ino);
            }
        }
        Err(VfsError::NotFound)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        Ok(self
            .fs
            .devices
            .iter()
            .map(|dev| DirEntry {
                name: dev.name.clone(),
                ino: dev.ino,
                file_type: match dev.dev_type {
                    DeviceType::Char => FileType::CharDevice,
                    DeviceType::Block => FileType::BlockDevice,
                },
            })
            .collect())
    }
}

/// Wrapper for Filesystem trait.
pub struct DevfsFilesystem {
    inner: Arc<Devfs>,
}

impl DevfsFilesystem {
    pub fn new(devfs: Devfs) -> Arc<Self> {
        Arc::new(DevfsFilesystem {
            inner: Arc::new(devfs),
        })
    }
}

impl Filesystem for DevfsFilesystem {
    fn root_inode(&self) -> Arc<dyn InodeOps> {
        Arc::new(DevfsRootInode {
            fs: self.inner.clone(),
        })
    }

    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>> {
        if ino == 0 {
            return Ok(self.root_inode());
        }
        let idx = (ino - 1) as usize;
        if idx >= self.inner.devices.len() {
            return Err(VfsError::NotFound);
        }
        Ok(Arc::new(DevfsInode {
            idx,
            fs: self.inner.clone(),
        }))
    }

    fn name(&self) -> &str {
        "devfs"
    }
}
