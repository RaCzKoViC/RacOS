// RaCore — VFS File and FileDescriptor abstractions
//
// File: an open file with position tracking.
// FileDescriptor: per-process fd table entry.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::inode::{InodeOps, VfsResult, VfsError, InodeNum};

/// Open file flags (KERNEL_ABI.md §6.2).
pub mod flags {
    pub const O_RDONLY: u32 = 0x0000;
    pub const O_WRONLY: u32 = 0x0001;
    pub const O_RDWR: u32   = 0x0002;
    pub const O_CREAT: u32  = 0x0040;
    pub const O_TRUNC: u32  = 0x0200;
    pub const O_APPEND: u32 = 0x0400;

    pub const ACCESS_MODE_MASK: u32 = 0x0003;
}

/// An open file.
pub struct OpenFile {
    pub inode_num: InodeNum,
    pub inode: Arc<dyn InodeOps>,
    pub flags: u32,
    pub offset: AtomicU64,
}

impl OpenFile {
    pub fn new(inode_num: InodeNum, inode: Arc<dyn InodeOps>, flags: u32) -> Self {
        OpenFile {
            inode_num,
            inode,
            flags,
            offset: AtomicU64::new(0),
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> VfsResult<usize> {
        let access = self.flags & flags::ACCESS_MODE_MASK;
        if access == flags::O_WRONLY {
            return Err(VfsError::PermissionDenied);
        }

        let off = self.offset.load(Ordering::Relaxed);
        let n = self.inode.read(off, buf)?;
        self.offset.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    pub fn write(&self, buf: &[u8]) -> VfsResult<usize> {
        let access = self.flags & flags::ACCESS_MODE_MASK;
        if access == flags::O_RDONLY {
            return Err(VfsError::PermissionDenied);
        }

        let off = if self.flags & flags::O_APPEND != 0 {
            // For append, get current size
            let meta = self.inode.metadata()?;
            meta.size
        } else {
            self.offset.load(Ordering::Relaxed)
        };

        let n = self.inode.write(off, buf)?;
        self.offset.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Maximum file descriptors per process.
pub const MAX_FDS: usize = 256;

/// Per-process file descriptor table.
pub struct FdTable {
    entries: Vec<Option<Arc<OpenFile>>>,
}

impl Clone for FdTable {
    fn clone(&self) -> Self {
        FdTable {
            entries: self.entries.clone(),
        }
    }
}

impl FdTable {
    pub fn new() -> Self {
        let mut entries = Vec::with_capacity(MAX_FDS);
        for _ in 0..MAX_FDS {
            entries.push(None);
        }
        FdTable { entries }
    }

    /// Allocate the lowest available fd.
    pub fn alloc(&mut self, file: Arc<OpenFile>) -> VfsResult<i32> {
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(file);
                return Ok(i as i32);
            }
        }
        Err(VfsError::TooManyOpenFiles)
    }

    /// Get a file by fd.
    pub fn get(&self, fd: i32) -> VfsResult<Arc<OpenFile>> {
        if fd < 0 || fd as usize >= self.entries.len() {
            return Err(VfsError::BadFileDescriptor);
        }
        self.entries[fd as usize]
            .as_ref()
            .cloned()
            .ok_or(VfsError::BadFileDescriptor)
    }

    /// Close a fd.
    pub fn close(&mut self, fd: i32) -> VfsResult<()> {
        if fd < 0 || fd as usize >= self.entries.len() {
            return Err(VfsError::BadFileDescriptor);
        }
        if self.entries[fd as usize].is_none() {
            return Err(VfsError::BadFileDescriptor);
        }
        self.entries[fd as usize] = None;
        Ok(())
    }

    /// Close every open descriptor. Each `OpenFile` Arc is dropped; pipes and
    /// inodes see their refcount drop, so consumers observe EOF promptly.
    pub fn close_all(&mut self) {
        for slot in self.entries.iter_mut() {
            *slot = None;
        }
    }

    /// Duplicate a fd.
    pub fn dup(&mut self, oldfd: i32) -> VfsResult<i32> {
        let file = self.get(oldfd)?;
        self.alloc(file)
    }

    /// Duplicate a fd to a specific number.
    pub fn dup2(&mut self, oldfd: i32, newfd: i32) -> VfsResult<i32> {
        if newfd < 0 || newfd as usize >= self.entries.len() {
            return Err(VfsError::BadFileDescriptor);
        }
        let file = self.get(oldfd)?;
        // Close newfd if open (ignore error)
        let _ = self.close(newfd);
        self.entries[newfd as usize] = Some(file);
        Ok(newfd)
    }

    /// Clone all open FDs from self into another FdTable.
    pub fn clone_into(&self, other: &mut FdTable) {
        for (i, entry) in self.entries.iter().enumerate() {
            if i < other.entries.len() {
                other.entries[i] = entry.clone();
            }
        }
    }

    /// Create an independent clone of this FD table (for fork).
    pub fn clone_fds(&self) -> FdTable {
        let mut new = FdTable::new();
        self.clone_into(&mut new);
        new
    }
}
