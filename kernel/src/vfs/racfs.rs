// RaCore — racfs: block-device-backed writable filesystem (Phase B)
//
// On-disk layout (all sizes in 512-byte sectors):
//   Sector 0:                    Superblock (magic, version, counts, offsets)
//   [1, bitmap_start):           Metadata journal (header + data slots)
//   [bitmap_start, inode_start): Free-block bitmap (1 bit per data block)
//   [inode_start, data_start):   Inode table (fixed-size 128-byte inodes)
//   [data_start, total_sectors): Data blocks (512 bytes each)
//
// The bitmap spans however many sectors it takes to describe every data
// block. Its length is not a superblock field: it is `inode_start -
// bitmap_start`, which the layout has always implied. Deriving it rather than
// adding a field keeps images written by the single-sector version mountable
// - they report a length of 1, which is exactly what they have.
//
// The journal's length is derived the same way, as `bitmap_start - 1`. An
// image written before the journal existed puts the bitmap at sector 1, so it
// reports a journal of length 0 and runs unjournaled - which is exactly what
// it is. Neither region needed a format version bump.
//
// Constraints:
// - Max 128 inodes
// - Max file size (8 direct + 128 single-indirect + 128*128 double-indirect)
//   * 512 B = 8.06 MiB
// - Backed by a BlockDevice through BlockCache
// - CLI/STI serialization (single-core)

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::{Cell, UnsafeCell};

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

/// Block pointers that fit in one indirect block (512 B / 4 B).
const POINTERS_PER_BLOCK: usize = SECTOR_SIZE / 4; // 128

/// Logical blocks reachable through the single-indirect pointer.
const SINGLE_INDIRECT_BLOCKS: usize = POINTERS_PER_BLOCK; // 128

/// Logical blocks reachable through the double-indirect pointer.
const DOUBLE_INDIRECT_BLOCKS: usize = POINTERS_PER_BLOCK * POINTERS_PER_BLOCK; // 16384

/// Highest logical block index a file can address, exclusive.
const MAX_FILE_BLOCKS: usize = DIRECT_BLOCKS + SINGLE_INDIRECT_BLOCKS + DOUBLE_INDIRECT_BLOCKS;

/// Data blocks one sector of allocation bitmap can describe.
const BLOCKS_PER_BITMAP_SECTOR: usize = SECTOR_SIZE * 8; // 4096

// --- Metadata journal -----------------------------------------------------

/// Journal header magic, "RJL1".
const JOURNAL_MAGIC: u32 = 0x524A_4C31;

/// Header states. The transition between them is a single sector write, and
/// that write is the moment an operation becomes durable.
const JOURNAL_EMPTY: u32 = 0;
const JOURNAL_COMMITTED: u32 = 1;

/// Sectors one transaction may log. The header carries their target LBAs as
/// u64s after a 24-byte preamble, so (512 - 24) / 8 = 61 fit; 60 leaves the
/// arithmetic obviously safe.
const MAX_JOURNAL_ENTRIES: usize = 60;

/// Total journal size: one header plus its data slots.
const JOURNAL_SECTORS: u32 = 1 + MAX_JOURNAL_ENTRIES as u32;

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
    bitmap_start: u32, // sector offset
    inode_start: u32,  // sector offset
    data_start: u32,   // sector offset
    free_inodes: u32,
    free_blocks: u32,
    _pad: [u8; 512 - 40],
}

impl Superblock {
    /// How many sectors the allocation bitmap occupies.
    ///
    /// Derived from the gap between `bitmap_start` and `inode_start` rather
    /// than stored, so an image written before the bitmap could span more than
    /// one sector still reads correctly: its gap is 1, which is what it has. A
    /// stored field would have needed a format version bump, making every
    /// existing disk unmountable to buy nothing.
    fn bitmap_sectors(&self) -> u32 {
        self.inode_start.saturating_sub(self.bitmap_start).max(1)
    }

    /// Data blocks the bitmap can actually describe. Anything past this is
    /// unreachable: `alloc_block` never scans there.
    fn addressable_blocks(&self) -> usize {
        (self.bitmap_sectors() as usize).saturating_mul(BLOCKS_PER_BITMAP_SECTOR)
    }

    /// First journal sector. The journal always starts right after the
    /// superblock; only its length varies.
    fn journal_start(&self) -> u64 {
        1
    }

    /// Sectors the journal occupies, derived like the bitmap's length.
    /// Zero on an image written before the journal existed.
    fn journal_sectors(&self) -> u32 {
        self.bitmap_start.saturating_sub(1)
    }

    /// Whether this filesystem can journal at all. A header with no data slot
    /// behind it cannot log anything, so one sector does not count.
    fn has_journal(&self) -> bool {
        self.journal_sectors() > 1
    }

    /// How many data slots the journal actually has, capped by what a header
    /// can address.
    fn journal_capacity(&self) -> usize {
        (self.journal_sectors().saturating_sub(1) as usize).min(MAX_JOURNAL_ENTRIES)
    }
}

/// What a consistency check found. Counts rather than a verdict, so the
/// caller decides whether to warn, repair, or refuse to mount.
#[derive(Default, Clone, Copy, Debug)]
pub struct FsckReport {
    /// Blocks marked used in the bitmap that no live inode references.
    /// Wasted space, but safe: nothing will overwrite live data.
    pub leaked_blocks: u32,
    /// Blocks a live inode references but the bitmap calls free. Dangerous —
    /// the allocator will hand them out again and corrupt the file that
    /// already owns them.
    pub unallocated_in_use: u32,
    /// Blocks claimed by more than one inode. Also dangerous, and not
    /// repairable without deciding which owner is right.
    pub doubly_claimed: u32,
    /// Directory entries pointing at a free inode slot.
    pub dangling_entries: u32,
    /// Directory entries whose inode number is out of range.
    pub out_of_range_entries: u32,
    /// Superblock `free_blocks` minus what the bitmap actually says.
    ///
    /// This counts only blocks the bitmap can describe. A disk formatted by
    /// this version sizes its bitmap to cover every data block, so the two
    /// agree. An image from the single-sector era has a bitmap covering the
    /// first 4096 blocks only, and there the superblock's figure legitimately
    /// exceeds it — the drift then reports a real, if harmless, disagreement
    /// about blocks nothing can allocate.
    pub superblock_free_blocks_drift: i64,

    /// Data blocks the allocation bitmap cannot describe, and which are
    /// therefore unreachable. Zero on anything this version formatted;
    /// non-zero on an image from the single-sector era.
    pub unaddressable_blocks: u32,
}

impl FsckReport {
    /// Nothing at all was wrong.
    ///
    /// `unaddressable_blocks` is deliberately excluded: wasted capacity on a
    /// legacy image is a fact about its layout, not damage, and reporting a
    /// healthy old disk as unclean would train the reader to ignore the line.
    pub fn is_clean(&self) -> bool {
        self.leaked_blocks == 0
            && self.unallocated_in_use == 0
            && self.doubly_claimed == 0
            && self.dangling_entries == 0
            && self.out_of_range_entries == 0
            && self.superblock_free_blocks_drift == 0
    }

    /// Damage that can corrupt live data if the filesystem is used as-is.
    /// Leaked blocks and superblock drift are untidy but safe; these are not.
    pub fn is_dangerous(&self) -> bool {
        self.unallocated_in_use > 0 || self.doubly_claimed > 0
    }
}

/// On-disk inode (128 bytes).
#[derive(Clone, Copy)]
#[repr(C)]
struct DiskInode {
    itype: u8, // ITYPE_*
    mode: u16, // permission bits
    _pad1: u8,
    size: u32, // file size in bytes
    nlink: u16,
    _pad2: u16,
    uid: u32,
    gid: u32,
    /// Direct block indices (relative to data_start).
    direct: [u32; DIRECT_BLOCKS], // 32 bytes
    /// Number of directory entries (for dirs).
    dir_entry_count: u32,
    /// Block holding POINTERS_PER_BLOCK further block indices, or 0.
    indirect: u32,
    /// Block holding POINTERS_PER_BLOCK single-indirect block indices, or 0.
    double_indirect: u32,
    _reserved: [u8; 128 - 64],
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
    buf[off] = b[0];
    buf[off + 1] = b[1];
}
fn write_u32_le(buf: &mut [u8], off: usize, v: u32) {
    let b = v.to_le_bytes();
    buf[off] = b[0];
    buf[off + 1] = b[1];
    buf[off + 2] = b[2];
    buf[off + 3] = b[3];
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
        // Bytes 52.. were reserved and always written as zero, so an inode
        // from before indirect blocks existed decodes as "no indirect
        // blocks" - which is exactly true of it.
        indirect: read_u32_le(buf, 52),
        double_indirect: read_u32_le(buf, 56),
        _reserved: [0u8; 128 - 64],
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
    write_u32_le(&mut buf, 52, inode.indirect);
    write_u32_le(&mut buf, 56, inode.double_indirect);
    buf
}

/// Decode an on-disk directory entry.
///
/// `buf` is raw disk content and is therefore untrusted: a torn write or a
/// stale slot can put arbitrary bytes here, so `buf[4]` may claim a name far
/// longer than `name` can hold. The returned struct upholds the invariant
/// `name_len as usize <= name.len()`, which every consumer relies on to slice
/// `name[..name_len]`. Without the clamp a corrupt entry took the whole kernel
/// down with "range end index N out of range" instead of surfacing as a
/// filesystem error.
fn direntry_from_bytes(buf: &[u8]) -> DiskDirEntry {
    let mut name = [0u8; MAX_NAME_LEN - 4];
    let copy_len = (buf[4] as usize).min(name.len());
    name[..copy_len].copy_from_slice(&buf[8..8 + copy_len]);
    DiskDirEntry {
        ino: read_u32_le(buf, 0),
        name_len: copy_len as u8,
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
    /// Data-block index where the next allocation scan starts (0-based).
    ///
    /// Purely an optimisation, and deliberately not on disk: a stale hint
    /// costs one wasted scan, never a wrong answer, so there is nothing to
    /// keep consistent across a crash. Without it every allocation restarts
    /// from block 0 and re-reads the same full bitmap sectors, which on a
    /// multi-sector bitmap is the difference between one sector read per
    /// allocation and a dozen.
    alloc_hint: Cell<u32>,
    /// Sequence number stamped on the next journal commit. Diagnostic only -
    /// correctness comes from the header's state field, not from this - but a
    /// replay that can name which transaction it restored is a great deal
    /// easier to reason about after a crash.
    journal_seq: Cell<u64>,
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
        cache
            .read_sector(0, &mut buf)
            .map_err(|_| VfsError::IoError)?;
        let sb = superblock_from_sector(&buf);
        if sb.magic != RACFS_MAGIC || sb.version != RACFS_VERSION {
            return Err(VfsError::IoError);
        }
        // Layout sanity, checked because the bitmap's length is *derived*
        // from these offsets. A superblock claiming `inode_start <=
        // bitmap_start` would clamp the bitmap to one sector and leave
        // `free_block` writing bits into the inode table. Refusing here turns
        // that into a mount failure, which `open_or_format` handles.
        if sb.bitmap_start == 0
            || sb.inode_start <= sb.bitmap_start
            || sb.data_start <= sb.inode_start
        {
            return Err(VfsError::IoError);
        }
        Ok(Arc::new(Racfs {
            cache: UnsafeCell::new(cache),
            sb: UnsafeCell::new(sb),
            alloc_hint: Cell::new(0),
            journal_seq: Cell::new(1),
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

        // Layout calculation. The journal sits between the superblock and the
        // bitmap, so `bitmap_start` is where its length is recorded - see the
        // header comment on why neither region is a superblock field.
        let bitmap_start: u32 = 1 + JOURNAL_SECTORS;
        let inode_sectors = MAX_INODES.div_ceil(INODES_PER_SECTOR);

        // Size the bitmap so it describes every data block, rather than
        // fixing it at one sector and stranding the rest of the device.
        //
        // It is a fixed point because the bitmap's size and the data area's
        // size each depend on the other: every sector added to the bitmap
        // costs one data block and buys BLOCKS_PER_BITMAP_SECTOR of
        // description. Growing the bitmap can only shrink the data area, so
        // the requirement never rises after the first round and the loop
        // settles in two passes; the iteration cap is a guard against a
        // future edit breaking that monotonicity, not a real bound.
        let mut bitmap_sectors: u32 = 1;
        for _ in 0..8 {
            let overhead = 1 + JOURNAL_SECTORS + bitmap_sectors + inode_sectors as u32;
            let data_blocks = total_sectors.saturating_sub(overhead) as usize;
            let needed = data_blocks.div_ceil(BLOCKS_PER_BITMAP_SECTOR).max(1) as u32;
            if needed <= bitmap_sectors {
                break;
            }
            bitmap_sectors = needed;
        }

        let inode_start = bitmap_start + bitmap_sectors;
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
        cache
            .write_sector(0, &sb_buf)
            .map_err(|_| VfsError::IoError)?;

        // An empty journal header. Without it a fresh disk's journal sector
        // holds whatever was there before, and a stale committed header would
        // replay a previous filesystem's sectors over this one at first mount.
        let zero = [0u8; SECTOR_SIZE];
        {
            let mut header = [0u8; SECTOR_SIZE];
            write_u32_le(&mut header, 0, JOURNAL_MAGIC);
            write_u32_le(&mut header, 4, JOURNAL_EMPTY);
            cache
                .write_sector(1, &header)
                .map_err(|_| VfsError::IoError)?;
        }

        // Zero bitmap. Every sector of it: a stale sector left from a
        // previous filesystem would read as "allocated" and quietly cost the
        // new one 4096 blocks.
        for s in 0..bitmap_sectors {
            cache
                .write_sector((bitmap_start + s) as u64, &zero)
                .map_err(|_| VfsError::IoError)?;
        }

        // Zero inode table.
        for s in 0..inode_sectors {
            cache
                .write_sector((inode_start as usize + s) as u64, &zero)
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
            indirect: 0,
            double_indirect: 0,
            _reserved: [0u8; 128 - 64],
        };
        let fs = Arc::new(Racfs {
            cache: UnsafeCell::new(cache),
            sb: UnsafeCell::new(sb),
            alloc_hint: Cell::new(0),
            journal_seq: Cell::new(1),
        });
        fs.write_inode(0, &root_inode)?;
        fs.cache_mut().flush().map_err(|_| VfsError::IoError)?;

        crate::serial::serial_println!(
            "[  RACFS  ] Formatted: {} sectors, {} data blocks, {} inodes, {} bitmap sectors, {} journal sectors",
            total_sectors,
            data_block_count,
            MAX_INODES,
            bitmap_sectors,
            JOURNAL_SECTORS
        );

        Ok(fs)
    }

    fn cache_mut(&self) -> &mut BlockCache {
        // SAFETY: racfs is single-CPU MVP; callers run inside their own syscall.
        unsafe { &mut *self.cache.get() }
    }

    fn sb(&self) -> &Superblock {
        // SAFETY: see cache_mut().
        unsafe { &*self.sb.get() }
    }

    fn sb_mut(&self) -> &mut Superblock {
        // SAFETY: see cache_mut().
        unsafe { &mut *self.sb.get() }
    }

    /// Public stats snapshot (total_blocks, free_blocks, total_inodes, free_inodes).
    /// Block size is SECTOR_SIZE (512 B). Used by /proc/diskstats.
    pub fn stats(&self) -> (u32, u32, u32, u32) {
        let sb = self.sb();
        // Report the blocks the bitmap can describe, not every block on the
        // device. They are the same figure on anything this version
        // formatted; on a single-sector-era image the rest is unreachable,
        // and `df` promising 16 MiB where `alloc_block` will only ever find
        // 2 MiB is a lie the user then has to debug.
        let addressable = sb.addressable_blocks().min(u32::MAX as usize) as u32;
        let total = sb.data_block_count.min(addressable);
        let unreachable = sb.data_block_count.saturating_sub(addressable);
        (
            total,
            sb.free_blocks.saturating_sub(unreachable),
            sb.inode_count,
            sb.free_inodes,
        )
    }

    /// Force all dirty cache entries to disk. Idempotent — no-op if nothing
    /// is dirty. Called by sys_sync and the periodic flushd kernel task.
    pub fn sync(&self) -> VfsResult<()> {
        // The superblock too: it lives in memory and only reaches the disk
        // when something writes it. `sync` promising durability while
        // leaving `free_blocks` stale was a lie by omission - and the
        // sb_drift=-1 that kept showing up on "cleanly" seeded images.
        self.flush_sb()?;
        self.cache_mut().flush().map_err(|_| VfsError::IoError)
    }

    /// Cache counters: (hits, misses, cached_entries, dirty_entries).
    /// Used by /proc/cachestats.
    pub fn cache_stats(&self) -> (u64, u64, usize, usize) {
        let cache = self.cache_mut();
        let (h, m, e) = cache.stats();
        (h, m, e, cache.dirty_count())
    }

    /// Flush the superblock to disk.
    fn flush_sb(&self) -> VfsResult<()> {
        let buf = superblock_to_sector(self.sb());
        self.cache_mut()
            .write_sector(0, &buf)
            .map_err(|_| VfsError::IoError)
    }

    // --- Metadata journal ---------------------------------------------------
    //
    // Write-ahead log for metadata. An operation's sectors are copied into the
    // journal and a commit record is written before any of them is allowed to
    // land in place, so a crash leaves the filesystem either entirely before
    // the operation or entirely after it - never in the half-applied state
    // that once made a directory block read as file text and took the kernel
    // down with `name_len = 111`.
    //
    // File *data* is deliberately not journalled. Logging it would double
    // every write and bound file size by the journal; losing the tail of a
    // file that was being written during a crash is the normal, expected
    // outcome, whereas losing the directory that names it is not.

    fn journal_header_sector(&self) -> u64 {
        self.sb().journal_start()
    }

    fn journal_slot_sector(&self, index: usize) -> u64 {
        self.sb().journal_start() + 1 + index as u64
    }

    /// Write the journal header. This single sector write is the atomic point
    /// of the whole scheme: before it the operation did not happen, after it
    /// the operation is guaranteed to complete, if necessary by replay.
    fn write_journal_header(&self, state: u32, seq: u64, lbas: &[u64]) -> VfsResult<()> {
        let mut buf = [0u8; SECTOR_SIZE];
        write_u32_le(&mut buf, 0, JOURNAL_MAGIC);
        write_u32_le(&mut buf, 4, state);
        buf[8..16].copy_from_slice(&seq.to_le_bytes());
        write_u32_le(&mut buf, 16, lbas.len() as u32);
        for (i, lba) in lbas.iter().enumerate() {
            let off = 24 + i * 8;
            buf[off..off + 8].copy_from_slice(&lba.to_le_bytes());
        }
        let sector = self.journal_header_sector();
        self.cache_mut()
            .write_through(sector, &buf)
            .map_err(|_| VfsError::IoError)
    }

    /// Read the journal header as (state, seq, target LBAs).
    ///
    /// A header whose magic does not match is treated as an empty journal
    /// rather than an error: that is what an image from before the journal
    /// existed looks like, and refusing to mount it would be wrong.
    fn read_journal_header(&self) -> VfsResult<(u32, u64, Vec<u64>)> {
        let mut buf = [0u8; SECTOR_SIZE];
        let sector = self.journal_header_sector();
        self.cache_mut()
            .read_through(sector, &mut buf)
            .map_err(|_| VfsError::IoError)?;

        if read_u32_le(&buf, 0) != JOURNAL_MAGIC {
            return Ok((JOURNAL_EMPTY, 0, Vec::new()));
        }
        let state = read_u32_le(&buf, 4);
        let seq = u64::from_le_bytes([
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]);
        // Clamped against what the journal can hold, not trusted: a torn or
        // stale header could claim any count, and replaying that many slots
        // would copy unrelated sectors over live metadata.
        let count = (read_u32_le(&buf, 16) as usize).min(self.sb().journal_capacity());
        let mut lbas = Vec::with_capacity(count);
        for i in 0..count {
            let off = 24 + i * 8;
            lbas.push(u64::from_le_bytes([
                buf[off],
                buf[off + 1],
                buf[off + 2],
                buf[off + 3],
                buf[off + 4],
                buf[off + 5],
                buf[off + 6],
                buf[off + 7],
            ]));
        }
        Ok((state, seq, lbas))
    }

    /// Finish any transaction a crash interrupted. Returns how many sectors
    /// were restored, and the sequence number of the transaction they came
    /// from.
    ///
    /// Runs before the consistency check, so `check()` sees the filesystem as
    /// the completed operation left it rather than mid-write.
    pub fn replay_journal(&self) -> VfsResult<(u32, u64)> {
        if !self.sb().has_journal() {
            return Ok((0, 0));
        }
        let (state, seq, lbas) = self.read_journal_header()?;
        if state != JOURNAL_COMMITTED || lbas.is_empty() {
            return Ok((0, 0));
        }

        let total_sectors = self.sb().total_sectors as u64;
        let mut restored = 0u32;
        for (i, &lba) in lbas.iter().enumerate() {
            // A target outside the device means the header is not
            // trustworthy. Skip it rather than write somewhere arbitrary;
            // the check that follows the mount will report what is left.
            if lba >= total_sectors {
                continue;
            }
            let mut buf = [0u8; SECTOR_SIZE];
            self.cache_mut()
                .read_through(self.journal_slot_sector(i), &mut buf)
                .map_err(|_| VfsError::IoError)?;
            self.cache_mut()
                .write_through(lba, &buf)
                .map_err(|_| VfsError::IoError)?;
            restored += 1;
        }

        // Only once every sector is in place. Clearing the header first would
        // turn a crash during replay into permanent corruption; as written, a
        // crash mid-replay simply replays again next boot, which is safe
        // because writing the same sector twice is the same as writing it
        // once.
        self.write_journal_header(JOURNAL_EMPTY, seq, &[])?;

        // The superblock was very likely one of the replayed sectors.
        let mut sb_buf = [0u8; SECTOR_SIZE];
        self.cache_mut()
            .read_through(0, &mut sb_buf)
            .map_err(|_| VfsError::IoError)?;
        *self.sb_mut() = superblock_from_sector(&sb_buf);
        self.journal_seq.set(seq.saturating_add(1));

        Ok((restored, seq))
    }

    /// Run `op` as one journalled transaction.
    ///
    /// While it runs, every sector it writes is pinned in the cache: neither
    /// eviction nor `flush()` may put one on the device, so nothing lands in
    /// place until the commit record says the whole operation exists. On
    /// failure the pinned sectors are dropped and the superblock re-read,
    /// which is a complete rollback precisely because none of them ever
    /// reached the disk.
    fn transaction<T>(&self, op: impl FnOnce() -> VfsResult<T>) -> VfsResult<T> {
        if !self.sb().has_journal() {
            // An image from before the journal existed. Behave exactly as
            // that image always has: correct, just not crash-atomic.
            return op();
        }

        self.cache_mut().begin_recording();
        let result = op();

        match result {
            Ok(value) => {
                // The superblock changes with almost every metadata operation
                // (free_inodes, free_blocks) and has to be part of the same
                // transaction; a commit without it replays into a filesystem
                // whose counters disagree with its bitmap.
                let sb_result = self.flush_sb();
                let sectors: Vec<u64> = self.cache_mut().recorded().to_vec();
                self.cache_mut().end_recording();
                sb_result?;
                self.journal_commit(&sectors)?;
                Ok(value)
            }
            Err(e) => {
                self.cache_mut().discard_recorded();
                let mut sb_buf = [0u8; SECTOR_SIZE];
                if self.cache_mut().read_through(0, &mut sb_buf).is_ok() {
                    *self.sb_mut() = superblock_from_sector(&sb_buf);
                }
                Err(e)
            }
        }
    }

    /// Log `sectors`, mark the log committed, apply them, then clear the log.
    fn journal_commit(&self, sectors: &[u64]) -> VfsResult<()> {
        if sectors.is_empty() {
            return Ok(());
        }

        // Too large to log. The operation is already staged in the cache and
        // is correct; what it loses is atomicity, so it degrades to the
        // pre-journal behaviour rather than failing. Said out loud, because a
        // filesystem quietly dropping its crash guarantee is worth noticing.
        if sectors.len() > self.sb().journal_capacity() {
            crate::serial::serial_println!(
                "[  RACFS  ] transaction of {} sectors exceeds the {}-slot journal; writing unjournaled",
                sectors.len(),
                self.sb().journal_capacity()
            );
            return self.cache_mut().flush().map_err(|_| VfsError::IoError);
        }

        // 1. The operation's sectors, as the cache has them, into the log.
        for (i, &lba) in sectors.iter().enumerate() {
            let mut buf = [0u8; SECTOR_SIZE];
            self.cache_mut()
                .peek_sector(lba, &mut buf)
                .map_err(|_| VfsError::IoError)?;
            self.cache_mut()
                .write_through(self.journal_slot_sector(i), &buf)
                .map_err(|_| VfsError::IoError)?;
        }

        // 2. The commit record. Everything before this point is undone by a
        //    crash; everything after it is completed by one.
        let seq = self.journal_seq.get();
        self.write_journal_header(JOURNAL_COMMITTED, seq, sectors)?;
        self.journal_seq.set(seq.saturating_add(1));

        // 3. Apply. Recording has ended, so these are no longer pinned.
        self.cache_mut().flush().map_err(|_| VfsError::IoError)?;

        // 4. Retire the log. A crash before this replays a transaction that
        //    is already applied, which changes nothing.
        self.write_journal_header(JOURNAL_EMPTY, seq, &[])
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
        self.cache_mut()
            .read_sector(sector, &mut buf)
            .map_err(|_| VfsError::IoError)?;
        Ok(inode_from_bytes(
            &buf[offset_in_sector..offset_in_sector + INODE_DISK_SIZE],
        ))
    }

    /// Write an on-disk inode by number.
    fn write_inode(&self, ino: u32, inode: &DiskInode) -> VfsResult<()> {
        let sb = self.sb();
        let sector = sb.inode_start as u64 + (ino as u64 / INODES_PER_SECTOR as u64);
        let offset_in_sector = (ino as usize % INODES_PER_SECTOR) * INODE_DISK_SIZE;

        let mut buf = [0u8; SECTOR_SIZE];
        self.cache_mut()
            .read_sector(sector, &mut buf)
            .map_err(|_| VfsError::IoError)?;
        let inode_bytes = inode_to_bytes(inode);
        buf[offset_in_sector..offset_in_sector + INODE_DISK_SIZE].copy_from_slice(&inode_bytes);
        self.cache_mut()
            .write_sector(sector, &buf)
            .map_err(|_| VfsError::IoError)
    }

    /// Walk the filesystem and compare what the inodes claim against what the
    /// bitmap and superblock say (ROADMAP v0.3 §3.2).
    ///
    /// Read-only: it reports, it does not repair. Repair means choosing an
    /// owner for a doubly-claimed block, and that choice belongs to whoever
    /// can see which file matters — not to a boot-time routine.
    ///
    /// The block allocator is a linear bitmap scan, so an inconsistency here
    /// is not academic: a block the bitmap calls free while an inode still
    /// uses it *will* be handed out again, and the two files will scribble
    /// over each other.
    pub fn check(&self) -> VfsResult<FsckReport> {
        let sb = *self.sb();
        let total_blocks = sb.data_block_count as usize;
        let mut report = FsckReport::default();

        // Per-block reference count from the inode side. u8 saturating: any
        // count above 1 is already "doubly claimed", the exact number does
        // not change the verdict.
        let mut refs: Vec<u8> = alloc::vec![0u8; total_blocks];

        for ino in 0..sb.inode_count {
            let di = self.read_inode(ino)?;
            if di.itype == ITYPE_FREE {
                continue;
            }

            // Data blocks *and* the indirect blocks holding their pointers:
            // both are allocated from the same bitmap, so counting only the
            // data blocks would report every indirect block as leaked.
            for block in self.owned_blocks(&di) {
                // Data blocks are 1-based on disk.
                let idx = (block - 1) as usize;
                if idx >= total_blocks {
                    // A block pointer past the end of the device: count it
                    // with the doubly-claimed group, since it is the same
                    // class of "this inode's block list is not trustworthy".
                    report.doubly_claimed += 1;
                    continue;
                }
                refs[idx] = refs[idx].saturating_add(1);
                if refs[idx] == 2 {
                    report.doubly_claimed += 1;
                }
            }

            // Directory entries must point at inodes that are in range and
            // actually allocated.
            if di.itype == ITYPE_DIR {
                for i in 0..di.dir_entry_count {
                    let de = match self.read_direntry(&di, i) {
                        Ok(d) => d,
                        // An unreadable entry means the directory's block
                        // list is already broken; the block-level counts
                        // above have recorded that.
                        Err(_) => break,
                    };
                    if de.name_len == 0 {
                        continue;
                    }
                    if de.ino >= sb.inode_count {
                        report.out_of_range_entries += 1;
                        continue;
                    }
                    if self.read_inode(de.ino)?.itype == ITYPE_FREE {
                        report.dangling_entries += 1;
                    }
                }
            }
        }

        // Now the bitmap side, one sector at a time. A disk this version
        // formatted has a bitmap covering every data block; an image from the
        // single-sector era covers only the first 4096, and the remainder is
        // dead space that no allocation can ever reach.
        let addressable = sb.addressable_blocks();
        let tracked = total_blocks.min(addressable);
        report.unaddressable_blocks = total_blocks.saturating_sub(addressable) as u32;

        let mut used_per_bitmap: i64 = 0;
        let mut idx = 0usize;
        while idx < tracked {
            let sector_no = idx / BLOCKS_PER_BITMAP_SECTOR;
            let mut bitmap = [0u8; SECTOR_SIZE];
            self.cache_mut()
                .read_sector(sb.bitmap_start as u64 + sector_no as u64, &mut bitmap)
                .map_err(|_| VfsError::IoError)?;

            let sector_end = ((sector_no + 1) * BLOCKS_PER_BITMAP_SECTOR).min(tracked);
            while idx < sector_end {
                let bit = idx % BLOCKS_PER_BITMAP_SECTOR;
                let marked_used = bitmap[bit / 8] & (1 << (bit % 8)) != 0;
                let referenced = refs[idx] > 0;
                match (marked_used, referenced) {
                    (true, false) => report.leaked_blocks += 1,
                    (false, true) => report.unallocated_in_use += 1,
                    _ => {}
                }
                if marked_used {
                    used_per_bitmap += 1;
                }
                idx += 1;
            }
        }

        // A reference to a block the bitmap cannot even describe means the
        // inode points somewhere the allocator could never have given it.
        for idx in tracked..total_blocks {
            if refs[idx] > 0 {
                report.unallocated_in_use += 1;
            }
        }

        // Compare *used* counts, not free ones. The superblock's free_blocks
        // covers the whole device including blocks the single-sector bitmap
        // cannot describe, so subtracting the two free figures would report a
        // huge drift on a perfectly healthy disk. Allocation only ever happens
        // inside the tracked range, so the used counts are directly
        // comparable.
        let used_per_superblock = total_blocks as i64 - sb.free_blocks as i64;
        report.superblock_free_blocks_drift = used_per_superblock - used_per_bitmap;
        Ok(report)
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
        self.free_block_map(&mut di);
        self.write_inode(ino, &di)?;
        self.sb_mut().free_inodes += 1;
        Ok(())
    }

    /// Allocate a data block from the bitmap. Returns a 1-based data-block
    /// index (0 is the "no block" sentinel).
    ///
    /// Scans from `alloc_hint` to the end of the addressable range and then
    /// wraps. The wrap is not optional: without it, a block freed below the
    /// hint would stay unreachable until the next mount, and a filesystem
    /// that churns files would report free space it refused to hand out.
    fn alloc_block(&self) -> VfsResult<u32> {
        let sb = *self.sb();
        if sb.free_blocks == 0 {
            return Err(VfsError::NoSpace);
        }
        // Blocks the bitmap cannot describe are not allocatable - there is no
        // bit to set for them. On a disk this version formatted the two
        // figures are equal; on a single-sector-era image the bitmap is
        // smaller than the data area, and the excess is simply dead space.
        let total = (sb.data_block_count as usize).min(sb.addressable_blocks());
        if total == 0 {
            return Err(VfsError::NoSpace);
        }
        let start = (self.alloc_hint.get() as usize).min(total - 1);

        // Two passes cover every block exactly once: [start, total), [0, start).
        for (from, to) in [(start, total), (0, start)] {
            if let Some(idx) = self.claim_first_free(from, to)? {
                self.alloc_hint.set((idx as u32).saturating_add(1));
                self.sb_mut().free_blocks -= 1;
                return Ok(idx as u32 + 1);
            }
        }
        Err(VfsError::NoSpace)
    }

    /// Claim the first free block in `[from, to)` and return its 0-based
    /// index, or None if that range is full. Reads one bitmap sector at a
    /// time so a bitmap larger than a sector costs reads in proportion to how
    /// far the scan actually travels.
    fn claim_first_free(&self, from: usize, to: usize) -> VfsResult<Option<usize>> {
        let sb = *self.sb();
        let mut idx = from;
        while idx < to {
            let sector_no = idx / BLOCKS_PER_BITMAP_SECTOR;
            let sector = sb.bitmap_start as u64 + sector_no as u64;
            let mut bitmap = [0u8; SECTOR_SIZE];
            self.cache_mut()
                .read_sector(sector, &mut bitmap)
                .map_err(|_| VfsError::IoError)?;

            let sector_end = ((sector_no + 1) * BLOCKS_PER_BITMAP_SECTOR).min(to);
            while idx < sector_end {
                let bit_in_sector = idx % BLOCKS_PER_BITMAP_SECTOR;
                let byte = bit_in_sector / 8;
                if bitmap[byte] == 0xFF {
                    // Whole byte taken: step to the next byte boundary rather
                    // than testing eight bits that cannot be free.
                    idx += 8 - (bit_in_sector % 8);
                    continue;
                }
                let bit = bit_in_sector % 8;
                if bitmap[byte] & (1 << bit) == 0 {
                    bitmap[byte] |= 1 << bit;
                    self.cache_mut()
                        .write_sector(sector, &bitmap)
                        .map_err(|_| VfsError::IoError)?;
                    return Ok(Some(idx));
                }
                idx += 1;
            }
        }
        Ok(None)
    }

    /// Free a data block (1-based index).
    ///
    /// Returns `InvalidArgument` for a block the bitmap cannot describe. That
    /// only happens for a corrupt inode, and it used to index a 512-byte
    /// array with a byte offset of up to 4091 and take the kernel down.
    fn free_block(&self, block: u32) -> VfsResult<()> {
        if block == 0 {
            return Ok(()); // no-block sentinel
        }
        let idx = (block - 1) as usize;
        let sb = *self.sb();
        if idx >= sb.addressable_blocks() || idx >= sb.data_block_count as usize {
            return Err(VfsError::InvalidArgument);
        }
        let sector = sb.bitmap_start as u64 + (idx / BLOCKS_PER_BITMAP_SECTOR) as u64;
        let bit_in_sector = idx % BLOCKS_PER_BITMAP_SECTOR;
        let mut bitmap = [0u8; SECTOR_SIZE];
        self.cache_mut()
            .read_sector(sector, &mut bitmap)
            .map_err(|_| VfsError::IoError)?;
        bitmap[bit_in_sector / 8] &= !(1u8 << (bit_in_sector % 8));
        self.cache_mut()
            .write_sector(sector, &bitmap)
            .map_err(|_| VfsError::IoError)?;
        self.sb_mut().free_blocks += 1;

        // Reuse the hole before scanning past it, so a create/delete loop
        // stays in the same bitmap sector instead of marching down the disk.
        if (idx as u32) < self.alloc_hint.get() {
            self.alloc_hint.set(idx as u32);
        }
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
        self.cache_mut()
            .read_sector(self.data_sector(block), out)
            .map_err(|_| VfsError::IoError)
    }

    /// Write a data block.
    fn write_data_block(&self, block: u32, data: &[u8; SECTOR_SIZE]) -> VfsResult<()> {
        if block == 0 {
            return Err(VfsError::InvalidArgument);
        }
        self.cache_mut()
            .write_sector(self.data_sector(block), data)
            .map_err(|_| VfsError::IoError)
    }

    // --- Logical-to-physical block mapping ---------------------------------
    //
    // A file's blocks are addressed as 8 direct pointers in the inode, then
    // 128 more through a single indirect block, then 128*128 through a double
    // indirect block. Before this existed a file was 8 blocks - 4096 bytes -
    // and nothing bigger could live on persistent storage at all.

    /// Read pointer `slot` out of an indirect block.
    fn read_pointer(&self, block: u32, slot: usize) -> VfsResult<u32> {
        let mut sector = [0u8; SECTOR_SIZE];
        self.read_data_block(block, &mut sector)?;
        Ok(read_u32_le(&sector, slot * 4))
    }

    /// Write pointer `slot` into an indirect block.
    fn write_pointer(&self, block: u32, slot: usize, value: u32) -> VfsResult<()> {
        let mut sector = [0u8; SECTOR_SIZE];
        self.read_data_block(block, &mut sector)?;
        write_u32_le(&mut sector, slot * 4, value);
        self.write_data_block(block, &sector)
    }

    /// Allocate a block and zero it.
    ///
    /// Zeroing is what makes an indirect block safe to read: every slot must
    /// say "unallocated" rather than whatever the block's previous owner left
    /// behind, or the first read walks straight into someone else's data.
    fn alloc_zeroed_block(&self) -> VfsResult<u32> {
        let block = self.alloc_block()?;
        let zero = [0u8; SECTOR_SIZE];
        self.write_data_block(block, &zero)?;
        Ok(block)
    }

    /// Physical block backing logical block `idx`, or 0 if it is not mapped.
    /// Never allocates, so it takes the inode by shared reference.
    fn map_block(&self, inode: &DiskInode, idx: usize) -> VfsResult<u32> {
        if idx < DIRECT_BLOCKS {
            return Ok(inode.direct[idx]);
        }
        let idx = idx - DIRECT_BLOCKS;
        if idx < SINGLE_INDIRECT_BLOCKS {
            if inode.indirect == 0 {
                return Ok(0);
            }
            return self.read_pointer(inode.indirect, idx);
        }
        let idx = idx - SINGLE_INDIRECT_BLOCKS;
        if idx >= DOUBLE_INDIRECT_BLOCKS {
            return Err(VfsError::NoSpace);
        }
        if inode.double_indirect == 0 {
            return Ok(0);
        }
        let l1 = self.read_pointer(inode.double_indirect, idx / POINTERS_PER_BLOCK)?;
        if l1 == 0 {
            return Ok(0);
        }
        self.read_pointer(l1, idx % POINTERS_PER_BLOCK)
    }

    /// Physical block backing logical block `idx`, allocating it and every
    /// indirect block on the way there.
    ///
    /// Mutates `inode.indirect` / `inode.double_indirect`, so the caller must
    /// write the inode back. If a later step fails, the blocks allocated so
    /// far are leaked rather than half-linked - which is the damage class
    /// `check()` calls safe, and the one this ordering deliberately picks.
    fn map_block_alloc(&self, inode: &mut DiskInode, idx: usize) -> VfsResult<u32> {
        if idx >= MAX_FILE_BLOCKS {
            return Err(VfsError::NoSpace);
        }
        if idx < DIRECT_BLOCKS {
            if inode.direct[idx] == 0 {
                inode.direct[idx] = self.alloc_zeroed_block()?;
            }
            return Ok(inode.direct[idx]);
        }
        let idx = idx - DIRECT_BLOCKS;
        if idx < SINGLE_INDIRECT_BLOCKS {
            if inode.indirect == 0 {
                inode.indirect = self.alloc_zeroed_block()?;
            }
            let existing = self.read_pointer(inode.indirect, idx)?;
            if existing != 0 {
                return Ok(existing);
            }
            let block = self.alloc_zeroed_block()?;
            self.write_pointer(inode.indirect, idx, block)?;
            return Ok(block);
        }
        let idx = idx - SINGLE_INDIRECT_BLOCKS;
        if inode.double_indirect == 0 {
            inode.double_indirect = self.alloc_zeroed_block()?;
        }
        let l1_slot = idx / POINTERS_PER_BLOCK;
        let mut l1 = self.read_pointer(inode.double_indirect, l1_slot)?;
        if l1 == 0 {
            l1 = self.alloc_zeroed_block()?;
            self.write_pointer(inode.double_indirect, l1_slot, l1)?;
        }
        let slot = idx % POINTERS_PER_BLOCK;
        let existing = self.read_pointer(l1, slot)?;
        if existing != 0 {
            return Ok(existing);
        }
        let block = self.alloc_zeroed_block()?;
        self.write_pointer(l1, slot, block)?;
        Ok(block)
    }

    /// Every block an inode owns: its data blocks, plus the indirect blocks
    /// that hold their pointers. The indirect blocks come out of the same
    /// bitmap, so omitting them would make `check()` report each one leaked.
    fn owned_blocks(&self, inode: &DiskInode) -> Vec<u32> {
        let mut out = Vec::new();
        for i in 0..DIRECT_BLOCKS {
            if inode.direct[i] != 0 {
                out.push(inode.direct[i]);
            }
        }
        if inode.indirect != 0 {
            out.push(inode.indirect);
            for slot in 0..POINTERS_PER_BLOCK {
                // An unreadable pointer block means this inode's block list is
                // already broken; report what can be seen rather than failing
                // the whole check.
                if let Ok(p) = self.read_pointer(inode.indirect, slot) {
                    if p != 0 {
                        out.push(p);
                    }
                }
            }
        }
        if inode.double_indirect != 0 {
            out.push(inode.double_indirect);
            for slot in 0..POINTERS_PER_BLOCK {
                let l1 = match self.read_pointer(inode.double_indirect, slot) {
                    Ok(b) if b != 0 => b,
                    _ => continue,
                };
                out.push(l1);
                for inner in 0..POINTERS_PER_BLOCK {
                    if let Ok(p) = self.read_pointer(l1, inner) {
                        if p != 0 {
                            out.push(p);
                        }
                    }
                }
            }
        }
        out
    }

    /// Free every block an inode owns and clear its block map.
    ///
    /// Errors from `free_block` are swallowed on purpose: this runs while
    /// removing a file, and a pointer the bitmap cannot describe - a corrupt
    /// inode - must not make the inode un-freeable. Being able to `rm` a
    /// damaged file is exactly what the fsck warning tells the user to do.
    fn free_block_map(&self, inode: &mut DiskInode) {
        for i in 0..DIRECT_BLOCKS {
            let _ = self.free_block(inode.direct[i]);
            inode.direct[i] = 0;
        }
        if inode.indirect != 0 {
            self.free_pointer_block(inode.indirect);
            inode.indirect = 0;
        }
        if inode.double_indirect != 0 {
            for slot in 0..POINTERS_PER_BLOCK {
                if let Ok(l1) = self.read_pointer(inode.double_indirect, slot) {
                    if l1 != 0 {
                        self.free_pointer_block(l1);
                    }
                }
            }
            let _ = self.free_block(inode.double_indirect);
            inode.double_indirect = 0;
        }
    }

    /// Free every block an indirect block points at, then the block itself.
    fn free_pointer_block(&self, block: u32) {
        for slot in 0..POINTERS_PER_BLOCK {
            if let Ok(p) = self.read_pointer(block, slot) {
                let _ = self.free_block(p);
            }
        }
        let _ = self.free_block(block);
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

    /// Create a file or directory, as one journalled transaction.
    ///
    /// The work spans an inode, its parent's directory block, the allocation
    /// bitmap and the superblock. A crash between any two of those used to
    /// leave a directory entry pointing at an inode that was never written,
    /// or a block marked used by nobody.
    fn create_entry(&self, parent_ino: u32, name: &str, itype: u8, mode: u16) -> VfsResult<u32> {
        self.transaction(|| self.create_entry_inner(parent_ino, name, itype, mode))
    }

    fn create_entry_inner(
        &self,
        parent_ino: u32,
        name: &str,
        itype: u8,
        mode: u16,
    ) -> VfsResult<u32> {
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
            indirect: 0,
            double_indirect: 0,
            _reserved: [0u8; 128 - 64],
        };
        self.write_inode(new_ino, &new_inode)?;

        // Add directory entry to parent.
        self.dir_add_entry(parent_ino, new_ino, name)?;
        self.flush_sb()?;
        self.cache_mut().flush().map_err(|_| VfsError::IoError)?;
        Ok(new_ino)
    }

    /// Remove a name. The inode itself is freed only when its last name goes.
    ///
    /// This is what makes hard links work: `link()` adds a second directory
    /// entry for one inode and bumps `nlink`, so unlinking either name must
    /// drop the count rather than free blocks the other name still points at.
    /// A file with `nlink == 1` behaves exactly as before.
    pub fn unlink(&self, parent_ino: u32, name: &str) -> VfsResult<()> {
        self.transaction(|| self.unlink_inner(parent_ino, name))
    }

    fn unlink_inner(&self, parent_ino: u32, name: &str) -> VfsResult<()> {
        let parent = self.read_inode(parent_ino)?;
        if parent.itype != ITYPE_DIR {
            return Err(VfsError::NotADirectory);
        }
        let child_ino = self.dir_lookup(&parent, name)?.ok_or(VfsError::NotFound)?;
        let mut child = self.read_inode(child_ino)?;

        // If directory, must be empty.
        if child.itype == ITYPE_DIR && child.dir_entry_count > 0 {
            return Err(VfsError::InvalidArgument); // ENOTEMPTY
        }

        // Remove dir entry from parent.
        self.dir_remove_entry(parent_ino, name)?;

        // Directories are never hard-linked, so their nlink bookkeeping (2 for
        // "." plus the parent's entry) says nothing about liveness — drop them
        // outright, as before. Files fall through to refcounting.
        if child.itype == ITYPE_DIR || child.nlink <= 1 {
            self.free_inode(child_ino)?;
        } else {
            child.nlink -= 1;
            self.write_inode(child_ino, &child)?;
        }

        self.flush_sb()?;
        self.cache_mut().flush().map_err(|_| VfsError::IoError)?;
        Ok(())
    }

    /// Create `name` in `parent_ino` as another name for the existing inode
    /// `target_ino`.
    ///
    /// Refuses to link a directory: with no `..` fixups and no cycle detection
    /// in the VFS, a directory hard link turns the tree into a graph that
    /// `du`, `find` and the mount logic would all walk forever.
    pub fn link(&self, parent_ino: u32, name: &str, target_ino: u32) -> VfsResult<()> {
        self.transaction(|| self.link_inner(parent_ino, name, target_ino))
    }

    fn link_inner(&self, parent_ino: u32, name: &str, target_ino: u32) -> VfsResult<()> {
        if name.is_empty() || name.len() > MAX_NAME_LEN - 4 {
            return Err(VfsError::InvalidArgument);
        }

        let parent = self.read_inode(parent_ino)?;
        if parent.itype != ITYPE_DIR {
            return Err(VfsError::NotADirectory);
        }
        if self.dir_lookup(&parent, name)?.is_some() {
            return Err(VfsError::AlreadyExists);
        }

        let mut target = self.read_inode(target_ino)?;
        if target.itype == ITYPE_DIR {
            return Err(VfsError::IsADirectory);
        }
        // u16 counter: refuse rather than wrap into a count that would free
        // live data on the next unlink.
        if target.nlink == u16::MAX {
            return Err(VfsError::InvalidArgument);
        }

        self.dir_add_entry(parent_ino, target_ino, name)?;
        target.nlink += 1;
        self.write_inode(target_ino, &target)?;

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
            // Past the addressable block map. Only a corrupt `size` can get
            // here, since no write can produce a file that long; stop rather
            // than fail, so the readable prefix still comes back.
            let Ok(block) = self.map_block(&inode, block_idx) else {
                break;
            };
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
        let blocks_needed = end.div_ceil(SECTOR_SIZE);
        if blocks_needed > MAX_FILE_BLOCKS {
            return Err(VfsError::NoSpace);
        }

        // Blocks below the current size are already allocated - racfs has no
        // holes, and this loop is what maintains that. Starting from the last
        // block the file already has, rather than from 0, is what keeps a
        // sequential write linear instead of quadratic in the file's length;
        // at 8 blocks that distinction did not exist, at 16520 it does.
        //
        // Allocation is *metadata* - it touches the bitmap, the superblock's
        // free count, and the inode's block map - so it runs as a
        // transaction like any other metadata operation. It did not at
        // first, and the roadmap's list ("create / unlink / rename /
        // set_metadata") is why: extending a file mutates the same
        // structures, and leaving it out meant two real holes. The cache
        // flushes its sectors in whatever order it likes, so a crash could
        // land the inode's new block pointers before the bitmap marked those
        // blocks used - an inode referencing blocks the allocator will hand
        // out again, the damage class fsck calls dangerous. And nothing on
        // this path wrote the superblock at all, so every boot whose last
        // operation was an extending write left `free_blocks` stale on disk
        // - the sb_drift the journal-replay test kept finding in images
        // believed clean.
        let already = inode.size as usize / SECTOR_SIZE;
        if blocks_needed > already {
            self.transaction(|| {
                for i in already..blocks_needed {
                    self.map_block_alloc(&mut inode, i)?;
                }
                // The block map is durable before any data lands in it. A
                // crash after this commit leaves zeroed blocks the file owns
                // - visible garbage at worst, never someone else's sectors.
                self.write_inode(ino, &inode)
            })?;
        }

        // The data itself, outside any transaction. Journaling it would
        // double every write and cap file size at the journal's slot count;
        // losing the tail of a file that was mid-write is the expected
        // outcome of a crash, in a way losing the metadata that names it is
        // not.
        let mut done = 0usize;
        let mut pos = offset as usize;
        while done < data.len() {
            let block_idx = pos / SECTOR_SIZE;
            let block_off = pos % SECTOR_SIZE;
            let block = self.map_block(&inode, block_idx)?;
            let mut sector = [0u8; SECTOR_SIZE];
            self.read_data_block(block, &mut sector)?;
            let chunk = (SECTOR_SIZE - block_off).min(data.len() - done);
            sector[block_off..block_off + chunk].copy_from_slice(&data[done..done + chunk]);
            self.write_data_block(block, &sector)?;
            done += chunk;
            pos += chunk;
        }

        // Size last, and only after the data it describes is in the cache:
        // a crash before this commit reads back the old length, never
        // uninitialised bytes.
        if end as u32 > inode.size {
            inode.size = end as u32;
            self.transaction(|| self.write_inode(ino, &inode))?;
        } else {
            self.write_inode(ino, &inode)?;
        }
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
        let block = self.map_block(inode, block_idx)?;
        if block == 0 {
            return Err(VfsError::IoError);
        }
        let mut sector = [0u8; SECTOR_SIZE];
        self.read_data_block(block, &mut sector)?;
        let off = entry_in_block * DIR_ENTRY_SIZE;
        Ok(direntry_from_bytes(&sector[off..off + DIR_ENTRY_SIZE]))
    }

    /// Add a directory entry to a directory inode.
    fn dir_add_entry(&self, dir_ino: u32, child_ino: u32, name: &str) -> VfsResult<()> {
        let mut dir = self.read_inode(dir_ino)?;
        let idx = dir.dir_entry_count;
        let block_idx = idx as usize / DIR_ENTRIES_PER_BLOCK;
        let entry_in_block = idx as usize % DIR_ENTRIES_PER_BLOCK;

        // Directories share the file block map, so a directory is no longer
        // capped at 8 blocks - 64 entries - either. `/home` and `/var/lib`
        // hitting that cap was a v0.3 blocker nobody had run into yet only
        // because nothing persistent had that many names in it.
        let block = self.map_block_alloc(&mut dir, block_idx)?;

        // Build entry. name_len records what actually fits, not what was
        // asked for: `name.len() as u8` would wrap past 255 and, for anything
        // over 56 bytes, claim more than the entry stores.
        let mut de = DiskDirEntry {
            ino: child_ino,
            name_len: 0,
            _pad: [0; 3],
            name: [0u8; MAX_NAME_LEN - 4],
        };
        let copy_len = name.len().min(de.name.len());
        de.name[..copy_len].copy_from_slice(&name.as_bytes()[..copy_len]);
        de.name_len = copy_len as u8;

        // Write entry into data block.
        let mut sector = [0u8; SECTOR_SIZE];
        self.read_data_block(block, &mut sector)?;
        let off = entry_in_block * DIR_ENTRY_SIZE;
        let de_bytes = direntry_to_bytes(&de);
        sector[off..off + DIR_ENTRY_SIZE].copy_from_slice(&de_bytes);
        self.write_data_block(block, &sector)?;

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
            let block = self.map_block(&dir, block_idx)?;
            let mut sector = [0u8; SECTOR_SIZE];
            self.read_data_block(block, &mut sector)?;
            let off = entry_in_block * DIR_ENTRY_SIZE;
            let bytes = direntry_to_bytes(&last);
            sector[off..off + DIR_ENTRY_SIZE].copy_from_slice(&bytes);
            self.write_data_block(block, &sector)?;
        }

        dir.dir_entry_count -= 1;
        self.write_inode(dir_ino, &dir)?;
        Ok(())
    }

    /// Lookup a path from the disk root. Returns inode number.
    pub fn lookup_path(&self, path: &str) -> VfsResult<u32> {
        self.lookup_path_from(0, path)
    }

    /// Walk `path` starting at `root` instead of at the disk root.
    ///
    /// This is what a subtree mount is: the same filesystem entered at a
    /// different inode. Without it, `/home` mounted at the disk's `home`
    /// directory would resolve `mkdir /home/x` against the disk root and
    /// create `x` in entirely the wrong place.
    pub fn lookup_path_from(&self, root: u32, path: &str) -> VfsResult<u32> {
        let mut current: u32 = root;
        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            let inode = self.read_inode(current)?;
            if inode.itype != ITYPE_DIR {
                return Err(VfsError::NotADirectory);
            }
            current = self
                .dir_lookup(&inode, component)?
                .ok_or(VfsError::NotFound)?;
        }
        Ok(current)
    }

    /// Split path into (parent_ino, leaf_name), resolved from the disk root.
    pub fn split_parent_leaf<'a>(&self, path: &'a str) -> VfsResult<(u32, &'a str)> {
        self.split_parent_leaf_from(0, path)
    }

    /// Split path into (parent_ino, leaf_name), resolved from `root`.
    /// The subtree-mount counterpart of `lookup_path_from`.
    pub fn split_parent_leaf_from<'a>(
        &self,
        root: u32,
        path: &'a str,
    ) -> VfsResult<(u32, &'a str)> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Err(VfsError::InvalidArgument);
        }
        let leaf = parts[parts.len() - 1];
        let mut parent_ino: u32 = root;
        for &part in &parts[..parts.len() - 1] {
            let inode = self.read_inode(parent_ino)?;
            if inode.itype != ITYPE_DIR {
                return Err(VfsError::NotADirectory);
            }
            parent_ino = self.dir_lookup(&inode, part)?.ok_or(VfsError::NotFound)?;
        }
        Ok((parent_ino, leaf))
    }

    /// Create `path` and any missing parents; return the leaf's inode.
    ///
    /// Existing directories are reused. A *file* in the way is an error
    /// rather than something to replace silently: at boot this lays out
    /// `/home`, `/etc` and `/var/lib/rpkg` on a disk whose contents came
    /// from an earlier run, and quietly deleting a user's file to make room
    /// for a mount point would be the wrong way to find that out.
    pub fn ensure_dir_path(&self, path: &str) -> VfsResult<u32> {
        let mut current: u32 = 0;
        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            let parent = self.read_inode(current)?;
            if parent.itype != ITYPE_DIR {
                return Err(VfsError::NotADirectory);
            }
            current = match self.dir_lookup(&parent, component)? {
                Some(ino) => {
                    if self.read_inode(ino)?.itype != ITYPE_DIR {
                        return Err(VfsError::NotADirectory);
                    }
                    ino
                }
                None => self.create_dir(current, component)?,
            };
        }
        Ok(current)
    }

    /// How many entries a directory holds. Used to tell "never populated"
    /// from "populated and then emptied", which decides whether the boot
    /// seeds a persistent /etc from the initramfs defaults.
    pub fn dir_entry_count(&self, ino: u32) -> VfsResult<u32> {
        let di = self.read_inode(ino)?;
        if di.itype != ITYPE_DIR {
            return Err(VfsError::NotADirectory);
        }
        Ok(di.dir_entry_count)
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
            // Skip slots that don't describe a usable entry rather than
            // listing them or failing the whole directory. A damaged image
            // (torn write, stale slot) otherwise shows up as blank `ls` rows,
            // or makes one bad entry hide every good one behind a `?`.
            if de.name_len == 0 {
                continue;
            }
            let child = match self.read_inode(de.ino) {
                Ok(c) => c,
                Err(_) => continue,
            };
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
        self.transaction(|| self.set_inode_metadata_inner(ino, meta))
    }

    fn set_inode_metadata_inner(&self, ino: u32, meta: &InodeMetadata) -> VfsResult<()> {
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
        self.fs
            .dir_lookup(&di, name)?
            .ok_or(VfsError::NotFound)
            .map(|i| i as InodeNum)
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
    /// Inode this mount presents as `/`. Zero for a whole-disk mount; a
    /// directory's inode for a subtree mount, which is how `/home`, `/etc`
    /// and `/var/lib/rpkg` all live on the one persistent disk without
    /// needing a block device each.
    root_ino: u32,
}

impl RacfsFilesystem {
    pub fn new(racfs: Arc<Racfs>) -> Arc<Self> {
        Arc::new(RacfsFilesystem {
            inner: racfs,
            root_ino: 0,
        })
    }

    /// Mount the directory `root_ino` of `racfs` as a filesystem of its own.
    pub fn new_subtree(racfs: Arc<Racfs>, root_ino: u32) -> Arc<Self> {
        Arc::new(RacfsFilesystem {
            inner: racfs,
            root_ino,
        })
    }

    /// The inode this mount treats as its root. Path resolution for writes
    /// must start here, not at inode 0.
    pub fn root_ino(&self) -> u32 {
        self.root_ino
    }

    /// Access the concrete Racfs backing this mount. Used by syscall handlers
    /// to route create/mkdir/unlink to the right disk, not a global singleton.
    pub fn inner(&self) -> Arc<Racfs> {
        self.inner.clone()
    }
}

impl Filesystem for RacfsFilesystem {
    fn root_inode(&self) -> Arc<dyn InodeOps> {
        Arc::new(RacfsInode {
            ino: self.root_ino,
            fs: self.inner.clone(),
        })
    }

    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>> {
        // Validate inode exists.
        let di = self.inner.read_inode(ino as u32)?;
        if di.itype == ITYPE_FREE {
            return Err(VfsError::NotFound);
        }
        Ok(Arc::new(RacfsInode {
            ino: ino as u32,
            fs: self.inner.clone(),
        }))
    }

    fn name(&self) -> &str {
        "racfs"
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

// ─── Global instance ────────────────────────────────────────────────────────

static mut RACFS_INSTANCE: Option<Arc<Racfs>> = None;

/// Initialize racfs on the first registered block device (ram0).
///
/// # Safety
/// Must be called once during kernel init after block devices are ready.
pub unsafe fn init() -> Arc<Racfs> {
    let dev = crate::drivers::block::find("ram0").expect("racfs: no ram0 block device found");
    let racfs = Racfs::format_and_new(dev).expect("racfs: format failed");
    let inst = &mut *core::ptr::addr_of_mut!(RACFS_INSTANCE);
    *inst = Some(racfs.clone());
    crate::serial::serial_println!(
        "[  0.000360] RACORE: racfs initialized (block-device-backed on ram0)"
    );
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
    // (what the previous boot wrote, what this boot writes), or None if the
    // counter could not be established at all.
    let mut generation: Option<(u32, u32)> = None;
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
                label,
                next,
                value,
            );
            generation = Some((value, next));
        }
        Err(_) => match fs.create_file(0, NAME) {
            Ok(ino) => {
                let _ = fs.write_file(ino, 0, b"1");
                crate::serial::serial_println!(
                    "[ RACFS {} ] created boot-counter = 1 (first boot)",
                    label
                );
                generation = Some((0, 1));
            }
            Err(e) => crate::serial::serial_println!(
                "[ RACFS {} ] create boot-counter failed: {:?}",
                label,
                e
            ),
        },
    }

    if let Some((previous, current)) = generation {
        big_probe(fs, label, previous, current);
    }
}

/// The other half of the persistence probe: a file past the eight direct
/// block pointers, so a reboot proves the *indirect* block map survives and
/// not merely an inode's first block.
///
/// `boot-counter` is one byte long, so it only ever exercised `direct[0]`. It
/// would have kept passing on a filesystem whose indirect blocks were written
/// to the wrong place entirely, which is precisely the code v0.3 §3.2 added.
///
/// 8192 bytes is 16 blocks: 8 direct and 8 reached through the single
/// indirect. The last 16 bytes carry the number of the boot that wrote them,
/// and reading *those* is the point — they are the part only the indirect map
/// can reach.
fn big_probe(fs: &Racfs, label: &str, previous: u32, current: u32) {
    const NAME: &str = "big-probe";
    const LEN: usize = 8192;
    const TAIL: usize = 16;

    // "#" + 14 digits + "\n" is exactly TAIL bytes, so the marker always lands
    // at the same offset however large the counter grows.
    fn marker(n: u32) -> [u8; TAIL] {
        let mut out = [b'0'; TAIL];
        out[0] = b'#';
        out[TAIL - 1] = b'\n';
        let mut v = n;
        let mut i = TAIL - 2;
        loop {
            out[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 || i == 1 {
                break;
            }
            i -= 1;
        }
        out
    }

    fn parse(buf: &[u8]) -> Option<u32> {
        if buf.len() != TAIL || buf[0] != b'#' {
            return None;
        }
        let mut v: u32 = 0;
        for &b in &buf[1..TAIL - 1] {
            if !b.is_ascii_digit() {
                return None;
            }
            v = v.wrapping_mul(10).wrapping_add((b - b'0') as u32);
        }
        Some(v)
    }

    let existing = fs.lookup_path(NAME).ok();

    if let Some(ino) = existing {
        let mut tail = [0u8; TAIL];
        let n = fs
            .read_file(ino, (LEN - TAIL) as u64, &mut tail)
            .unwrap_or(0);
        match parse(&tail[..n]) {
            Some(found) if found == previous => crate::serial::serial_println!(
                "[ RACFS {} ] big-probe tail = {} (expected {}, indirect blocks survived reboot)",
                label,
                found,
                previous
            ),
            Some(found) => crate::serial::serial_println!(
                "[ RACFS {} ] big-probe tail MISMATCH: got {}, expected {}",
                label,
                found,
                previous
            ),
            None => crate::serial::serial_println!(
                "[ RACFS {} ] big-probe tail unreadable ({} bytes at offset {})",
                label,
                n,
                LEN - TAIL
            ),
        }
    }

    let ino = match existing {
        Some(ino) => ino,
        None => match fs.create_file(0, NAME) {
            Ok(ino) => {
                crate::serial::serial_println!(
                    "[ RACFS {} ] created big-probe ({} B, past the direct blocks)",
                    label,
                    LEN
                );
                ino
            }
            Err(e) => {
                crate::serial::serial_println!(
                    "[ RACFS {} ] create big-probe failed: {:?}",
                    label,
                    e
                );
                return;
            }
        },
    };

    let mut body = alloc::vec![b'.'; LEN];
    body[LEN - TAIL..].copy_from_slice(&marker(current));
    if let Err(e) = fs.write_file(ino, 0, &body) {
        crate::serial::serial_println!("[ RACFS {} ] big-probe write failed: {:?}", label, e);
    }
}
