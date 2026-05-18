// RaCore — Block cache (page/block cache MVP)
//
// Provides a write-back cache layer between filesystems and block devices.
// Sectors are cached in memory and flushed on demand or when evicted.
//
// MVP: simple direct-mapped cache with LRU eviction, single-core safe
// (cli/sti serialization like the rest of the kernel).

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use super::block::{BlockDevice, BlockError, BlockResult, SECTOR_SIZE};

/// Maximum number of cached sectors (256 sectors = 128 KiB with 512-byte sectors).
const MAX_CACHED_SECTORS: usize = 256;

/// A cached sector entry.
struct CacheEntry {
    /// Logical block address on the device.
    lba: u64,
    /// Cached sector data.
    data: [u8; SECTOR_SIZE],
    /// Whether this entry has been modified since last flush.
    dirty: bool,
    /// Access counter for LRU eviction.
    access_seq: u64,
}

/// Block cache sitting between a filesystem and a block device.
pub struct BlockCache {
    device: Arc<dyn BlockDevice>,
    entries: Vec<CacheEntry>,
    /// Monotonically increasing access counter.
    seq: u64,
    /// Statistics.
    hits: u64,
    misses: u64,
}

// SAFETY: Access serialized by cli/sti (single-core MVP).
unsafe impl Send for BlockCache {}
unsafe impl Sync for BlockCache {}

impl BlockCache {
    /// Create a new cache wrapping the given block device.
    pub fn new(device: Arc<dyn BlockDevice>) -> Self {
        BlockCache {
            device,
            entries: Vec::new(),
            seq: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Read a sector through the cache.
    pub fn read_sector(&mut self, lba: u64, out: &mut [u8]) -> BlockResult<()> {
        if out.len() != SECTOR_SIZE {
            return Err(BlockError::InvalidBuffer);
        }
        if lba >= self.device.sector_count() {
            return Err(BlockError::OutOfRange);
        }

        // Check cache hit.
        self.seq += 1;
        let seq = self.seq;
        for entry in self.entries.iter_mut() {
            if entry.lba == lba {
                entry.access_seq = seq;
                out.copy_from_slice(&entry.data);
                self.hits += 1;
                return Ok(());
            }
        }

        // Cache miss — read from device.
        self.misses += 1;
        let mut buf = [0u8; SECTOR_SIZE];
        self.device.read_sector(lba, &mut buf)?;

        // Insert into cache, evicting LRU if full.
        if self.entries.len() >= MAX_CACHED_SECTORS {
            self.evict_one()?;
        }
        self.entries.push(CacheEntry {
            lba,
            data: buf,
            dirty: false,
            access_seq: seq,
        });
        out.copy_from_slice(&buf);
        Ok(())
    }

    /// Write a sector through the cache (write-back: data is buffered).
    pub fn write_sector(&mut self, lba: u64, input: &[u8]) -> BlockResult<()> {
        if input.len() != SECTOR_SIZE {
            return Err(BlockError::InvalidBuffer);
        }
        if lba >= self.device.sector_count() {
            return Err(BlockError::OutOfRange);
        }

        self.seq += 1;
        let seq = self.seq;

        // Update existing entry if present.
        for entry in self.entries.iter_mut() {
            if entry.lba == lba {
                entry.data.copy_from_slice(input);
                entry.dirty = true;
                entry.access_seq = seq;
                return Ok(());
            }
        }

        // Not cached — insert new entry.
        if self.entries.len() >= MAX_CACHED_SECTORS {
            self.evict_one()?;
        }
        let mut data = [0u8; SECTOR_SIZE];
        data.copy_from_slice(input);
        self.entries.push(CacheEntry {
            lba,
            data,
            dirty: true,
            access_seq: seq,
        });
        Ok(())
    }

    /// Flush all dirty entries to the device.
    pub fn flush(&mut self) -> BlockResult<()> {
        for entry in self.entries.iter_mut() {
            if entry.dirty {
                self.device.write_sector(entry.lba, &entry.data)?;
                entry.dirty = false;
            }
        }
        Ok(())
    }

    /// Flush and invalidate all entries.
    pub fn flush_and_invalidate(&mut self) -> BlockResult<()> {
        self.flush()?;
        self.entries.clear();
        Ok(())
    }

    /// Number of dirty entries.
    pub fn dirty_count(&self) -> usize {
        self.entries.iter().filter(|e| e.dirty).count()
    }

    /// Cache statistics: (hits, misses, cached_entries).
    pub fn stats(&self) -> (u64, u64, usize) {
        (self.hits, self.misses, self.entries.len())
    }

    /// Evict the least-recently-used entry, flushing if dirty.
    fn evict_one(&mut self) -> BlockResult<()> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let mut lru_idx = 0;
        let mut lru_seq = self.entries[0].access_seq;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.access_seq < lru_seq {
                lru_seq = entry.access_seq;
                lru_idx = i;
            }
        }
        // Flush if dirty.
        if self.entries[lru_idx].dirty {
            let e = &self.entries[lru_idx];
            self.device.write_sector(e.lba, &e.data)?;
        }
        self.entries.swap_remove(lru_idx);
        Ok(())
    }
}
