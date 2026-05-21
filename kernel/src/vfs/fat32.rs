// RaCore — FAT32 Filesystem Implementation (ADR-009)
//
// Phase F.4 adds full write support on top of the original Phase F.1 reader:
// - cluster allocation / chain release (preserves upper 4 bits of FAT entry)
// - write_chain extending the chain as needed
// - 8.3 directory entry create / unlink / mkdir
// - in-kernel format_fat32 helper (also the foundation for `mkfs.fat32`)
//
// Intentional limitations (sufficient for current tests, room to grow later):
// - 8.3 names only (no LFN read/write — long names are simply skipped on read)
// - single FAT updated on writes if `fat_count > 1` we mirror to all copies
// - no FSInfo free-cluster hint update (next alloc rescans the FAT)

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::string::String;
use crate::vfs::inode::{InodeOps, VfsResult, VfsError, FileType, FileMode, InodeMetadata, DirEntry, InodeNum};
use crate::vfs::mount::Filesystem;
use crate::drivers::block::{BlockDevice, SECTOR_SIZE};
use crate::sync::SpinLock;
use core::mem::size_of;

const FAT_ENTRY_MASK: u32      = 0x0FFF_FFFF;
const FAT_ENTRY_FREE: u32      = 0x0000_0000;
const FAT_ENTRY_EOC_MIN: u32   = 0x0FFF_FFF8;
const FAT_ENTRY_EOC: u32       = 0x0FFF_FFFF;
const FAT_ENTRY_BAD: u32       = 0x0FFF_FFF7;

const ATTR_READ_ONLY: u8 = 0x01;
const ATTR_HIDDEN:    u8 = 0x02;
const ATTR_SYSTEM:    u8 = 0x04;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_ARCHIVE:   u8 = 0x20;
const ATTR_LFN:       u8 = ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID;

const DIR_ENTRY_SIZE: usize = 32;
const DIR_END:        u8    = 0x00;
const DIR_FREE:       u8    = 0xE5;

/// Maximum cluster we can track when scanning during alloc / FAT bookkeeping.
/// Keeps a single linear scan reasonable for small images (< few hundred MiB).
const MAX_SCAN_CLUSTERS: u32 = 1_048_576;

/// BIOS Parameter Block (FAT32)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BpbFat32 {
    pub jmp: [u8; 3],
    pub oem_name: [u8; 8],
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub fat_count: u8,
    pub root_entry_count: u16,
    pub total_sectors_16: u16,
    pub media_type: u8,
    pub sectors_per_fat_16: u16,
    pub sectors_per_track: u16,
    pub head_count: u16,
    pub hidden_sectors: u32,
    pub total_sectors_32: u32,
    pub sectors_per_fat_32: u32,
    pub flags: u16,
    pub version: u16,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
    pub backup_boot_sector: u16,
    pub reserved_bytes: [u8; 12],
    pub drive_number: u8,
    pub nt_reserved: u8,
    pub signature: u8,
    pub serial_number: u32,
    pub vol_label: [u8; 11],
    pub sys_id: [u8; 8],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct FatDirEntry {
    pub name: [u8; 8],
    pub ext: [u8; 3],
    pub attr: u8,
    pub nt_reserved: u8,
    pub creation_time_tenths: u8,
    pub creation_time: u16,
    pub creation_date: u16,
    pub last_access_date: u16,
    pub first_cluster_high: u16,
    pub write_time: u16,
    pub write_date: u16,
    pub first_cluster_low: u16,
    pub size: u32,
}

impl FatDirEntry {
    pub fn is_end(&self) -> bool     { self.name[0] == DIR_END }
    pub fn is_deleted(&self) -> bool { self.name[0] == DIR_FREE }
    pub fn is_lfn(&self) -> bool     { (self.attr & ATTR_LFN) == ATTR_LFN }
    pub fn is_volume(&self) -> bool  { self.attr & ATTR_VOLUME_ID != 0 }
    pub fn is_dir(&self) -> bool     { self.attr & ATTR_DIRECTORY != 0 }

    pub fn get_name(&self) -> String {
        let mut name = String::new();
        for &b in &self.name {
            if b == b' ' { break; }
            name.push(b as char);
        }
        if self.ext[0] != b' ' {
            name.push('.');
            for &b in &self.ext {
                if b == b' ' { break; }
                name.push(b as char);
            }
        }
        name
    }

    pub fn get_cluster(&self) -> u32 {
        ((self.first_cluster_high as u32) << 16) | (self.first_cluster_low as u32)
    }

    pub fn set_cluster(&mut self, cluster: u32) {
        self.first_cluster_low  = (cluster & 0xFFFF) as u16;
        self.first_cluster_high = ((cluster >> 16) & 0xFFFF) as u16;
    }
}

/// Encode a UTF-8 name into 8.3 (uppercase, space-padded). Returns
/// Err(InvalidArgument) if the name does not fit or contains illegal chars.
fn encode_short_name(name: &str) -> VfsResult<([u8; 8], [u8; 3])> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(VfsError::InvalidArgument);
    }
    // Split on the LAST '.' (extension).
    let (base, ext) = match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    };
    if base.len() > 8 || ext.len() > 3 {
        return Err(VfsError::InvalidArgument);
    }

    let mut name_bytes = [b' '; 8];
    let mut ext_bytes  = [b' '; 3];

    for (i, ch) in base.bytes().enumerate() {
        if !is_legal_short_char(ch) { return Err(VfsError::InvalidArgument); }
        name_bytes[i] = ascii_upper(ch);
    }
    for (i, ch) in ext.bytes().enumerate() {
        if !is_legal_short_char(ch) { return Err(VfsError::InvalidArgument); }
        ext_bytes[i] = ascii_upper(ch);
    }
    // The very first byte can never legally be 0x00 or 0xE5 on a real entry —
    // 0xE5 is allowed as a literal byte only via the 0x05 alias which we don't
    // emit.
    if name_bytes[0] == 0x00 || name_bytes[0] == 0xE5 {
        return Err(VfsError::InvalidArgument);
    }
    Ok((name_bytes, ext_bytes))
}

fn is_legal_short_char(c: u8) -> bool {
    matches!(c,
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
        | b'!' | b'#' | b'$' | b'%' | b'&' | b'\''
        | b'(' | b')' | b'-' | b'@' | b'^' | b'_'
        | b'`' | b'{' | b'}' | b'~'
    )
}

fn ascii_upper(c: u8) -> u8 {
    if (b'a'..=b'z').contains(&c) { c - 32 } else { c }
}

pub struct Fat32Fs {
    pub device: Arc<dyn BlockDevice>,
    pub bpb: BpbFat32,
    pub fat_offset: u64,
    pub data_offset: u64,
    /// Bytes per cluster (cached).
    pub cluster_size: u64,
    /// Total number of clusters in the data area (cached).
    total_clusters: u32,
    metadata_cache: SpinLock<Vec<(InodeNum, InodeMetadata)>>,
    /// Cached size of every regular file we've handed out. FAT does not store
    /// size on the inode itself (it lives in the parent dir entry), so we
    /// have to remember it when an InodeOps::write extends the file.
    sizes: SpinLock<Vec<(InodeNum, u64)>>,
    /// (parent_dir_cluster, child_first_cluster) — lets us find the dir entry
    /// to rewrite when a file grows so the on-disk size stays consistent.
    parent_map: SpinLock<Vec<(u32, u32)>>,
}

impl Fat32Fs {
    /// Mount an existing FAT32 by reading its BPB.
    pub fn new(device: Arc<dyn BlockDevice>) -> VfsResult<Arc<Self>> {
        let mut sector = [0u8; SECTOR_SIZE];
        device.read_sector(0, &mut sector).map_err(|_| VfsError::IoError)?;

        let bpb: BpbFat32 = unsafe { core::ptr::read_unaligned(sector.as_ptr() as *const BpbFat32) };

        if bpb.signature != 0x28 && bpb.signature != 0x29 {
            return Err(VfsError::InvalidArgument);
        }
        if bpb.bytes_per_sector as usize != SECTOR_SIZE {
            return Err(VfsError::InvalidArgument);
        }
        if bpb.sectors_per_fat_32 == 0 || bpb.sectors_per_cluster == 0 {
            return Err(VfsError::InvalidArgument);
        }

        let fat_offset = bpb.reserved_sectors as u64;
        let data_offset = fat_offset + (bpb.fat_count as u64 * bpb.sectors_per_fat_32 as u64);
        let cluster_size = bpb.sectors_per_cluster as u64 * SECTOR_SIZE as u64;
        let total_sectors = if bpb.total_sectors_32 != 0 {
            bpb.total_sectors_32 as u64
        } else {
            bpb.total_sectors_16 as u64
        };
        let data_sectors = total_sectors.saturating_sub(data_offset);
        let total_clusters = (data_sectors / bpb.sectors_per_cluster as u64) as u32;

        let fs = Arc::new(Fat32Fs {
            device,
            bpb,
            fat_offset,
            data_offset,
            cluster_size,
            total_clusters,
            metadata_cache: SpinLock::new(Vec::new()),
            sizes: SpinLock::new(Vec::new()),
            parent_map: SpinLock::new(Vec::new()),
        });

        let root_ino = fs.bpb.root_cluster as InodeNum;
        let mut root_meta = InodeMetadata::new(root_ino, FileType::Directory);
        root_meta.mode = FileMode::new(0o755);
        fs.cache_metadata(root_ino, root_meta);

        Ok(fs)
    }

    fn cache_metadata(&self, ino: InodeNum, meta: InodeMetadata) {
        let mut cache = self.metadata_cache.lock();
        if let Some((_, existing)) = cache.iter_mut().find(|(i, _)| *i == ino) {
            *existing = meta;
            return;
        }
        cache.push((ino, meta));
    }

    fn get_cached_metadata(&self, ino: InodeNum) -> Option<InodeMetadata> {
        let cache = self.metadata_cache.lock();
        cache.iter().find(|(i, _)| *i == ino).map(|(_, m)| m.clone())
    }

    fn remember_parent(&self, parent_cluster: u32, child_cluster: u32) {
        let mut map = self.parent_map.lock();
        if let Some((_, p)) = map.iter_mut().find(|(c, _)| *c == child_cluster) {
            *p = parent_cluster;
            return;
        }
        map.push((parent_cluster, child_cluster));
    }

    fn get_parent(&self, child_cluster: u32) -> Option<u32> {
        let map = self.parent_map.lock();
        map.iter().find(|(_, c)| *c == child_cluster).map(|(p, _)| *p)
    }

    fn cached_size(&self, ino: InodeNum) -> Option<u64> {
        let s = self.sizes.lock();
        s.iter().find(|(i, _)| *i == ino).map(|(_, sz)| *sz)
    }

    fn set_cached_size(&self, ino: InodeNum, size: u64) {
        let mut s = self.sizes.lock();
        if let Some((_, sz)) = s.iter_mut().find(|(i, _)| *i == ino) {
            *sz = size;
            return;
        }
        s.push((ino, size));
    }

    // ─── FAT entry I/O ─────────────────────────────────────────────────────

    pub fn next_cluster(&self, cluster: u32) -> VfsResult<u32> {
        let fat_sector = self.fat_offset + (cluster as u64 * 4 / SECTOR_SIZE as u64);
        let offset = (cluster as usize * 4) % SECTOR_SIZE;
        let mut sector_data = [0u8; SECTOR_SIZE];
        self.device.read_sector(fat_sector, &mut sector_data).map_err(|_| VfsError::IoError)?;
        let entry = unsafe { core::ptr::read_unaligned(sector_data.as_ptr().add(offset) as *const u32) };
        Ok(entry & FAT_ENTRY_MASK)
    }

    /// Write a FAT entry, preserving the top 4 bits of the existing dword
    /// (per FAT32 spec) and mirroring to every FAT copy if fat_count > 1.
    fn write_fat_entry(&self, cluster: u32, value: u32) -> VfsResult<()> {
        let value = value & FAT_ENTRY_MASK;
        let fat_sectors = self.bpb.sectors_per_fat_32 as u64;
        let sec_within = cluster as u64 * 4 / SECTOR_SIZE as u64;
        let offset = (cluster as usize * 4) % SECTOR_SIZE;

        for fat_idx in 0..self.bpb.fat_count as u64 {
            let lba = self.fat_offset + fat_idx * fat_sectors + sec_within;
            let mut buf = [0u8; SECTOR_SIZE];
            self.device.read_sector(lba, &mut buf).map_err(|_| VfsError::IoError)?;
            let old = unsafe { core::ptr::read_unaligned(buf.as_ptr().add(offset) as *const u32) };
            let new = (old & 0xF000_0000) | value;
            unsafe { core::ptr::write_unaligned(buf.as_mut_ptr().add(offset) as *mut u32, new); }
            self.device.write_sector(lba, &buf).map_err(|_| VfsError::IoError)?;
        }
        Ok(())
    }

    /// Find the first free cluster (>= 2). Returns the cluster number; caller
    /// is responsible for writing whatever chain marker they need.
    fn alloc_cluster(&self) -> VfsResult<u32> {
        let upper = self.total_clusters.saturating_add(2).min(MAX_SCAN_CLUSTERS);
        for cluster in 2..upper {
            let entry = self.next_cluster(cluster)?;
            if entry == FAT_ENTRY_FREE {
                // Mark as end-of-chain by default.
                self.write_fat_entry(cluster, FAT_ENTRY_EOC)?;
                // Zero the cluster contents so callers don't see stale data.
                let zero = [0u8; SECTOR_SIZE];
                let lba = self.cluster_to_lba(cluster);
                for s in 0..self.bpb.sectors_per_cluster as u64 {
                    self.device.write_sector(lba + s, &zero).map_err(|_| VfsError::IoError)?;
                }
                return Ok(cluster);
            }
        }
        Err(VfsError::NoSpace)
    }

    /// Free an entire cluster chain starting at `start` (no-op if start < 2).
    fn free_chain(&self, start: u32) -> VfsResult<()> {
        if start < 2 || start >= FAT_ENTRY_EOC_MIN {
            return Ok(());
        }
        let mut cur = start;
        for _ in 0..MAX_SCAN_CLUSTERS {
            if cur < 2 || cur >= FAT_ENTRY_EOC_MIN || cur == FAT_ENTRY_BAD {
                break;
            }
            let next = self.next_cluster(cur)?;
            self.write_fat_entry(cur, FAT_ENTRY_FREE)?;
            cur = next;
        }
        Ok(())
    }

    fn cluster_to_lba(&self, cluster: u32) -> u64 {
        self.data_offset + ((cluster as u64 - 2) * self.bpb.sectors_per_cluster as u64)
    }

    // ─── Read / write through cluster chains ───────────────────────────────

    pub fn read_chain(&self, start_cluster: u32, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        if start_cluster < 2 { return Ok(0); }
        let mut current_cluster = start_cluster;
        let mut bytes_read = 0;
        let cluster_size = self.cluster_size;

        let mut skip_clusters = offset / cluster_size;
        let mut cluster_offset = offset % cluster_size;

        while skip_clusters > 0 {
            current_cluster = self.next_cluster(current_cluster)?;
            if current_cluster >= FAT_ENTRY_EOC_MIN { return Ok(0); }
            skip_clusters -= 1;
        }

        while bytes_read < buf.len() && current_cluster < FAT_ENTRY_EOC_MIN {
            let cluster_lba = self.cluster_to_lba(current_cluster);

            while bytes_read < buf.len() && cluster_offset < cluster_size {
                let sector_in_cluster = cluster_offset / SECTOR_SIZE as u64;
                let sector_offset = (cluster_offset % SECTOR_SIZE as u64) as usize;

                let mut sector_data = [0u8; SECTOR_SIZE];
                self.device.read_sector(cluster_lba + sector_in_cluster, &mut sector_data)
                    .map_err(|_| VfsError::IoError)?;

                let remain_in_sector = SECTOR_SIZE - sector_offset;
                let remain_in_cluster = (cluster_size - cluster_offset) as usize;
                let remain_in_buf = buf.len() - bytes_read;
                let to_copy = remain_in_sector.min(remain_in_cluster).min(remain_in_buf);

                buf[bytes_read..bytes_read + to_copy]
                    .copy_from_slice(&sector_data[sector_offset..sector_offset + to_copy]);

                bytes_read += to_copy;
                cluster_offset += to_copy as u64;
            }

            if bytes_read >= buf.len() { break; }
            current_cluster = self.next_cluster(current_cluster)?;
            cluster_offset = 0;
        }

        Ok(bytes_read)
    }

    /// Write `data` starting at `offset` into the chain rooted at
    /// `start_cluster`. Extends the chain (and may even change its starting
    /// cluster, returned via `Ok((new_start, bytes_written))`). If
    /// `start_cluster == 0` the file currently has no clusters and the first
    /// one is allocated here.
    pub fn write_chain(
        &self,
        start_cluster: u32,
        offset: u64,
        data: &[u8],
    ) -> VfsResult<(u32, usize)> {
        if data.is_empty() {
            return Ok((start_cluster, 0));
        }

        let cluster_size = self.cluster_size;
        let total_end = offset + data.len() as u64;
        let clusters_needed = ((total_end + cluster_size - 1) / cluster_size) as u32;

        // Materialise the chain to at least `clusters_needed` clusters,
        // allocating fresh ones as we go.
        let mut chain: Vec<u32> = Vec::new();
        let mut cur = start_cluster;
        while cur >= 2 && cur < FAT_ENTRY_EOC_MIN && (chain.len() as u32) < clusters_needed {
            chain.push(cur);
            cur = self.next_cluster(cur)?;
        }
        while (chain.len() as u32) < clusters_needed {
            let new_cluster = self.alloc_cluster()?;
            if let Some(&prev) = chain.last() {
                self.write_fat_entry(prev, new_cluster)?;
            }
            chain.push(new_cluster);
        }
        let head = *chain.first().ok_or(VfsError::NoSpace)?;

        // Now write the data into the chain.
        let mut bytes_written = 0usize;
        let mut pos = offset;
        while bytes_written < data.len() {
            let cluster_index = (pos / cluster_size) as usize;
            let off_in_cluster = pos % cluster_size;
            let cluster = chain[cluster_index];
            let cluster_lba = self.cluster_to_lba(cluster);

            let sector_in_cluster = off_in_cluster / SECTOR_SIZE as u64;
            let sector_offset = (off_in_cluster % SECTOR_SIZE as u64) as usize;
            let lba = cluster_lba + sector_in_cluster;

            // Read-modify-write the sector unless we're writing it whole.
            let remain_in_sector = SECTOR_SIZE - sector_offset;
            let remain_in_cluster = (cluster_size - off_in_cluster) as usize;
            let remain_in_data = data.len() - bytes_written;
            let to_copy = remain_in_sector.min(remain_in_cluster).min(remain_in_data);

            let mut sector_buf = [0u8; SECTOR_SIZE];
            if sector_offset != 0 || to_copy != SECTOR_SIZE {
                self.device.read_sector(lba, &mut sector_buf).map_err(|_| VfsError::IoError)?;
            }
            sector_buf[sector_offset..sector_offset + to_copy]
                .copy_from_slice(&data[bytes_written..bytes_written + to_copy]);
            self.device.write_sector(lba, &sector_buf).map_err(|_| VfsError::IoError)?;

            bytes_written += to_copy;
            pos += to_copy as u64;
        }

        Ok((head, bytes_written))
    }

    // ─── Directory operations (raw entry slot access) ──────────────────────

    /// Iterate over directory cluster sectors, calling `f(lba, slot_idx, &mut entry)`
    /// for each 32-byte slot. Stops when `f` returns Some(_). Returns whatever
    /// the callback returned, or Ok(None) when the end-of-directory is hit.
    fn for_each_dir_slot<F, R>(&self, dir_cluster: u32, mut f: F) -> VfsResult<Option<R>>
    where
        F: FnMut(u64, usize, &FatDirEntry) -> Option<R>,
    {
        let mut cluster = dir_cluster;
        while cluster >= 2 && cluster < FAT_ENTRY_EOC_MIN {
            let cluster_lba = self.cluster_to_lba(cluster);
            for s in 0..self.bpb.sectors_per_cluster as u64 {
                let lba = cluster_lba + s;
                let mut sector = [0u8; SECTOR_SIZE];
                self.device.read_sector(lba, &mut sector).map_err(|_| VfsError::IoError)?;
                for slot in 0..(SECTOR_SIZE / DIR_ENTRY_SIZE) {
                    let off = slot * DIR_ENTRY_SIZE;
                    let entry: FatDirEntry = unsafe {
                        core::ptr::read_unaligned(sector.as_ptr().add(off) as *const FatDirEntry)
                    };
                    if entry.is_end() {
                        return Ok(None);
                    }
                    if let Some(r) = f(lba, slot, &entry) {
                        return Ok(Some(r));
                    }
                }
            }
            cluster = self.next_cluster(cluster)?;
        }
        Ok(None)
    }

    /// Find a free directory slot in `dir_cluster`, or allocate a new cluster
    /// to extend the directory. Returns (lba, slot_index).
    fn find_or_alloc_dir_slot(&self, dir_cluster: u32) -> VfsResult<(u64, usize)> {
        let mut cluster = dir_cluster;
        let mut last_cluster = dir_cluster;
        while cluster >= 2 && cluster < FAT_ENTRY_EOC_MIN {
            let cluster_lba = self.cluster_to_lba(cluster);
            for s in 0..self.bpb.sectors_per_cluster as u64 {
                let lba = cluster_lba + s;
                let mut sector = [0u8; SECTOR_SIZE];
                self.device.read_sector(lba, &mut sector).map_err(|_| VfsError::IoError)?;
                for slot in 0..(SECTOR_SIZE / DIR_ENTRY_SIZE) {
                    let off = slot * DIR_ENTRY_SIZE;
                    let first = sector[off];
                    if first == DIR_END || first == DIR_FREE {
                        return Ok((lba, slot));
                    }
                }
            }
            last_cluster = cluster;
            cluster = self.next_cluster(cluster)?;
        }
        // Need to extend the dir chain with a fresh, zeroed cluster.
        let new_cluster = self.alloc_cluster()?;
        self.write_fat_entry(last_cluster, new_cluster)?;
        Ok((self.cluster_to_lba(new_cluster), 0))
    }

    fn write_dir_entry(&self, lba: u64, slot: usize, entry: &FatDirEntry) -> VfsResult<()> {
        let mut sector = [0u8; SECTOR_SIZE];
        self.device.read_sector(lba, &mut sector).map_err(|_| VfsError::IoError)?;
        let off = slot * DIR_ENTRY_SIZE;
        unsafe {
            core::ptr::write_unaligned(
                sector.as_mut_ptr().add(off) as *mut FatDirEntry,
                *entry,
            );
        }
        self.device.write_sector(lba, &sector).map_err(|_| VfsError::IoError)?;
        Ok(())
    }

    fn find_dir_entry(
        &self, dir_cluster: u32, name: &str,
    ) -> VfsResult<Option<(u64, usize, FatDirEntry)>> {
        let want_lower = name.to_ascii_uppercase();
        self.for_each_dir_slot(dir_cluster, |lba, slot, entry| {
            if entry.is_deleted() || entry.is_lfn() || entry.is_volume() {
                return None;
            }
            let n = entry.get_name();
            if n.eq_ignore_ascii_case(&want_lower) || n == want_lower {
                return Some((lba, slot, *entry));
            }
            None
        })
    }

    fn update_dir_entry_size(&self, dir_cluster: u32, file_cluster: u32, size: u32) -> VfsResult<()> {
        let result = self.for_each_dir_slot(dir_cluster, |lba, slot, entry| {
            if entry.is_deleted() || entry.is_lfn() || entry.is_volume() {
                return None;
            }
            if entry.get_cluster() == file_cluster {
                return Some((lba, slot, *entry));
            }
            None
        })?;
        if let Some((lba, slot, mut entry)) = result {
            entry.size = size;
            self.write_dir_entry(lba, slot, &entry)?;
        }
        Ok(())
    }

    fn update_dir_entry_cluster(
        &self, dir_cluster: u32, name: &str, new_cluster: u32, new_size: u32,
    ) -> VfsResult<()> {
        if let Some((lba, slot, mut entry)) = self.find_dir_entry(dir_cluster, name)? {
            entry.set_cluster(new_cluster);
            entry.size = new_size;
            self.write_dir_entry(lba, slot, &entry)?;
        }
        Ok(())
    }

    // ─── High-level create/unlink/mkdir ────────────────────────────────────

    fn make_entry(name8: [u8; 8], ext3: [u8; 3], attr: u8, cluster: u32, size: u32) -> FatDirEntry {
        let mut e = FatDirEntry {
            name: name8,
            ext: ext3,
            attr,
            nt_reserved: 0,
            creation_time_tenths: 0,
            creation_time: 0,
            creation_date: 0,
            last_access_date: 0,
            first_cluster_high: 0,
            write_time: 0,
            write_date: 0,
            first_cluster_low: 0,
            size,
        };
        e.set_cluster(cluster);
        e
    }

    /// Create an empty regular file in `parent_cluster`. Returns the file's
    /// first cluster (used as its inode number in the VFS layer).
    pub fn create_file(&self, parent_cluster: u32, name: &str) -> VfsResult<u32> {
        if self.find_dir_entry(parent_cluster, name)?.is_some() {
            return Err(VfsError::AlreadyExists);
        }
        let (name8, ext3) = encode_short_name(name)?;
        let cluster = self.alloc_cluster()?;
        let entry = Self::make_entry(name8, ext3, ATTR_ARCHIVE, cluster, 0);
        let (lba, slot) = self.find_or_alloc_dir_slot(parent_cluster)?;
        self.write_dir_entry(lba, slot, &entry)?;
        self.remember_parent(parent_cluster, cluster);
        self.set_cached_size(cluster as InodeNum, 0);
        Ok(cluster)
    }

    /// Create an empty subdirectory. Initialises "." and ".." entries.
    pub fn create_dir(&self, parent_cluster: u32, name: &str) -> VfsResult<u32> {
        if self.find_dir_entry(parent_cluster, name)?.is_some() {
            return Err(VfsError::AlreadyExists);
        }
        let (name8, ext3) = encode_short_name(name)?;
        let cluster = self.alloc_cluster()?;

        // Build "." and ".." entries inside the new directory's cluster.
        let dot_name = *b".       ";
        let dotdot_name = *b"..      ";
        let space_ext  = *b"   ";
        let dot = Self::make_entry(dot_name, space_ext, ATTR_DIRECTORY, cluster, 0);
        // ".." stores 0 when the parent is the root cluster.
        let parent_for_dotdot = if parent_cluster == self.bpb.root_cluster { 0 } else { parent_cluster };
        let dotdot = Self::make_entry(dotdot_name, space_ext, ATTR_DIRECTORY, parent_for_dotdot, 0);

        let cluster_lba = self.cluster_to_lba(cluster);
        // The cluster is freshly allocated and zeroed by alloc_cluster, so a
        // direct slot write is safe.
        self.write_dir_entry(cluster_lba, 0, &dot)?;
        self.write_dir_entry(cluster_lba, 1, &dotdot)?;

        let entry = Self::make_entry(name8, ext3, ATTR_DIRECTORY, cluster, 0);
        let (lba, slot) = self.find_or_alloc_dir_slot(parent_cluster)?;
        self.write_dir_entry(lba, slot, &entry)?;
        self.remember_parent(parent_cluster, cluster);
        Ok(cluster)
    }

    /// Remove a file or empty subdirectory. Returns Err(IsADirectory) for
    /// non-empty dirs (kept as IsADirectory to avoid plumbing ENOTEMPTY).
    pub fn unlink(&self, parent_cluster: u32, name: &str) -> VfsResult<()> {
        let (lba, slot, entry) = self.find_dir_entry(parent_cluster, name)?
            .ok_or(VfsError::NotFound)?;

        if entry.is_dir() {
            // Empty = only "." and ".." (or no entries at all).
            let mut non_special = 0u32;
            self.for_each_dir_slot(entry.get_cluster(), |_, _, e| {
                if e.is_deleted() || e.is_lfn() { return None; }
                let n = e.get_name();
                if n != "." && n != ".." { non_special += 1; }
                None::<()>
            })?;
            if non_special > 0 { return Err(VfsError::InvalidArgument); }
        }

        let cluster = entry.get_cluster();
        // Mark dir entry deleted in-place.
        let mut sector = [0u8; SECTOR_SIZE];
        self.device.read_sector(lba, &mut sector).map_err(|_| VfsError::IoError)?;
        sector[slot * DIR_ENTRY_SIZE] = DIR_FREE;
        self.device.write_sector(lba, &sector).map_err(|_| VfsError::IoError)?;

        // Free its cluster chain.
        if cluster >= 2 {
            self.free_chain(cluster)?;
        }
        Ok(())
    }

    pub fn lookup_in_dir(&self, parent_cluster: u32, name: &str) -> VfsResult<u32> {
        let (_, _, entry) = self.find_dir_entry(parent_cluster, name)?
            .ok_or(VfsError::NotFound)?;
        let c = entry.get_cluster();
        // Cache metadata + size + parent so VFS walk-by-inode can resolve this
        // cluster later (Fat32Filesystem::get_inode reads from this cache).
        let ft = if entry.is_dir() { FileType::Directory } else { FileType::Regular };
        let mut meta = InodeMetadata::new(c as InodeNum, ft);
        meta.mode = FileMode::new(if ft == FileType::Directory { 0o755 } else { 0o644 });
        meta.size = if ft == FileType::Regular { entry.size as u64 } else { 0 };
        self.cache_metadata(c as InodeNum, meta);
        if !entry.is_dir() {
            self.set_cached_size(c as InodeNum, entry.size as u64);
        }
        self.remember_parent(parent_cluster, c);
        Ok(c)
    }

    /// Split a path into (parent_cluster, leaf_name) starting at root.
    pub fn split_parent_leaf<'a>(&self, path: &'a str) -> VfsResult<(u32, &'a str)> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Err(VfsError::InvalidArgument);
        }
        let leaf = parts[parts.len() - 1];
        let mut cur = self.bpb.root_cluster;
        for &part in &parts[..parts.len() - 1] {
            cur = self.lookup_in_dir(cur, part)?;
        }
        Ok((cur, leaf))
    }
}

pub struct FatInode {
    pub fs: Arc<Fat32Fs>,
    pub cluster: u32,
    pub metadata: InodeMetadata,
}

impl FatInode {
    pub fn new(fs: Arc<Fat32Fs>, cluster: u32, metadata: InodeMetadata) -> Self {
        FatInode { fs, cluster, metadata }
    }
}

impl InodeOps for FatInode {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        if self.metadata.file_type == FileType::Directory {
            return Err(VfsError::IsADirectory);
        }
        // The cached size on the dir entry takes priority — it may have been
        // updated by another open handle.
        let size = self.fs.cached_size(self.metadata.ino).unwrap_or(self.metadata.size);
        if offset >= size { return Ok(0); }
        let to_read = (size - offset).min(buf.len() as u64) as usize;
        self.fs.read_chain(self.cluster, offset, &mut buf[..to_read])
    }

    fn write(&self, offset: u64, buf: &[u8]) -> VfsResult<usize> {
        if self.metadata.file_type == FileType::Directory {
            return Err(VfsError::IsADirectory);
        }
        let (new_head, n) = self.fs.write_chain(self.cluster, offset, buf)?;
        let new_size = (offset + n as u64).max(
            self.fs.cached_size(self.metadata.ino).unwrap_or(self.metadata.size),
        );
        self.fs.set_cached_size(self.metadata.ino, new_size);
        if let Some(parent) = self.fs.get_parent(self.cluster) {
            if new_head != self.cluster {
                // Cluster zero start: rewrite both cluster and size by name —
                // requires us to know the dir entry's name, which we don't
                // track in this minimal layer. Fall back: just update size of
                // the existing entry by old cluster; the new_head case only
                // arises if cluster was 0, which create_file precludes.
            }
            let _ = self.fs.update_dir_entry_size(parent, self.cluster, new_size as u32);
        }
        Ok(n)
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let mut m = self.metadata.clone();
        if let Some(sz) = self.fs.cached_size(m.ino) {
            m.size = sz;
        }
        Ok(m)
    }

    fn set_metadata(&self, _meta: &InodeMetadata) -> VfsResult<()> {
        // FAT has no per-file mode/uid storage. Accept the call as a no-op so
        // sys_open's post-create chmod doesn't fail.
        Ok(())
    }

    fn lookup(&self, name: &str) -> VfsResult<InodeNum> {
        if self.metadata.file_type != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }
        let c = self.fs.lookup_in_dir(self.cluster, name)?;
        Ok(c as InodeNum)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        if self.metadata.file_type != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }
        let mut entries = Vec::new();
        self.fs.for_each_dir_slot(self.cluster, |_, _, entry| {
            if entry.is_deleted() || entry.is_lfn() || entry.is_volume() {
                return None;
            }
            let name = entry.get_name();
            if name == "." || name == ".." {
                return None;
            }
            let ft = if entry.is_dir() { FileType::Directory } else { FileType::Regular };
            let ino = entry.get_cluster() as InodeNum;
            let mut meta = InodeMetadata::new(ino, ft);
            meta.mode = FileMode::new(if ft == FileType::Directory { 0o755 } else { 0o644 });
            meta.size = if ft == FileType::Regular { entry.size as u64 } else { 0 };
            self.fs.cache_metadata(ino, meta);
            if ft == FileType::Regular {
                self.fs.set_cached_size(ino, entry.size as u64);
            }
            self.fs.remember_parent(self.cluster, entry.get_cluster());
            entries.push(DirEntry { name, ino, file_type: ft });
            None::<()>
        })?;
        Ok(entries)
    }

    fn sync(&self) -> VfsResult<()> { Ok(()) }
}

pub struct Fat32Filesystem {
    inner: Arc<Fat32Fs>,
}

impl Fat32Filesystem {
    pub fn new(fat32: Arc<Fat32Fs>) -> Arc<Self> {
        Arc::new(Fat32Filesystem { inner: fat32 })
    }

    /// Concrete FS handle, used by syscall handlers to route writes via the
    /// per-mount instance instead of any global singleton.
    pub fn inner(&self) -> Arc<Fat32Fs> {
        self.inner.clone()
    }
}

impl Filesystem for Fat32Filesystem {
    fn root_inode(&self) -> Arc<dyn InodeOps> {
        let root_ino = self.inner.bpb.root_cluster as InodeNum;
        let mut meta = self
            .inner
            .get_cached_metadata(root_ino)
            .unwrap_or_else(|| InodeMetadata::new(root_ino, FileType::Directory));
        meta.mode = FileMode::new(0o755);
        Arc::new(FatInode::new(self.inner.clone(), root_ino as u32, meta))
    }

    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>> {
        let root_ino = self.inner.bpb.root_cluster as InodeNum;
        let meta = if ino == root_ino {
            let mut meta = self
                .inner
                .get_cached_metadata(ino)
                .unwrap_or_else(|| InodeMetadata::new(ino, FileType::Directory));
            meta.mode = FileMode::new(0o755);
            meta
        } else {
            self.inner.get_cached_metadata(ino).ok_or(VfsError::NotFound)?
        };

        Ok(Arc::new(FatInode::new(self.inner.clone(), ino as u32, meta)))
    }

    fn name(&self) -> &str {
        "fat32"
    }

    fn as_any(&self) -> &dyn core::any::Any { self }
}

// ─── In-kernel format helper ───────────────────────────────────────────────

/// Format the given block device with a minimal FAT32 layout. Used by tests,
/// boot-time scaffolding for `/fat`, and the future `mkfs.fat32` userland
/// tool. Returns a freshly mounted Fat32Fs.
pub fn format_fat32(device: Arc<dyn BlockDevice>, label: &str) -> VfsResult<Arc<Fat32Fs>> {
    let total_sectors = device.sector_count();
    if total_sectors < 256 {
        return Err(VfsError::NoSpace);
    }

    // Layout choices.
    let bytes_per_sector: u16 = SECTOR_SIZE as u16;
    let sectors_per_cluster: u8 = 1;
    let reserved_sectors: u16 = 32;
    let fat_count: u8 = 1;
    let root_cluster: u32 = 2;

    // Iteratively size the FAT until everything fits.
    let mut sectors_per_fat: u32 = 1;
    for _ in 0..32 {
        let data_offset = reserved_sectors as u64 + fat_count as u64 * sectors_per_fat as u64;
        if data_offset >= total_sectors { return Err(VfsError::NoSpace); }
        let data_sectors = total_sectors - data_offset;
        let clusters = (data_sectors / sectors_per_cluster as u64) as u32;
        // Each FAT must cover (clusters + 2) entries × 4 bytes.
        let needed_fat_bytes = (clusters + 2) as u64 * 4;
        let needed_fat_sectors = ((needed_fat_bytes + SECTOR_SIZE as u64 - 1) / SECTOR_SIZE as u64) as u32;
        if needed_fat_sectors <= sectors_per_fat { break; }
        sectors_per_fat = needed_fat_sectors;
    }

    let total_sectors_32: u32 = total_sectors.min(u32::MAX as u64) as u32;

    // Build BPB.
    let mut bpb = BpbFat32 {
        jmp: [0xEB, 0x58, 0x90],
        oem_name: *b"RACOS1.0",
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        fat_count,
        root_entry_count: 0,
        total_sectors_16: 0,
        media_type: 0xF8,
        sectors_per_fat_16: 0,
        sectors_per_track: 0,
        head_count: 0,
        hidden_sectors: 0,
        total_sectors_32,
        sectors_per_fat_32: sectors_per_fat,
        flags: 0,
        version: 0,
        root_cluster,
        fs_info_sector: 1,
        backup_boot_sector: 6,
        reserved_bytes: [0u8; 12],
        drive_number: 0x80,
        nt_reserved: 0,
        signature: 0x29,
        serial_number: 0x5241_4332, // "RAC2"
        vol_label: [b' '; 11],
        sys_id: *b"FAT32   ",
    };
    // Volume label (uppercase, space-padded, max 11 chars).
    for (i, ch) in label.bytes().take(11).enumerate() {
        bpb.vol_label[i] = ascii_upper(ch);
    }

    // Sector 0: boot sector with BPB + signature.
    let mut boot = [0u8; SECTOR_SIZE];
    unsafe { core::ptr::write_unaligned(boot.as_mut_ptr() as *mut BpbFat32, bpb); }
    boot[510] = 0x55;
    boot[511] = 0xAA;
    device.write_sector(0, &boot).map_err(|_| VfsError::IoError)?;

    // Backup boot sector.
    device.write_sector(bpb.backup_boot_sector as u64, &boot).map_err(|_| VfsError::IoError)?;

    // FSInfo (sector 1): lead 0x41615252, struc 0x61417272, free_count = 0xFFFFFFFF (unknown),
    // next_free = 0xFFFFFFFF, trail 0xAA550000.
    let mut fsinfo = [0u8; SECTOR_SIZE];
    fsinfo[0..4].copy_from_slice(&0x41615252u32.to_le_bytes());
    fsinfo[484..488].copy_from_slice(&0x61417272u32.to_le_bytes());
    fsinfo[488..492].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    fsinfo[492..496].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    fsinfo[508..512].copy_from_slice(&0xAA550000u32.to_le_bytes());
    device.write_sector(1, &fsinfo).map_err(|_| VfsError::IoError)?;

    // Zero the FAT region.
    let zero = [0u8; SECTOR_SIZE];
    let fat_offset = reserved_sectors as u64;
    for fat_idx in 0..fat_count as u64 {
        for s in 0..sectors_per_fat as u64 {
            device.write_sector(fat_offset + fat_idx * sectors_per_fat as u64 + s, &zero)
                .map_err(|_| VfsError::IoError)?;
        }
    }

    // Initialise FAT[0..=1] (media + EOC sentinels) and FAT[2] (root cluster = EOC).
    let mut fat0 = [0u8; SECTOR_SIZE];
    fat0[0..4].copy_from_slice(&(0x0FFF_FF00u32 | bpb.media_type as u32).to_le_bytes());
    fat0[4..8].copy_from_slice(&FAT_ENTRY_EOC.to_le_bytes());
    fat0[8..12].copy_from_slice(&FAT_ENTRY_EOC.to_le_bytes()); // root cluster (#2) = EOC
    for fat_idx in 0..fat_count as u64 {
        device.write_sector(fat_offset + fat_idx * sectors_per_fat as u64, &fat0)
            .map_err(|_| VfsError::IoError)?;
    }

    // Zero the root cluster.
    let data_offset = fat_offset + fat_count as u64 * sectors_per_fat as u64;
    let root_lba = data_offset + (root_cluster as u64 - 2) * sectors_per_cluster as u64;
    for s in 0..sectors_per_cluster as u64 {
        device.write_sector(root_lba + s, &zero).map_err(|_| VfsError::IoError)?;
    }

    crate::serial::serial_println!(
        "[ FAT32 ] Formatted '{}': {} total sectors, {} FAT sectors, root cluster {}",
        device.name(), total_sectors, sectors_per_fat, root_cluster,
    );

    Fat32Fs::new(device)
}

// ─── Smoke test (called from main) ─────────────────────────────────────────

/// Exercise the full FAT32 write path against a freshly mounted instance.
/// Logs PASS/FAIL lines so the boot transcript records F.4 status.
pub fn smoke_test(fs: &Arc<Fat32Fs>) {
    use alloc::string::ToString;
    let root = fs.bpb.root_cluster;

    // 1. mkdir /TEST
    match fs.create_dir(root, "TEST") {
        Ok(c) => crate::serial::serial_println!("[ FAT32 ] mkdir /TEST OK (cluster {})", c),
        Err(VfsError::AlreadyExists) => {
            crate::serial::serial_println!("[ FAT32 ] /TEST already exists (re-running test)")
        }
        Err(e) => {
            crate::serial::serial_println!("[ FAT32 ] FAIL mkdir /TEST: {:?}", e);
            return;
        }
    }

    let test_cluster = match fs.lookup_in_dir(root, "TEST") {
        Ok(c) => c,
        Err(e) => {
            crate::serial::serial_println!("[ FAT32 ] FAIL lookup /TEST: {:?}", e);
            return;
        }
    };

    // 2. boot-counter style persistence check.
    const NAME: &str = "BOOT.CNT";
    let file_cluster = match fs.find_dir_entry(test_cluster, NAME) {
        Ok(Some((_, _, entry))) => entry.get_cluster(),
        Ok(None) => match fs.create_file(test_cluster, NAME) {
            Ok(c) => {
                crate::serial::serial_println!("[ FAT32 ] created {} (cluster {})", NAME, c);
                c
            }
            Err(e) => {
                crate::serial::serial_println!("[ FAT32 ] FAIL create {}: {:?}", NAME, e);
                return;
            }
        }
        Err(e) => {
            crate::serial::serial_println!("[ FAT32 ] FAIL find {}: {:?}", NAME, e);
            return;
        }
    };

    // Read current value (if any).
    let mut buf = [0u8; 16];
    let size = fs.cached_size(file_cluster as InodeNum).unwrap_or(0);
    let n = if size > 0 {
        fs.read_chain(file_cluster, 0, &mut buf[..size as usize]).unwrap_or(0)
    } else {
        0
    };
    let text = core::str::from_utf8(&buf[..n]).unwrap_or("0").trim();
    let value: u32 = text.parse().unwrap_or(0);
    let next = value.saturating_add(1);
    let s = next.to_string();

    match fs.write_chain(file_cluster, 0, s.as_bytes()) {
        Ok((_, w)) => {
            fs.set_cached_size(file_cluster as InodeNum, w as u64);
            let _ = fs.update_dir_entry_size(test_cluster, file_cluster, w as u32);
            crate::serial::serial_println!(
                "[ FAT32 ] boot-counter = {} (was {}, wrote {} bytes)", next, value, w,
            );
        }
        Err(e) => crate::serial::serial_println!("[ FAT32 ] FAIL write: {:?}", e),
    }

    // 3. Read-back verification of the *just-written* value.
    let mut verify = [0u8; 16];
    let vn = fs.read_chain(file_cluster, 0, &mut verify[..s.len()]).unwrap_or(0);
    if &verify[..vn] == s.as_bytes() {
        crate::serial::serial_println!("[ FAT32 ] read-back PASS: '{}'", s);
    } else {
        crate::serial::serial_println!(
            "[ FAT32 ] read-back FAIL: got '{}'",
            core::str::from_utf8(&verify[..vn]).unwrap_or("?"),
        );
    }
}
