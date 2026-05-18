// RaCore — FAT32 Filesystem Implementation (ADR-009)
//
// Full FAT32 implementation including BPB parsing, FAT table traversal,
// and cluster-based sector I/O via BlockDevice.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::string::String;
use crate::vfs::inode::{InodeOps, VfsResult, VfsError, FileType, FileMode, InodeMetadata, DirEntry, InodeNum};
use crate::vfs::mount::Filesystem;
use crate::drivers::block::{BlockDevice, SECTOR_SIZE};
use crate::sync::SpinLock;
use core::mem::size_of;

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
    pub fn is_end(&self) -> bool { self.name[0] == 0x00 }
    pub fn is_deleted(&self) -> bool { self.name[0] == 0xE5 }
    pub fn is_lfn(&self) -> bool { self.attr == 0x0F }
    
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
}

pub struct Fat32Fs {
    pub device: Arc<dyn BlockDevice>,
    pub bpb: BpbFat32,
    pub fat_offset: u64,
    pub data_offset: u64,
    metadata_cache: SpinLock<Vec<(InodeNum, InodeMetadata)>>,
}

impl Fat32Fs {
    /// Create a new FAT32 instance by reading BPB from a device.
    pub fn new(device: Arc<dyn BlockDevice>) -> VfsResult<Arc<Self>> {
        let mut sector = [0u8; SECTOR_SIZE];
        device.read_sector(0, &mut sector).map_err(|_| VfsError::IoError)?;

        let bpb: BpbFat32 = unsafe { core::ptr::read_unaligned(sector.as_ptr() as *const BpbFat32) };

        // Verify signatures (basic validation)
        if bpb.signature != 0x28 && bpb.signature != 0x29 {
             return Err(VfsError::InvalidArgument);
        }

        let fat_offset = bpb.reserved_sectors as u64;
        let data_offset = fat_offset + (bpb.fat_count as u64 * bpb.sectors_per_fat_32 as u64);

        let fs = Arc::new(Fat32Fs {
            device,
            bpb,
            fat_offset,
            data_offset,
            metadata_cache: SpinLock::new(Vec::new()),
        });

        let root_ino = fs.bpb.root_cluster as InodeNum;
        let mut root_meta = InodeMetadata::new(root_ino, FileType::Directory);
        root_meta.mode = FileMode::new(0o755);
        fs.cache_metadata(root_ino, root_meta);

        Ok(fs)
    }

    fn cache_metadata(&self, ino: InodeNum, meta: InodeMetadata) {
        let mut cache = self.metadata_cache.lock();
        if let Some((_, existing)) = cache.iter_mut().find(|(cached_ino, _)| *cached_ino == ino) {
            *existing = meta;
            return;
        }
        cache.push((ino, meta));
    }

    fn get_cached_metadata(&self, ino: InodeNum) -> Option<InodeMetadata> {
        let cache = self.metadata_cache.lock();
        cache
            .iter()
            .find(|(cached_ino, _)| *cached_ino == ino)
            .map(|(_, meta)| meta.clone())
    }

    /// Read the FAT entry for a given cluster.
    pub fn next_cluster(&self, cluster: u32) -> VfsResult<u32> {
        let fat_sector = self.fat_offset + (cluster as u64 * 4 / SECTOR_SIZE as u64);
        let offset = (cluster as usize * 4) % SECTOR_SIZE;

        let mut sector_data = [0u8; SECTOR_SIZE];
        self.device.read_sector(fat_sector, &mut sector_data).map_err(|_| VfsError::IoError)?;

        let entry = unsafe { core::ptr::read_unaligned(sector_data.as_ptr().add(offset) as *const u32) };
        Ok(entry & 0x0FFFFFFF) // FAT32 uses 28 bits
    }

    /// Convert cluster number to absolute LBA.
    fn cluster_to_lba(&self, cluster: u32) -> u64 {
        self.data_offset + ((cluster as u64 - 2) * self.bpb.sectors_per_cluster as u64)
    }

    /// Read data from a cluster chain.
    pub fn read_chain(&self, start_cluster: u32, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let mut current_cluster = start_cluster;
        let mut bytes_read = 0;
        let cluster_size = self.bpb.sectors_per_cluster as u64 * SECTOR_SIZE as u64;

        // Skip to offset
        let mut skip_clusters = offset / cluster_size;
        let mut cluster_offset = offset % cluster_size;

        while skip_clusters > 0 {
            current_cluster = self.next_cluster(current_cluster)?;
            if current_cluster >= 0x0ffffff8 { return Ok(0); } // End of chain
            skip_clusters -= 1;
        }

        // Read data sector-by-sector within each cluster, then advance FAT chain.
        while bytes_read < buf.len() && current_cluster < 0x0ffffff8 {
            let cluster_lba = self.cluster_to_lba(current_cluster);

            while bytes_read < buf.len() && cluster_offset < cluster_size {
                let sector_in_cluster = cluster_offset / SECTOR_SIZE as u64;
                let sector_offset = (cluster_offset % SECTOR_SIZE as u64) as usize;

                let mut sector_data = [0u8; SECTOR_SIZE];
                self.device
                    .read_sector(cluster_lba + sector_in_cluster, &mut sector_data)
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

            if bytes_read >= buf.len() {
                break;
            }

            current_cluster = self.next_cluster(current_cluster)?;
            cluster_offset = 0;
        }

        Ok(bytes_read)
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
        let size = self.metadata.size;
        if offset >= size { return Ok(0); }
        let to_read = (size - offset).min(buf.len() as u64) as usize;
        self.fs.read_chain(self.cluster, offset, &mut buf[..to_read])
    }

    fn write(&self, _offset: u64, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::PermissionDenied)
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        Ok(self.metadata.clone())
    }

    fn lookup(&self, name: &str) -> VfsResult<InodeNum> {
        let entries = self.readdir()?;
        for entry in entries {
            if entry.name.eq_ignore_ascii_case(name) {
                return Ok(entry.ino);
            }
        }
        Err(VfsError::NotFound)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        if self.metadata.file_type != FileType::Directory {
             return Err(VfsError::NotADirectory);
        }

        let mut entries = Vec::new();
        let mut offset = 0;
        loop {
            let mut entry_data = [0u8; size_of::<FatDirEntry>()];
            let n = self.fs.read_chain(self.cluster, offset as u64, &mut entry_data)?;
            if n < size_of::<FatDirEntry>() { break; }
            
            let entry: FatDirEntry = unsafe { core::ptr::read_unaligned(entry_data.as_ptr() as *const FatDirEntry) };
            if entry.is_end() { break; }
            if !entry.is_deleted() && !entry.is_lfn() && (entry.attr & 0x08 == 0) { // Not Volume Label
                let name = entry.get_name();
                if name != "." && name != ".." {
                    let ft = if entry.attr & 0x10 != 0 { FileType::Directory } else { FileType::Regular };
                    let ino = entry.get_cluster() as InodeNum;
                    let mut meta = InodeMetadata::new(ino, ft);
                    meta.mode = FileMode::new(if ft == FileType::Directory { 0o755 } else { 0o644 });
                    meta.size = if ft == FileType::Regular { entry.size as u64 } else { 0 };
                    self.fs.cache_metadata(ino, meta);
                    entries.push(DirEntry {
                        name,
                        ino,
                        file_type: ft,
                    });
                }
            }
            offset += size_of::<FatDirEntry>();
        }
        Ok(entries)
    }
}

pub struct Fat32Filesystem {
    inner: Arc<Fat32Fs>,
}

impl Fat32Filesystem {
    pub fn new(fat32: Arc<Fat32Fs>) -> Arc<Self> {
        Arc::new(Fat32Filesystem { inner: fat32 })
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
}


