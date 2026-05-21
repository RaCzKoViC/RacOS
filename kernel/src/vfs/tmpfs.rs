// RaCore — tmpfs: In-memory read-write filesystem
//
// Provides a writable filesystem mounted at /tmp (or elsewhere).
// Supports: files (create, read, write, truncate), directories (mkdir, lookup, readdir),
// and removal (unlink, rmdir).
//
// Not persistent — all data lives in kernel heap and is lost on reboot.

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

use super::inode::{
    DirEntry, FileMode, FileType, InodeMetadata, InodeNum, InodeOps, VfsError, VfsResult,
};
use super::mount::Filesystem;

/// Maximum total bytes stored in a single tmpfs instance (16 MiB).
const TMPFS_MAX_SIZE: usize = 16 * 1024 * 1024;

/// A node in the tmpfs tree — either a file or a directory.
struct TmpfsNode {
    name: String,
    ino: InodeNum,
    file_type: FileType,
    mode: FileMode,
    uid: u32,
    gid: u32,
    /// File content (only for Regular).
    data: Vec<u8>,
    /// Child inode numbers (only for Directory).
    children: Vec<InodeNum>,
    /// Whether node is removed (still accessible via ino but unlisted).
    removed: bool,
}

/// The tmpfs filesystem state. Protected by cli/sti (single-CPU).
pub struct Tmpfs {
    nodes: UnsafeCell<Vec<TmpfsNode>>,
    /// Total bytes used by file data.
    total_bytes: UnsafeCell<usize>,
}

// SAFETY: Protected by cli/sti — no concurrent access on single CPU.
unsafe impl Send for Tmpfs {}
unsafe impl Sync for Tmpfs {}

impl Tmpfs {
    /// Create a new empty tmpfs with a root directory.
    pub fn new() -> Arc<Self> {
        let mut nodes = Vec::new();
        // Root directory — inode 0
        nodes.push(TmpfsNode {
            name: String::from("/"),
            ino: 0,
            file_type: FileType::Directory,
            mode: FileMode::new(0o755),
            uid: 0,
            gid: 0,
            data: Vec::new(),
            children: Vec::new(),
            removed: false,
        });

        Arc::new(Tmpfs {
            nodes: UnsafeCell::new(nodes),
            total_bytes: UnsafeCell::new(0),
        })
    }

    fn nodes(&self) -> &Vec<TmpfsNode> {
        unsafe { &*self.nodes.get() }
    }

    fn nodes_mut(&self) -> &mut Vec<TmpfsNode> {
        unsafe { &mut *self.nodes.get() }
    }

    fn total_bytes(&self) -> usize {
        unsafe { *self.total_bytes.get() }
    }

    fn set_total_bytes(&self, v: usize) {
        unsafe { *self.total_bytes.get() = v; }
    }

    /// Allocate a new inode.
    fn alloc_ino(&self) -> InodeNum {
        let nodes = self.nodes();
        nodes.len() as InodeNum
    }

    /// Create a file in a parent directory. Returns the new inode number.
    pub fn create_file(&self, parent_ino: InodeNum, name: &str) -> VfsResult<InodeNum> {
        let nodes = self.nodes_mut();
        let parent = nodes.get(parent_ino as usize).ok_or(VfsError::NotFound)?;
        if parent.file_type != FileType::Directory || parent.removed {
            return Err(VfsError::NotADirectory);
        }
        // Check for duplicate
        for &child_ino in &parent.children {
            if let Some(child) = nodes.get(child_ino as usize) {
                if !child.removed && child.name == name {
                    return Err(VfsError::AlreadyExists);
                }
            }
        }

        let ino = self.alloc_ino();
        nodes.push(TmpfsNode {
            name: String::from(name),
            ino,
            file_type: FileType::Regular,
            mode: FileMode::new(0o644),
            uid: 0,
            gid: 0,
            data: Vec::new(),
            children: Vec::new(),
            removed: false,
        });

        // Add to parent's children
        let parent = nodes.get_mut(parent_ino as usize).ok_or(VfsError::NotFound)?;
        parent.children.push(ino);
        Ok(ino)
    }

    /// Create a directory in a parent directory.
    pub fn create_dir(&self, parent_ino: InodeNum, name: &str) -> VfsResult<InodeNum> {
        let nodes = self.nodes_mut();
        let parent = nodes.get(parent_ino as usize).ok_or(VfsError::NotFound)?;
        if parent.file_type != FileType::Directory || parent.removed {
            return Err(VfsError::NotADirectory);
        }
        // Check for duplicate
        for &child_ino in &parent.children {
            if let Some(child) = nodes.get(child_ino as usize) {
                if !child.removed && child.name == name {
                    return Err(VfsError::AlreadyExists);
                }
            }
        }

        let ino = self.alloc_ino();
        nodes.push(TmpfsNode {
            name: String::from(name),
            ino,
            file_type: FileType::Directory,
            mode: FileMode::new(0o755),
            uid: 0,
            gid: 0,
            data: Vec::new(),
            children: Vec::new(),
            removed: false,
        });

        let parent = nodes.get_mut(parent_ino as usize).ok_or(VfsError::NotFound)?;
        parent.children.push(ino);
        Ok(ino)
    }

    /// Remove a file or empty directory from a parent.
    pub fn unlink(&self, parent_ino: InodeNum, name: &str) -> VfsResult<()> {
        let nodes = self.nodes_mut();
        let parent = nodes.get(parent_ino as usize).ok_or(VfsError::NotFound)?;
        if parent.file_type != FileType::Directory || parent.removed {
            return Err(VfsError::NotADirectory);
        }

        // Find child
        let mut child_idx_in_parent = None;
        let mut child_ino = 0u64;
        for (i, &cino) in parent.children.iter().enumerate() {
            if let Some(child) = nodes.get(cino as usize) {
                if !child.removed && child.name == name {
                    child_idx_in_parent = Some(i);
                    child_ino = cino;
                    break;
                }
            }
        }
        let idx = child_idx_in_parent.ok_or(VfsError::NotFound)?;

        // If directory, check it's empty
        let child = nodes.get(child_ino as usize).ok_or(VfsError::NotFound)?;
        if child.file_type == FileType::Directory {
            let active_children = child.children.iter()
                .filter(|&&c| nodes.get(c as usize).map(|n| !n.removed).unwrap_or(false))
                .count();
            if active_children > 0 {
                return Err(VfsError::InvalidArgument); // ENOTEMPTY
            }
        }

        // Free data
        let data_size = nodes[child_ino as usize].data.len();
        self.set_total_bytes(self.total_bytes().saturating_sub(data_size));
        nodes[child_ino as usize].data = Vec::new();
        nodes[child_ino as usize].removed = true;

        // Remove from parent's children list
        let parent = nodes.get_mut(parent_ino as usize).ok_or(VfsError::NotFound)?;
        parent.children.remove(idx);
        Ok(())
    }

    /// Look up a path relative to the tmpfs root. Returns (parent_ino, leaf_ino).
    /// If the path has only one component (e.g., "foo"), returns (0, ino_of_foo).
    pub fn lookup_path(&self, path: &str) -> VfsResult<InodeNum> {
        let nodes = self.nodes();
        let mut current: InodeNum = 0; // root

        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            let node = nodes.get(current as usize).ok_or(VfsError::NotFound)?;
            if node.file_type != FileType::Directory || node.removed {
                return Err(VfsError::NotADirectory);
            }
            let mut found = false;
            for &child_ino in &node.children {
                if let Some(child) = nodes.get(child_ino as usize) {
                    if !child.removed && child.name == component {
                        current = child_ino;
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                return Err(VfsError::NotFound);
            }
        }
        Ok(current)
    }

    /// Split a path into (parent_path_components, leaf_name).
    pub fn split_parent_leaf<'a>(&self, path: &'a str) -> VfsResult<(InodeNum, &'a str)> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Err(VfsError::InvalidArgument);
        }
        let leaf = parts[parts.len() - 1];
        let mut parent_ino: InodeNum = 0;
        let nodes = self.nodes();
        for &part in &parts[..parts.len() - 1] {
            let node = nodes.get(parent_ino as usize).ok_or(VfsError::NotFound)?;
            if node.file_type != FileType::Directory || node.removed {
                return Err(VfsError::NotADirectory);
            }
            let mut found = false;
            for &child_ino in &node.children {
                if let Some(child) = nodes.get(child_ino as usize) {
                    if !child.removed && child.name == part {
                        parent_ino = child_ino;
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                return Err(VfsError::NotFound);
            }
        }
        Ok((parent_ino, leaf))
    }
}

/// Inode wrapper for a tmpfs node.
struct TmpfsInode {
    ino: InodeNum,
    fs: Arc<Tmpfs>,
}

impl InodeOps for TmpfsInode {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let nodes = self.fs.nodes();
        let node = nodes.get(self.ino as usize).ok_or(VfsError::NotFound)?;
        if node.removed {
            return Err(VfsError::NotFound);
        }
        if node.file_type == FileType::Directory {
            return Err(VfsError::IsADirectory);
        }
        let data = &node.data;
        let off = offset as usize;
        if off >= data.len() {
            return Ok(0); // EOF
        }
        let avail = data.len() - off;
        let to_read = buf.len().min(avail);
        buf[..to_read].copy_from_slice(&data[off..off + to_read]);
        Ok(to_read)
    }

    fn write(&self, offset: u64, buf: &[u8]) -> VfsResult<usize> {
        let nodes = self.fs.nodes_mut();
        let node = nodes.get_mut(self.ino as usize).ok_or(VfsError::NotFound)?;
        if node.removed {
            return Err(VfsError::NotFound);
        }
        if node.file_type == FileType::Directory {
            return Err(VfsError::IsADirectory);
        }

        let off = offset as usize;
        let end = off + buf.len();

        // Check total size limit
        let old_len = node.data.len();
        let new_len = end.max(old_len);
        let growth = new_len.saturating_sub(old_len);
        let total = self.fs.total_bytes();
        if total + growth > TMPFS_MAX_SIZE {
            return Err(VfsError::NoSpace);
        }

        // Extend data if needed
        if end > node.data.len() {
            node.data.resize(end, 0);
        }
        node.data[off..end].copy_from_slice(buf);
        self.fs.set_total_bytes(total + growth);
        Ok(buf.len())
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let nodes = self.fs.nodes();
        let node = nodes.get(self.ino as usize).ok_or(VfsError::NotFound)?;
        let mut meta = InodeMetadata::new(node.ino, node.file_type);
        meta.mode = node.mode;
        meta.uid = node.uid;
        meta.gid = node.gid;
        meta.size = node.data.len() as u64;
        if node.file_type == FileType::Directory {
            meta.nlink = 2;
        }
        Ok(meta)
    }

    fn set_metadata(&self, meta: &InodeMetadata) -> VfsResult<()> {
        let nodes = self.fs.nodes_mut();
        let node = nodes.get_mut(self.ino as usize).ok_or(VfsError::NotFound)?;
        if node.removed {
            return Err(VfsError::NotFound);
        }
        node.mode = FileMode::new(meta.mode.0);
        node.uid = meta.uid;
        node.gid = meta.gid;
        Ok(())
    }

    fn lookup(&self, name: &str) -> VfsResult<InodeNum> {
        let nodes = self.fs.nodes();
        let node = nodes.get(self.ino as usize).ok_or(VfsError::NotFound)?;
        if node.file_type != FileType::Directory || node.removed {
            return Err(VfsError::NotADirectory);
        }
        for &child_ino in &node.children {
            if let Some(child) = nodes.get(child_ino as usize) {
                if !child.removed && child.name == name {
                    return Ok(child_ino);
                }
            }
        }
        Err(VfsError::NotFound)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        let nodes = self.fs.nodes();
        let node = nodes.get(self.ino as usize).ok_or(VfsError::NotFound)?;
        if node.file_type != FileType::Directory || node.removed {
            return Err(VfsError::NotADirectory);
        }
        let mut entries = Vec::new();
        for &child_ino in &node.children {
            if let Some(child) = nodes.get(child_ino as usize) {
                if !child.removed {
                    entries.push(DirEntry {
                        name: child.name.clone(),
                        ino: child_ino,
                        file_type: child.file_type,
                    });
                }
            }
        }
        Ok(entries)
    }

    fn ioctl(&self, _request: u64, _arg: u64) -> VfsResult<i64> {
        Err(VfsError::NotImplemented)
    }
}

/// Filesystem adapter for the mount table.
pub struct TmpfsFilesystem {
    inner: Arc<Tmpfs>,
}

impl TmpfsFilesystem {
    pub fn new(tmpfs: Arc<Tmpfs>) -> Arc<Self> {
        Arc::new(TmpfsFilesystem { inner: tmpfs })
    }

    pub fn inner(&self) -> Arc<Tmpfs> {
        self.inner.clone()
    }
}

impl Filesystem for TmpfsFilesystem {
    fn root_inode(&self) -> Arc<dyn InodeOps> {
        Arc::new(TmpfsInode { ino: 0, fs: self.inner.clone() })
    }

    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>> {
        let nodes = self.inner.nodes();
        if (ino as usize) >= nodes.len() {
            return Err(VfsError::NotFound);
        }
        if nodes[ino as usize].removed {
            return Err(VfsError::NotFound);
        }
        Ok(Arc::new(TmpfsInode { ino, fs: self.inner.clone() }))
    }

    fn name(&self) -> &str {
        "tmpfs"
    }

    fn as_any(&self) -> &dyn core::any::Any { self }
}

/// Global tmpfs instance for /tmp.
static mut TMPFS_INSTANCE: Option<Arc<Tmpfs>> = None;

/// Initialize and return the global tmpfs instance.
///
/// # Safety
/// Must be called once during kernel init with interrupts disabled.
pub unsafe fn init() -> Arc<Tmpfs> {
    let tmpfs = Tmpfs::new();
    let inst = &mut *core::ptr::addr_of_mut!(TMPFS_INSTANCE);
    *inst = Some(tmpfs.clone());
    crate::serial::serial_println!("[  0.000350] RACORE: tmpfs initialized (max {} KiB)", TMPFS_MAX_SIZE / 1024);
    tmpfs
}

/// Get the global tmpfs instance.
///
/// # Safety
/// Must be called after init().
pub unsafe fn instance() -> &'static Arc<Tmpfs> {
    (*core::ptr::addr_of!(TMPFS_INSTANCE)).as_ref().unwrap()
}
