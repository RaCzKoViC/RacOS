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
    /// Sectors written since `begin_recording`, in write order, or None when
    /// not recording.
    ///
    /// This exists for the racfs metadata journal, which has to know exactly
    /// which sectors an operation touched *before* any of them reaches the
    /// disk. See `evict_one` for the other half of that guarantee.
    recording: Option<Vec<u64>>,
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
            recording: None,
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

        if let Some(list) = self.recording.as_mut() {
            if !list.contains(&lba) {
                list.push(lba);
            }
        }

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
    ///
    /// Sectors belonging to an open transaction are skipped, for the same
    /// reason `evict_one` skips them: nothing an operation touched may reach
    /// the disk before the journal's commit record does. racfs operations
    /// call `flush()` at the end of their own work, so without this the
    /// journal would be writing a log of changes the disk already had.
    pub fn flush(&mut self) -> BlockResult<()> {
        // Collected first: the borrow checker will not allow consulting
        // `self.recording` while `self.entries` is mutably borrowed.
        let pinned: Vec<u64> = match self.recording.as_ref() {
            Some(list) => list.clone(),
            None => Vec::new(),
        };
        for entry in self.entries.iter_mut() {
            if entry.dirty && !pinned.contains(&entry.lba) {
                self.device.write_sector(entry.lba, &entry.data)?;
                entry.dirty = false;
            }
        }
        Ok(())
    }

    /// Drop the cached copies of an aborted transaction's sectors without
    /// writing them, so the next read comes from the disk again.
    ///
    /// This is the rollback: those sectors were never allowed onto the
    /// device, so forgetting them is all it takes to undo a half-finished
    /// operation.
    pub fn discard_recorded(&mut self) {
        let pinned: Vec<u64> = match self.recording.as_ref() {
            Some(list) => list.clone(),
            None => return,
        };
        self.entries
            .retain(|e| !(e.dirty && pinned.contains(&e.lba)));
        self.recording = None;
    }

    /// Flush and invalidate all entries.
    pub fn flush_and_invalidate(&mut self) -> BlockResult<()> {
        self.flush()?;
        self.entries.clear();
        Ok(())
    }

    /// Start recording which sectors get written, for the racfs journal.
    ///
    /// Recording is not nesting: a second call discards whatever the first
    /// collected. racfs opens exactly one transaction at a time and every
    /// path out of one ends in `end_recording`, so a nested begin would mean
    /// a bug worth losing the list over rather than silently merging two
    /// operations into one transaction.
    pub fn begin_recording(&mut self) {
        self.recording = Some(Vec::new());
    }

    /// Sectors written since `begin_recording`, in write order.
    pub fn recorded(&self) -> &[u64] {
        match self.recording.as_ref() {
            Some(list) => list.as_slice(),
            None => &[],
        }
    }

    /// Stop recording. Cached data is untouched; only the list is dropped.
    pub fn end_recording(&mut self) {
        self.recording = None;
    }

    /// Read a sector as the cache currently sees it, without disturbing LRU
    /// order or counting a hit. Used by the journal to copy an operation's
    /// sectors into the log before they are allowed in place.
    pub fn peek_sector(&mut self, lba: u64, out: &mut [u8; SECTOR_SIZE]) -> BlockResult<()> {
        for entry in self.entries.iter() {
            if entry.lba == lba {
                out.copy_from_slice(&entry.data);
                return Ok(());
            }
        }
        self.device.read_sector(lba, out)
    }

    /// Write straight to the device, bypassing the cache entirely.
    ///
    /// The journal needs this for its own sectors. Going through the cache
    /// would leave the log's ordering at the mercy of eviction, and the log
    /// is the one thing whose ordering has to be exact.
    pub fn write_through(&mut self, lba: u64, data: &[u8; SECTOR_SIZE]) -> BlockResult<()> {
        // Drop any cached copy first, or a later read would serve stale data
        // for a sector the device has just been given a newer version of.
        self.entries.retain(|e| e.lba != lba);
        self.device.write_sector(lba, data)
    }

    /// Read straight from the device, bypassing the cache.
    pub fn read_through(&mut self, lba: u64, out: &mut [u8; SECTOR_SIZE]) -> BlockResult<()> {
        self.device.read_sector(lba, out)
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

        // A sector recorded by the open transaction must not reach the device
        // yet: the journal's whole guarantee is that nothing lands in place
        // before the commit record does. Evicting one here would put a
        // half-applied operation on disk with no journal entry describing it —
        // exactly the torn state journaling exists to prevent, arriving
        // through the one path that looks like harmless cache maintenance.
        //
        // Such an entry is skipped rather than written. Clean entries can
        // always go, since re-reading them costs a read and nothing else.
        let mut lru_idx: Option<usize> = None;
        let mut lru_seq = u64::MAX;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.dirty && self.is_recorded(entry.lba) {
                continue;
            }
            if entry.access_seq < lru_seq {
                lru_seq = entry.access_seq;
                lru_idx = Some(i);
            }
        }

        // Everything cached belongs to the open transaction. Refusing is the
        // only correct answer: the caller wanted room for one more sector of
        // an operation that is already larger than the cache, and there is no
        // way to make room without breaking write ordering.
        let Some(lru_idx) = lru_idx else {
            return Err(BlockError::OutOfRange);
        };

        if self.entries[lru_idx].dirty {
            let e = &self.entries[lru_idx];
            self.device.write_sector(e.lba, &e.data)?;
        }
        self.entries.swap_remove(lru_idx);
        Ok(())
    }

    fn is_recorded(&self, lba: u64) -> bool {
        match self.recording.as_ref() {
            Some(list) => list.contains(&lba),
            None => false,
        }
    }
}
