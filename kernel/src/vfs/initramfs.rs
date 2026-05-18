// RaCore — Initramfs filesystem driver
//
// A simple read-only in-memory filesystem loaded from the boot initramfs.
// Uses a flat list of files with their data — no actual archive format parsing
// for MVP. Files are registered manually or from a simple table.
//
// The initramfs is the root filesystem during early boot, providing:
// - /sbin/init (RacInit binary)
// - /etc/racinit/ (unit files)
// - /lib/ (shared libraries if needed)

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::inode::{
    DirEntry, FileMode, FileType, InodeMetadata, InodeNum, InodeOps, VfsError, VfsResult,
};
use super::mount::Filesystem;

/// An entry in the initramfs.
struct InitramfsEntry {
    name: String,
    ino: InodeNum,
    file_type: FileType,
    data: &'static [u8],
    children: Vec<InodeNum>, // For directories
}

/// The initramfs filesystem.
pub struct Initramfs {
    entries: Vec<InitramfsEntry>,
}

impl Initramfs {
    /// Create a new empty initramfs.
    pub fn new() -> Self {
        let mut fs = Initramfs {
            entries: Vec::new(),
        };

        // Root directory (inode 0)
        fs.entries.push(InitramfsEntry {
            name: String::from("/"),
            ino: 0,
            file_type: FileType::Directory,
            data: &[],
            children: Vec::new(),
        });

        fs
    }

    /// Add a file to the initramfs at the root level.
    pub fn add_file(&mut self, name: &str, data: &'static [u8]) -> InodeNum {
        let ino = self.entries.len() as InodeNum;
        self.entries.push(InitramfsEntry {
            name: String::from(name),
            ino,
            file_type: FileType::Regular,
            data,
            children: Vec::new(),
        });

        // Add to root directory children
        self.entries[0].children.push(ino);

        crate::serial::serial_println!(
            "[INITRAMFS] Added file '{}' ({} bytes, inode {})",
            name,
            data.len(),
            ino
        );

        ino
    }

    /// Add a directory to the initramfs at the root level.
    pub fn add_dir(&mut self, name: &str) -> InodeNum {
        let ino = self.entries.len() as InodeNum;
        self.entries.push(InitramfsEntry {
            name: String::from(name),
            ino,
            file_type: FileType::Directory,
            data: &[],
            children: Vec::new(),
        });

        self.entries[0].children.push(ino);

        crate::serial::serial_println!(
            "[INITRAMFS] Added directory '{}' (inode {})",
            name,
            ino
        );

        ino
    }

    /// Add a file as child of a directory.
    pub fn add_file_to(&mut self, parent_ino: InodeNum, name: &str, data: &'static [u8]) -> InodeNum {
        let ino = self.entries.len() as InodeNum;
        self.entries.push(InitramfsEntry {
            name: String::from(name),
            ino,
            file_type: FileType::Regular,
            data,
            children: Vec::new(),
        });

        if let Some(parent) = self.entries.get_mut(parent_ino as usize) {
            parent.children.push(ino);
        }

        ino
    }

    /// Add a directory as child of a parent directory.
    pub fn add_dir_to(&mut self, parent_ino: InodeNum, name: &str) -> InodeNum {
        let ino = self.entries.len() as InodeNum;
        self.entries.push(InitramfsEntry {
            name: String::from(name),
            ino,
            file_type: FileType::Directory,
            data: &[],
            children: Vec::new(),
        });
        if let Some(parent) = self.entries.get_mut(parent_ino as usize) {
            parent.children.push(ino);
        }
        ino
    }

    /// Find a direct child of `parent_ino` by name.
    fn find_child(&self, parent_ino: InodeNum, name: &str) -> Option<InodeNum> {
        let parent = self.entries.get(parent_ino as usize)?;
        for &child_ino in &parent.children {
            if let Some(child) = self.entries.get(child_ino as usize) {
                if child.name == name {
                    return Some(child_ino);
                }
            }
        }
        None
    }

    /// Add a file at a potentially nested path (e.g. "sbin/init").
    /// Creates intermediate directories automatically.
    pub fn add_path(&mut self, path: &str, data: &'static [u8]) {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return;
        }

        let mut parent: InodeNum = 0; // root inode

        // Walk / create intermediate directories
        for part in &parts[..parts.len() - 1] {
            parent = match self.find_child(parent, part) {
                Some(ino) => ino,
                None => self.add_dir_to(parent, part),
            };
        }

        // Add the leaf file
        let file_name = parts[parts.len() - 1];
        self.add_file_to(parent, file_name, data);
    }

    /// Parse a binary initramfs image produced by scripts/pack-initramfs.ps1.
    ///
    /// # Safety
    /// `base` must point to valid physical memory that lives for `'static` (bootloader pages).
    pub fn from_binary(base: u64, size: u64) -> Option<Self> {
        if base == 0 || size < 12 {
            return None;
        }

        // SAFETY: The bootloader allocated these pages as LOADER_DATA.
        // After ExitBootServices they belong to us permanently — treat as 'static.
        let data: &'static [u8] = unsafe {
            core::slice::from_raw_parts(base as *const u8, size as usize)
        };

        // Validate magic "RACRAMFS"
        if &data[0..8] != b"RACRAMFS" {
            crate::serial::serial_println!("[INITRAMFS] Binary: invalid magic");
            return None;
        }

        let count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        crate::serial::serial_println!("[INITRAMFS] Binary: {} entries", count);

        let mut fs = Initramfs::new();
        let mut offset = 12usize;

        for i in 0..count {
            // name_len (u16 LE)
            if offset + 2 > data.len() {
                break;
            }
            let name_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;

            // name bytes
            if offset + name_len > data.len() {
                break;
            }
            let name = match core::str::from_utf8(&data[offset..offset + name_len]) {
                Ok(s) => s,
                Err(_) => break,
            };
            offset += name_len;

            // data_len (u32 LE)
            if offset + 4 > data.len() {
                break;
            }
            let data_len = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += 4;

            // file data (points into the loaded initramfs — 'static)
            if offset + data_len > data.len() {
                break;
            }
            let file_data: &'static [u8] = unsafe {
                core::slice::from_raw_parts(data.as_ptr().add(offset), data_len)
            };
            offset += data_len;

            crate::serial::serial_println!(
                "[INITRAMFS]   [{}/{}] {} ({} bytes)",
                i + 1, count, name, data_len
            );
            fs.add_path(name, file_data);
        }

        Some(fs)
    }
} // end impl Initramfs

/// Inode wrapper for initramfs entries.
struct InitramfsInode {
    entry_idx: usize,
    fs: Arc<Initramfs>,
}

impl InodeOps for InitramfsInode {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let entry = &self.fs.entries[self.entry_idx];
        if entry.file_type == FileType::Directory {
            return Err(VfsError::IsADirectory);
        }

        let data = entry.data;
        if offset >= data.len() as u64 {
            return Ok(0); // EOF
        }

        let start = offset as usize;
        let available = data.len() - start;
        let to_read = buf.len().min(available);
        buf[..to_read].copy_from_slice(&data[start..start + to_read]);
        Ok(to_read)
    }

    fn write(&self, _offset: u64, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::PermissionDenied) // Read-only filesystem
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let entry = &self.fs.entries[self.entry_idx];
        let mut meta = InodeMetadata::new(entry.ino, entry.file_type);
        meta.size = entry.data.len() as u64;
        meta.mode = FileMode::new(0o555); // r-xr-xr-x
        Ok(meta)
    }

    fn lookup(&self, name: &str) -> VfsResult<InodeNum> {
        let entry = &self.fs.entries[self.entry_idx];
        if entry.file_type != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }

        for &child_ino in &entry.children {
            if let Some(child) = self.fs.entries.get(child_ino as usize) {
                if child.name == name {
                    return Ok(child.ino);
                }
            }
        }
        Err(VfsError::NotFound)
    }

    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        let entry = &self.fs.entries[self.entry_idx];
        if entry.file_type != FileType::Directory {
            return Err(VfsError::NotADirectory);
        }

        let mut results = Vec::new();
        for &child_ino in &entry.children {
            if let Some(child) = self.fs.entries.get(child_ino as usize) {
                results.push(DirEntry {
                    name: child.name.clone(),
                    ino: child.ino,
                    file_type: child.file_type,
                });
            }
        }
        Ok(results)
    }
}

impl Filesystem for Initramfs {
    fn root_inode(&self) -> Arc<dyn InodeOps> {
        // We need self as Arc — this is a limitation. For MVP, create a new inode wrapper
        // This won't work properly because we can't get Arc<Self> from &self.
        // Instead, let's use a different approach: store data in a global.
        panic!("Use get_inode(0) for root");
    }

    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>> {
        if ino as usize >= self.entries.len() {
            return Err(VfsError::NotFound);
        }
        // This creates a dangling reference issue. We need Arc<Initramfs>.
        // For MVP, we'll use a global initramfs with a static reference.
        Err(VfsError::NotImplemented)
    }

    fn name(&self) -> &str {
        "initramfs"
    }
}

/// Wrapper that holds Arc<Initramfs> for proper Filesystem + InodeOps implementation.
pub struct InitramfsFs {
    inner: Arc<Initramfs>,
}

impl InitramfsFs {
    pub fn new(initramfs: Initramfs) -> Arc<Self> {
        Arc::new(InitramfsFs {
            inner: Arc::new(initramfs),
        })
    }

    pub fn inner(&self) -> &Arc<Initramfs> {
        &self.inner
    }
}

impl Filesystem for InitramfsFs {
    fn root_inode(&self) -> Arc<dyn InodeOps> {
        Arc::new(InitramfsInode {
            entry_idx: 0,
            fs: self.inner.clone(),
        })
    }

    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>> {
        if ino as usize >= self.inner.entries.len() {
            return Err(VfsError::NotFound);
        }
        Ok(Arc::new(InitramfsInode {
            entry_idx: ino as usize,
            fs: self.inner.clone(),
        }))
    }

    fn name(&self) -> &str {
        "initramfs"
    }
}
