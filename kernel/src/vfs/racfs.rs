// RaCore — racfs: block-device-backed writable filesystem (Phase B)
//
// On-disk layout (all sizes in 512-byte sectors):
//   Sector 0:       Superblock (magic, version, counts, offsets)
//   Sector 1..B:    Free-block bitmap (1 bit per data block)
//   Sector B+1..I:  Inode table (fixed-size 128-byte inodes)
//   Sector I+1..N:  Data blocks (512 bytes each)
//
// MVP constraints:
// - Max 128 inodes, max ~8 MiB data
// - Backed by a BlockDevice through BlockCache
// - CLI/STI serialization (single-core)

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use super::inode::{
    DirEntry, FileMode, FileType, InodeMetadata, InodeNum, InodeOps, VfsError, VfsResult,
};
use super::mount::Filesystem;

use crate::drivers::block::{BlockDevice, SECTOR_SIZE};
use crate::drivers::cache::BlockCache;

// ─── On-disk constants ──────────────────────────────────────────────────────

const RACFS_MAGIC: u32 = 0x5241_4346; // "RACF"
const RACFS_VERSION: u32 = 1;

/// Fixed number of inodes in MVP.
const MAX_INODES: usize = 128;

/// On-disk inode size in bytes (must divide SECTOR_SIZE evenly).
const INODE_DISK_SIZE: usize = 128;
const INODES_PER_SECTOR: usize = SECTOR_SIZE / INODE_DISK_SIZE; // 4

/// Maximum file name length stored in a directory entry.
const MAX_NAME_LEN: usize = 60;

/// Maximum direct block pointers per inode.
const DIRECT_BLOCKS: usize = 8;

/// Inode type tags stored on disk.
const ITYPE_FREE: u8 = 0;
const ITYPE_FILE: u8 = 1;
const ITYPE_DIR: u8 = 2;

// ─── On-disk structures (serialized manually to [u8]) ───────────────────────

/// On-disk superblock occupies sector 0.
#[derive(Clone, Copy)]
#[repr(C)]
struct Superblock {
    magic: u32,
    version: u32,
    total_sectors: u32,
    inode_count: u32,
    data_block_count: u32,
    bitmap_start: u32,   // sector offset
    inode_start: u32,     // sector offset
    data_start: u32,      // sector offset
    free_inodes: u32,
    free_blocks: u32,
    _pad: [u8; 512 - 40],
}

/// On-disk inode (128 bytes).
#[derive(Clone, Copy)]
#[repr(C)]
struct DiskInode {
    itype: u8,           // ITYPE_*
    mode: u16,           // permission bits
    _pad1: u8,
    size: u32,           // file size in bytes
    nlink: u16,
    _pad2: u16,
    uid: u32,
    gid: u32,
    /// Direct block indices (relative to data_start).
    direct: [u32; DIRECT_BLOCKS],  // 32 bytes
    /// Number of directory entries (for dirs).
    dir_entry_count: u32,
    _reserved: [u8; 128 - 56],
}

/// On-disk directory entry (64 bytes, stored in data blocks).
#[derive(Clone, Copy)]
#[repr(C)]
struct DiskDirEntry {
    ino: u32,
    name_len: u8,
    _pad: [u8; 3],
    name: [u8; MAX_NAME_LEN - 4], // 56 bytes
}

const DIR_ENTRY_SIZE: usize = 64;
const DIR_ENTRIES_PER_BLOCK: usize = SECTOR_SIZE / DIR_ENTRY_SIZE; // 8

// ─── Serialization helpers ──────────────────────────────────────────────────

fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}
fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}
fn write_u16_le(buf: &mut [u8], off: usize, v: u16) {
    let b = v.to_le_bytes();
    buf[off] = b[0]; buf[off + 1] = b[1];
}
fn write_u32_le(buf: &mut [u8], off: usize, v: u32) {
    let b = v.to_le_bytes();
    buf[off] = b[0]; buf[off + 1] = b[1]; buf[off + 2] = b[2]; buf[off + 3] = b[3];
}

fn superblock_from_sector(buf: &[u8; SECTOR_SIZE]) -> Superblock {
    Superblock {
        magic: read_u32_le(buf, 0),
        version: read_u32_le(buf, 4),
        total_sectors: read_u32_le(buf, 8),
        inode_count: read_u32_le(buf, 12),
        data_block_count: read_u32_le(buf, 16),
        bitmap_start: read_u32_le(buf, 20),
        inode_start: read_u32_le(buf, 24),
        data_start: read_u32_le(buf, 28),
        free_inodes: read_u32_le(buf, 32),
        free_blocks: read_u32_le(buf, 36),
        _pad: [0u8; 512 - 40],
    }
}

fn superblock_to_sector(sb: &Superblock) -> [u8; SECTOR_SIZE] {
    let mut buf = [0u8; SECTOR_SIZE];
    write_u32_le(&mut buf, 0, sb.magic);
    write_u32_le(&mut buf, 4, sb.version);
    write_u32_le(&mut buf, 8, sb.total_sectors);
    write_u32_le(&mut buf, 12, sb.inode_count);
    write_u32_le(&mut buf, 16, sb.data_block_count);
    write_u32_le(&mut buf, 20, sb.bitmap_start);
    write_u32_le(&mut buf, 24, sb.inode_start);
    write_u32_le(&mut buf, 28, sb.data_start);
    write_u32_le(&mut buf, 32, sb.free_inodes);
    write_u32_le(&mut buf, 36, sb.free_blocks);
    buf
}

fn inode_from_bytes(buf: &[u8]) -> DiskInode {
    let mut direct = [0u32; DIRECT_BLOCKS];
    for i in 0..DIRECT_BLOCKS {
        direct[i] = read_u32_le(buf, 16 + i * 4);
    }
    DiskInode {
        itype: buf[0],
        mode: read_u16_le(buf, 1),
        _pad1: 0,
        size: read_u32_le(buf, 4),
        nlink: read_u16_le(buf, 8),
        _pad2: 0,
        uid: read_u32_le(buf, 10),
        gid: read_u32_le(buf, 14),
        direct,
        dir_entry_count: read_u32_le(buf, 48),
        _reserved: [0u8; 128 - 56],
    }
}

fn inode_to_bytes(inode: &DiskInode) -> [u8; INODE_DISK_SIZE] {
    let mut buf = [0u8; INODE_DISK_SIZE];
    buf[0] = inode.itype;
    write_u16_le(&mut buf, 1, inode.mode);
    write_u32_le(&mut buf, 4, inode.size);
    write_u16_le(&mut buf, 8, inode.nlink);
    write_u32_le(&mut buf, 10, inode.uid);
    write_u32_le(&mut buf, 14, inode.gid);
    for i in 0..DIRECT_BLOCKS {
        write_u32_le(&mut buf, 16 + i * 4, inode.direct[i]);
    }
    write_u32_le(&mut buf, 48, inode.dir_entry_count);
    buf
}

fn direntry_from_bytes(buf: &[u8]) -> DiskDirEntry {
    let mut name = [0u8; MAX_NAME_LEN - 4];
    let name_len = buf[4] as usize;
    let copy_len = name_len.min(name.len());
    name[..copy_len].copy_from_slice(&buf[8..8 + copy_len]);
    DiskDirEntry {
        ino: read_u32_le(buf, 0),
        name_len: buf[4],
        _pad: [0; 3],
        name,
    }
}

fn direntry_to_bytes(de: &DiskDirEntry) -> [u8; DIR_ENTRY_SIZE] {
    let mut buf = [0u8; DIR_ENTRY_SIZE];
    write_u32_le(&mut buf, 0, de.ino);
    buf[4] = de.name_len;
    let copy_len = (de.name_len as usize).min(de.name.len());
    buf[8..8 + copy_len].copy_from_slice(&de.name[..copy_len]);
    buf
}

// ─── RacFS runtime ─────────────────────────────────────────────────────────

/// The racfs filesystem state backed by a block device + cache.
pub struct Racfs {
    cache: UnsafeCell<BlockCache>,
    sb: UnsafeCell<Superblock>,
}

unsafe impl Send for Racfs {}
unsafe impl Sync for Racfs {}

impl Racfs {
    /// Mount an existing racfs from `device` without touching its contents.
    /// Returns Err(IoError) if the superblock magic / version does not match,
    /// which the caller may use to fall back to `format_and_new`.
    pub fn open(device: Arc<dyn BlockDevice>) -> VfsResult<Arc<Self>> {
        let mut cache = BlockCache::new(device);
        let mut buf = [0u8; SECTOR_SIZE];
        cache.read_sector(0, &mut buf).map_err(|_| VfsError::IoError)?;
        let sb = superblock_from_sector(&buf);
        if sb.magic != RACFS_MAGIC || sb.version != RACFS_VERSION {
            return Err(VfsError::IoError);
        }
        Ok(Arc::new(Racfs {
            cache: UnsafeCell::new(cache),
            sb: UnsafeCell::new(sb),
        }))
    }

    /// Probe `device` for a valid racfs superblock; if absent, format it.
    /// Returns the resulting filesystem either way.
    pub fn open_or_format(device: Arc<dyn BlockDevice>) -> VfsResult<Arc<Self>> {
        match Self::open(device.clone()) {
            Ok(fs) => Ok(fs),
            Err(_) => Self::format_and_new(device),
        }
    }

    /// Format and mount a block device as racfs.
    pub fn format_and_new(device: Arc<dyn BlockDevice>) -> VfsResult<Arc<Self>> {
        let total_sectors = device.sector_count() as u32;

        // Layout calculation.
        let bitmap_start: u32 = 1;
        // 1 bit per data block; we need ceiling division.
        let inode_sectors = (MAX_INODES + INODES_PER_SECTOR - 1) / INODES_PER_SECTOR;
        let inode_start = bitmap_start + 1; // 1 sector of bitmap = 4096 data blocks max
        let data_start = inode_start + inode_sectors as u32;
        let data_block_count = total_sectors.saturating_sub(data_start);

        let sb = Superblock {
            magic: RACFS_MAGIC,
            version: RACFS_VERSION,
            total_sectors,
            inode_count: MAX_INODES as u32,
            data_block_count,
            bitmap_start,
            inode_start,
            data_start,
            free_inodes: (MAX_INODES - 1) as u32, // inode 0 reserved for root
            free_blocks: data_block_count,
            _pad: [0u8; 512 - 40],
        };

        let mut cache = BlockCache::new(device);

        // Write superblock.
        let sb_buf = superblock_to_sector(&sb);
        cache.write_sector(0, &sb_buf).map_err(|_| VfsError::IoError)?;

        // Zero bitmap.
        let zero = [0u8; SECTOR_SIZE];
        cache.write_sector(bitmap_start as u64, &zero).map_err(|_| VfsError::IoError)?;

        // Zero inode table.
        for s in 0..inode_sectors {
            cache.write_sector((inode_start as usize + s) as u64, &zero)
                .map_err(|_| VfsError::IoError)?;
        }

        // Write root directory inode (inode 0).
        let root_inode = DiskInode {
            itype: ITYPE_DIR,
            mode: 0o755,
            _pad1: 0,
            size: 0,
            nlink: 2,
            _pad2: 0,
            uid: 0,
            gid: 0,
            direct: [0u32; DIRECT_BLOCKS],
            dir_entry_count: 0,
            _reserved: [0u8; 128 - 56],
        };
        let fs = Arc::new(Racfs {
            cache: UnsafeCell::new(cache),
            sb: UnsafeCell::new(sb),
        });
        fs.write_inode(0, &root_inode)?;
        fs.cache_mut().flush().map_err(|_| VfsError::IoError)?;

        crate::serial::serial_println!(
            "[  RACFS  ] Formatted: {} sectors, {} data blocks, {} inodes",
            total_sectors, data_block_count, MAX_INODES
        );

        Ok(fs)
    }

    fn cache_mut(&self) -> &mut BlockCache {
        unsafe { &mut *self.cache.get() }
    }

    fn sb(&self) -> &Superblock {
        unsafe { &*self.sb.get() }
    }

    fn sb_mut(&self) -> &mut Superblock {
        unsafe { &mut *self.sb.get() }
    }

    /// Public stats snapshot (total_blocks, free_blocks, total_inodes, free_inodes).
    /// Block size is SECTOR_SIZE (512 B). Used by /proc/diskstats.
    pub fn stats(&self) -> (u32, u32, u32, u32) {
        let sb = self.sb();
        (sb.data_block_count, sb.free_blocks, sb.inode_count, sb.free_inodes)
    }

    /// Flush the superblock to disk.
    fn flush_sb(&self) -> VfsResult<()> {
        let buf = superblock_to_sector(self.sb());
        self.cache_mut().write_sector(0, &buf).map_err(|_| VfsError::IoError)
    }

    /// Read an on-disk inode by number.
    fn read_inode(&self, ino: u32) -> VfsResult<DiskInode> {
        let sb = self.sb();
        if ino >= sb.inode_count {
            return Err(VfsError::NotFound);
        }
        let sector = sb.inode_start as u64 + (ino as u64 / INODES_PER_SECTOR as u64);
        let offset_in_sector = (ino as usize % INODES_PER_SECTOR) * INODE_DISK_SIZE;

        let mut buf = [0u8; SECTOR_SIZE];
        self.cache_mut().read_sector(sector, &mut buf).map_err(|_| VfsError::IoError)?;
        Ok(inode_from_bytes(&buf[offset_in_sector..offset_in_sector + INODE_DISK_SIZE]))
    }

    /// Write an on-disk inode by number.
    fn write_inode(&self, ino: u32, inode: &DiskInode) -> VfsResult<()> {
        let sb = self.sb();
        let sector = sb.inode_start as u64 + (ino as u64 / INODES_PER_SECTOR as u64);
        let offset_in_sector = (ino as usize % INODES_PER_SECTOR) * INODE_DISK_SIZE;

        let mut buf = [0u8; SECTOR_SIZE];
        self.cache_mut().read_sector(sector, &mut buf).map_err(|_| VfsError::IoError)?;
        let inode_bytes = inode_to_bytes(inode);
        buf[offset_in_sector..offset_in_sector + INODE_DISK_SIZE].copy_from_slice(&inode_bytes);
        self.cache_mut().write_sector(sector, &buf).map_err(|_| VfsError::IoError)
    }

    /// Allocate a free inode. Returns inode number.
    fn alloc_inode(&self) -> VfsResult<u32> {
        let sb = self.sb();
        if sb.free_inodes == 0 {
            return Err(VfsError::NoSpace);
        }
        // Linear scan for a free inode slot (skip 0 = root).
        for ino in 1..sb.inode_count {
            let di = self.read_inode(ino)?;
            if di.itype == ITYPE_FREE {
                self.sb_mut().free_inodes -= 1;
                return Ok(ino);
            }
        }
        Err(VfsError::NoSpace)
    }

    /// Free an inode.
    fn free_inode(&self, ino: u32) -> VfsResult<()> {
        let mut di = self.read_inode(ino)?;
        di.itype = ITYPE_FREE;
        di.size = 0;
        di.dir_entry_count = 0;
        for i in 0..DIRECT_BLOCKS {
            if di.direct[i] != 0 {
                self.free_block(di.direct[i])?;
            }
            di.direct[i] = 0;
        }
        self.write_inode(ino, &di)?;
        self.sb_mut().free_inodes += 1;
        Ok(())
    }

    /// Allocate a data block from the bitmap. Returns data-block index (0-based).
    fn alloc_block(&self) -> VfsResult<u32> {
        let sb = self.sb();
        if sb.free_blocks == 0 {
            return Err(VfsError::NoSpace);
        }
        let mut bitmap = [0u8; SECTOR_SIZE];
        self.cache_mut().read_sector(sb.bitmap_start as u64, &mut bitmap)
            .map_err(|_| VfsError::IoError)?;

        let total = sb.data_block_count as usize;
        for byte_idx in 0..SECTOR_SIZE {
            if byte_idx * 8 >= total {
                break;
            }
            if bitmap[byte_idx] == 0xFF {
                continue;
            }
            for bit in 0..8u8 {
                let block_idx = byte_idx * 8 + bit as usize;
                if block_idx >= total {
                    return Err(VfsError::NoSpace);
                }
                if bitmap[byte_idx] & (1 << bit) == 0 {
                    bitmap[byte_idx] |= 1 << bit;
                    self.cache_mut().write_sector(sb.bitmap_start as u64, &bitmap)
                        .map_err(|_| VfsError::IoError)?;
                    self.sb_mut().free_blocks -= 1;
                    // Data block indices are 1-based (0 = "no block").
                    return Ok(block_idx as u32 + 1);
                }
            }
        }
        Err(VfsError::NoSpace)
    }

    /// Free a data block (1-based index).
    fn free_block(&self, block: u32) -> VfsResult<()> {
        if block == 0 {
            return Ok(()); // no-block sentinel
        }
        let idx = (block - 1) as usize;
        let sb = self.sb();
        let mut bitmap = [0u8; SECTOR_SIZE];
        self.cache_mut().read_sector(sb.bitmap_start as u64, &mut bitmap)
            .map_err(|_| VfsError::IoError)?;
        let byte_idx = idx / 8;
        let bit = idx % 8;
        bitmap[byte_idx] &= !(1u8 << bit);
        self.cache_mut().write_sector(sb.bitmap_start as u64, &bitmap)
            .map_err(|_| VfsError::IoError)?;
        self.sb_mut().free_blocks += 1;
        Ok(())
    }

    /// Absolute sector for a data block (1-based index).
    fn data_sector(&self, block: u32) -> u64 {
        self.sb().data_start as u64 + (block - 1) as u64
    }

    /// Read a data block.
    fn read_data_block(&self, block: u32, out: &mut [u8; SECTOR_SIZE]) -> VfsResult<()> {
        if block == 0 {
            *out = [0u8; SECTOR_SIZE];
            return Ok(());
        }
        self.cache_mut().read_sector(self.data_sector(block), out)
            .map_err(|_| VfsError::IoError)
    }

    /// Write a data block.
    fn write_data_block(&self, block: u32, data: &[u8; SECTOR_SIZE]) -> VfsResult<()> {
        if block == 0 {
            return Err(VfsError::InvalidArgument);
        }
        self.cache_mut().write_sector(self.data_sector(block), data)
            .map_err(|_| VfsError::IoError)
    }

    // ─── High-level FS operations ───────────────────────────────────────────

    /// Create a file in a directory. Returns new inode number.
    pub fn create_file(&self, parent_ino: u32, name: &str) -> VfsResult<u32> {
        self.create_entry(parent_ino, name, ITYPE_FILE, 0o644)
    }

    /// Create a subdirectory. Returns new inode number.
    pub fn create_dir(&self, parent_ino: u32, name: &str) -> VfsResult<u32> {
        self.create_entry(parent_ino, name, ITYPE_DIR, 0o755)
    }

    fn create_entry(&self, parent_ino: u32, name: &str, itype: u8, mode: u16) -> VfsResult<u32> {
        if name.len() > MAX_NAME_LEN - 4 {
            return Err(VfsError::InvalidArgument);
        }
        let parent = self.read_inode(parent_ino)?;
        if parent.itype != ITYPE_DIR {
            return Err(VfsError::NotADirectory);
        }
        // Check duplicate.
        if self.dir_lookup(&parent, name)?.is_some() {
            return Err(VfsError::AlreadyExists);
        }

        let new_ino = self.alloc_inode()?;
        let new_inode = DiskInode {
            itype,
            mode,
            _pad1: 0,
            size: 0,
            nlink: if itype == ITYPE_DIR { 2 } else { 1 },
            _pad2: 0,
            uid: 0,
            gid: 0,
            direct: [0u32; DIRECT_BLOCKS],
            dir_entry_count: 0,
            _reserved: [0u8; 128 - 56],
        };
        self.write_inode(new_ino, &new_inode)?;

        // Add directory entry to parent.
        self.dir_add_entry(parent_ino, new_ino, name)?;
        self.flush_sb()?;
        self.cache_mut().flush().map_err(|_| VfsError::IoError)?;
        Ok(new_ino)
    }

    /// Remove a file or empty directory from parent.
    pub fn unlink(&self, parent_ino: u32, name: &str) -> VfsResult<()> {
        let parent = self.read_inode(parent_ino)?;
        if parent.itype != ITYPE_DIR {
            return Err(VfsError::NotADirectory);
        }
        let child_ino = self.dir_lookup(&parent, name)?
            .ok_or(VfsError::NotFound)?;
        let child = self.read_inode(child_ino)?;

        // If directory, must be empty.
        if child.itype == ITYPE_DIR && child.dir_entry_count > 0 {
            return Err(VfsError::InvalidArgument); // ENOTEMPTY
        }

        // Remove dir entry from parent.
        self.dir_remove_entry(parent_ino, name)?;
        // Free child inode and its blocks.
        self.free_inode(child_ino)?;
        self.flush_sb()?;
        self.cache_mut().flush().map_err(|_| VfsError::IoError)?;
        Ok(())
    }

    /// Read file data.
    pub fn read_file(&self, ino: u32, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let inode = self.read_inode(ino)?;
        if inode.itype != ITYPE_FILE {
            return Err(VfsError::IsADirectory);
        }
        let size = inode.size as u64;
        if offset >= size {
            return Ok(0);
        }
        let avail = (size - offset) as usize;
        let to_read = buf.len().min(avail);

        let mut done = 0usize;
        let mut pos = offset as usize;
        while done < to_read {
            let block_idx = pos / SECTOR_SIZE;
            let block_off = pos % SECTOR_SIZE;
            if block_idx >= DIRECT_BLOCKS {
                break; // MVP: no indirect blocks
            }
            let block = inode.direct[block_idx];
            let mut sector = [0u8; SECTOR_SIZE];
            self.read_data_block(block, &mut sector)?;
            let chunk = (SECTOR_SIZE - block_off).min(to_read - done);
            buf[done..done + chunk].copy_from_slice(&sector[block_off..block_off + chunk]);
            done += chunk;
            pos += chunk;
        }
        Ok(done)
    }

    /// Write file data (extend as needed).
    pub fn write_file(&self, ino: u32, offset: u64, data: &[u8]) -> VfsResult<usize> {
        let mut inode = self.read_inode(ino)?;
        if inode.itype != ITYPE_FILE {
            return Err(VfsError::IsADirectory);
        }

        let end = offset as usize + data.len();
        // Allocate blocks as needed.
        let blocks_needed = (end + SECTOR_SIZE - 1) / SECTOR_SIZE;
        if blocks_needed > DIRECT_BLOCKS {
            return Err(VfsError::NoSpace); // MVP: direct blocks only
        }

        for i in 0..blocks_needed {
            if inode.direct[i] == 0 {
                inode.direct[i] = self.alloc_block()?;
                // Zero the new block.
                let zero = [0u8; SECTOR_SIZE];
                self.write_data_block(inode.direct[i], &zero)?;
            }
        }

        // Write data across blocks.
        let mut done = 0usize;
        let mut pos = offset as usize;
        while done < data.len() {
            let block_idx = pos / SECTOR_SIZE;
            let block_off = pos % SECTOR_SIZE;
            let block = inode.direct[block_idx];
            let mut sector = [0u8; SECTOR_SIZE];
            self.read_data_block(block, &mut sector)?;
            let chunk = (SECTOR_SIZE - block_off).min(data.len() - done);
            sector[block_off..block_off + chunk].copy_from_slice(&data[done..done + chunk]);
            self.write_data_block(block, &sector)?;
            done += chunk;
            pos += chunk;
        }

        if end as u32 > inode.size {
            inode.size = end as u32;
        }
        self.write_inode(ino, &inode)?;
        self.cache_mut().flush().map_err(|_| VfsError::IoError)?;
        Ok(data.len())
    }

    /// Look up a name in a directory inode's entries.
    fn dir_lookup(&self, inode: &DiskInode, name: &str) -> VfsResult<Option<u32>> {
        for i in 0..inode.dir_entry_count {
            let de = self.read_direntry(inode, i)?;
            let n = &de.name[..de.name_len as usize];
            if n == name.as_bytes() {
                return Ok(Some(de.ino));
            }
        }
        Ok(None)
    }

    /// Read directory entry i from an inode.
    fn read_direntry(&self, inode: &DiskInode, idx: u32) -> VfsResult<DiskDirEntry> {
        // Each data block holds DIR_ENTRIES_PER_BLOCK entries.
        let block_idx = idx as usize / DIR_ENTRIES_PER_BLOCK;
        let entry_in_block = idx as usize % DIR_ENTRIES_PER_BLOCK;
        if block_idx >= DIRECT_BLOCKS || inode.direct[block_idx] == 0 {
            return Err(VfsError::IoError);
        }
        let mut sector = [0u8; SECTOR_SIZE];
        self.read_data_block(inode.direct[block_idx], &mut sector)?;
        let off = entry_in_block * DIR_ENTRY_SIZE;
        Ok(direntry_from_bytes(&sector[off..off + DIR_ENTRY_SIZE]))
    }

    /// Add a directory entry to a directory inode.
    fn dir_add_entry(&self, dir_ino: u32, child_ino: u32, name: &str) -> VfsResult<()> {
        let mut dir = self.read_inode(dir_ino)?;
        let idx = dir.dir_entry_count;
        let block_idx = idx as usize / DIR_ENTRIES_PER_BLOCK;
        let entry_in_block = idx as usize % DIR_ENTRIES_PER_BLOCK;

        if block_idx >= DIRECT_BLOCKS {
            return Err(VfsError::NoSpace);
        }

        // Allocate data block if needed.
        if dir.direct[block_idx] == 0 {
            dir.direct[block_idx] = self.alloc_block()?;
            let zero = [0u8; SECTOR_SIZE];
            self.write_data_block(dir.direct[block_idx], &zero)?;
        }

        // Build entry.
        let mut de = DiskDirEntry {
            ino: child_ino,
            name_len: name.len() as u8,
            _pad: [0; 3],
            name: [0u8; MAX_NAME_LEN - 4],
        };
        let copy_len = name.len().min(de.name.len());
        de.name[..copy_len].copy_from_slice(&name.as_bytes()[..copy_len]);

        // Write entry into data block.
        let mut sector = [0u8; SECTOR_SIZE];
        self.read_data_block(dir.direct[block_idx], &mut sector)?;
        let off = entry_in_block * DIR_ENTRY_SIZE;
        let de_bytes = direntry_to_bytes(&de);
        sector[off..off + DIR_ENTRY_SIZE].copy_from_slice(&de_bytes);
        self.write_data_block(dir.direct[block_idx], &sector)?;

        dir.dir_entry_count += 1;
        self.write_inode(dir_ino, &dir)?;
        Ok(())
    }

    /// Remove a directory entry by name (compacts remaining entries).
    fn dir_remove_entry(&self, dir_ino: u32, name: &str) -> VfsResult<()> {
        let mut dir = self.read_inode(dir_ino)?;
        let count = dir.dir_entry_count;
        let mut found_idx = None;
        for i in 0..count {
            let de = self.read_direntry(&dir, i)?;
            let n = &de.name[..de.name_len as usize];
            if n == name.as_bytes() {
                found_idx = Some(i);
                break;
            }
        }
        let found = found_idx.ok_or(VfsError::NotFound)?;

        // Replace with last entry if not already last.
        if found < count - 1 {
            let last = self.read_direntry(&dir, count - 1)?;
            // Write last entry to found's position.
            let block_idx = found as usize / DIR_ENTRIES_PER_BLOCK;
            let entry_in_block = found as usize % DIR_ENTRIES_PER_BLOCK;
            let mut sector = [0u8; SECTOR_SIZE];
            self.read_data_block(dir.direct[block_idx], &mut sector)?;
            let off = entry_in_block * DIR_ENTRY_SIZE;
            let bytes = direntry_to_bytes(&last);
            sector[off..off + DIR_ENTRY_SIZE].copy_from_slice(&bytes);
            self.write_data_block(dir.direct[block_idx], &sector)?;
        }

        dir.dir_entry_count -= 1;
        self.write_inode(dir_ino, &dir)?;
        Ok(())
    }

    /// Lookup path from root. Returns inode number.
    pub fn lookup_path(&self, path: &str) -> VfsResult<u32> {
        let mut current: u32 = 0; // root
        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            let inode = self.read_inode(current)?;
            if inode.itype != ITYPE_DIR {
                return Err(VfsError::NotADirectory);
            }
            current = self.dir_lookup(&inode, component)?.ok_or(VfsError::NotFound)?;
        }
        Ok(current)
    }

    /// Split path into (parent_ino, leaf_name).
    pub fn split_parent_leaf<'a>(&self, path: &'a str) -> VfsResult<(u32, &'a str)> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Err(VfsError::InvalidArgument);
        }
        let leaf = parts[parts.len() - 1];
        let mut parent_ino: u32 = 0;
        for &part in &parts[..parts.len() - 1] {
            let inode = self.read_inode(parent_ino)?;
            if inode.itype != ITYPE_DIR {
                return Err(VfsError::NotADirectory);
            }
            parent_ino = self.dir_lookup(&inode, part)?.ok_or(VfsError::NotFound)?;
        }
        Ok((parent_ino, leaf))
    }

    /// List directory entries.
    pub fn readdir(&self, ino: u32) -> VfsResult<Vec<DirEntry>> {
        let inode = self.read_inode(ino)?;
        if inode.itype != ITYPE_DIR {
            return Err(VfsError::NotADirectory);
        }
        let mut entries = Vec::new();
        for i in 0..inode.dir_entry_count {
            let de = self.read_direntry(&inode, i)?;
            let child = self.read_inode(de.ino)?;
            let name_bytes = &de.name[..de.name_len as usize];
            let name = core::str::from_utf8(name_bytes).unwrap_or("???");
            entries.push(DirEntry {
                name: String::from(name),
                ino: de.ino as InodeNum,
                file_type: match child.itype {
                    ITYPE_DIR => FileType::Directory,
                    _ => FileType::Regular,
                },
            });
        }
        Ok(entries)
    }

    /// Get inode metadata.
    pub fn inode_metadata(&self, ino: u32) -> VfsResult<InodeMetadata> {
        let di = self.read_inode(ino)?;
        if di.itype == ITYPE_FREE {
            return Err(VfsError::NotFound);
        }
        let file_type = match di.itype {
            ITYPE_DIR => FileType::Directory,
            _ => FileType::Regular,
        };
        let mut meta = InodeMetadata::new(ino as InodeNum, file_type);
        meta.mode = FileMode::new(di.mode as u32);
        meta.size = di.size as u64;
        meta.nlink = di.nlink as u32;
        meta.uid = di.uid;
        meta.gid = di.gid;
        Ok(meta)
    }

    /// Update inode mode/uid/gid.
    pub fn set_inode_metadata(&self, ino: u32, meta: &InodeMetadata) -> VfsResult<()> {
        let mut di = self.read_inode(ino)?;
        if di.itype == ITYPE_FREE {
            return Err(VfsError::NotFound);
        }
        di.mode = (meta.mode.0 & 0o7777) as u16;
        di.uid = meta.uid;
        di.gid = meta.gid;
        self.write_inode(ino, &di)?;
        self.cache_mut().flush().map_err(|_| VfsError::IoError)?;
        Ok(())
    }
}

// ─── VFS trait adapters ─────────────────────────────────────────────────────

/// Inode adapter for racfs.
struct RacfsInode {
    ino: u32,
    fs: Arc<Racfs>,
}

impl InodeOps for RacfsInode {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let di = self.fs.read_inode(self.ino)?;
        if di.itype == ITYPE_DIR {
            return Err(VfsError::IsADirectory);
        }
        self.fs.read_file(self.ino, offset, buf)
    }

    fn write(&self, offset: u64, buf: &[u8]) -> VfsResult<usize> {
        self.fs.write_file(self.ino, offset, buf)
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        self.fs.inode_metadata(self.ino)
    }

    fn set_metadata(&self, meta: &InodeMetadata) -> VfsResult<()> {
        self.fs.set_inode_metadata(self.ino, meta)
    }

    fn lookup(&self, name: &str) -> VfsResult<InodeNum> {
        let di = self.fs.read_inode(self.ino)?;
        if di.itype != ITYPE_DIR {
            return Err(VfsError::NotADirectory);
        }
        self.fs.dir_lookup(&di, name)?.ok_or(VfsError::NotFound).map(|i| i as InodeNum)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        self.fs.readdir(self.ino)
    }

    fn ioctl(&self, _request: u64, _arg: u64) -> VfsResult<i64> {
        Err(VfsError::NotImplemented)
    }
}

/// Filesystem adapter.
pub struct RacfsFilesystem {
    inner: Arc<Racfs>,
}

impl RacfsFilesystem {
    pub fn new(racfs: Arc<Racfs>) -> Arc<Self> {
        Arc::new(RacfsFilesystem { inner: racfs })
    }

    /// Access the concrete Racfs backing this mount. Used by syscall handlers
    /// to route create/mkdir/unlink to the right disk, not a global singleton.
    pub fn inner(&self) -> Arc<Racfs> {
        self.inner.clone()
    }
}

impl Filesystem for RacfsFilesystem {
    fn root_inode(&self) -> Arc<dyn InodeOps> {
        Arc::new(RacfsInode { ino: 0, fs: self.inner.clone() })
    }

    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>> {
        // Validate inode exists.
        let di = self.inner.read_inode(ino as u32)?;
        if di.itype == ITYPE_FREE {
            return Err(VfsError::NotFound);
        }
        Ok(Arc::new(RacfsInode { ino: ino as u32, fs: self.inner.clone() }))
    }

    fn name(&self) -> &str {
        "racfs"
    }

    fn as_any(&self) -> &dyn core::any::Any { self }
}

// ─── Global instance ────────────────────────────────────────────────────────

static mut RACFS_INSTANCE: Option<Arc<Racfs>> = None;

/// Initialize racfs on the first registered block device (ram0).
///
/// # Safety
/// Must be called once during kernel init after block devices are ready.
pub unsafe fn init() -> Arc<Racfs> {
    let dev = crate::drivers::block::find("ram0")
        .expect("racfs: no ram0 block device found");
    let racfs = Racfs::format_and_new(dev)
        .expect("racfs: format failed");
    let inst = &mut *core::ptr::addr_of_mut!(RACFS_INSTANCE);
    *inst = Some(racfs.clone());
    crate::serial::serial_println!("[  0.000360] RACORE: racfs initialized (block-device-backed on ram0)");
    racfs
}

/// Get the global racfs instance.
///
/// # Safety
/// Must be called after init().
pub unsafe fn instance() -> &'static Arc<Racfs> {
    (*core::ptr::addr_of!(RACFS_INSTANCE)).as_ref().unwrap()
}

/// Persistence smoke test for a mounted racfs. Looks up `boot-counter` in the
/// root; if it exists, reads the integer, increments it and writes it back; if
/// not, creates it with `1`. Run on each boot to show that file contents
/// survive reboots across the entire on-disk format (inodes, bitmap, data
/// blocks, directory entries).
pub fn persistence_test(fs: &Racfs, label: &str) {
    use alloc::string::ToString;
    const NAME: &str = "boot-counter";
    match fs.lookup_path(NAME) {
        Ok(ino) => {
            let mut buf = [0u8; 16];
            let n = fs.read_file(ino, 0, &mut buf).unwrap_or(0);
            let text = core::str::from_utf8(&buf[..n]).unwrap_or("0");
            let value: u32 = text.trim().parse().unwrap_or(0);
            let next = value.saturating_add(1);
            let s = next.to_string();
            // Write the new counter back over the old contents.
            let _ = fs.write_file(ino, 0, s.as_bytes());
            crate::serial::serial_println!(
                "[ RACFS {} ] boot-counter = {} (was {}, file survived reboot)",
                label, next, value,
            );
        }
        Err(_) => {
            match fs.create_file(0, NAME) {
                Ok(ino) => {
                    let _ = fs.write_file(ino, 0, b"1");
                    crate::serial::serial_println!(
                        "[ RACFS {} ] created boot-counter = 1 (first boot)", label
                    );
                }
                Err(e) => crate::serial::serial_println!(
                    "[ RACFS {} ] create boot-counter failed: {:?}", label, e
                ),
            }
        }
    }
}

