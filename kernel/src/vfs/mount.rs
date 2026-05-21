// RaCore — VFS Mount table
//
// Tracks filesystem mounts and routes path lookups to the correct filesystem.

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::inode::{InodeOps, InodeNum, VfsResult, VfsError, DirEntry};

/// A filesystem driver that can provide inodes.
pub trait Filesystem: Send + Sync + 'static {
    /// Get the root inode of this filesystem.
    fn root_inode(&self) -> Arc<dyn InodeOps>;

    /// Get an inode by number.
    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>>;

    /// Filesystem name (e.g., "initramfs", "devfs", "tmpfs").
    fn name(&self) -> &str;

    /// Downcast hook so syscall handlers can reach the concrete writable
    /// backing store of a mount (instead of looking it up by name in a
    /// global singleton, which mixes up multiple mounts of the same FS).
    fn as_any(&self) -> &dyn core::any::Any;
}

/// A mount point entry.
pub struct MountEntry {
    pub path: String,
    pub fs: Arc<dyn Filesystem>,
}

/// Global mount table.
pub struct MountTable {
    mounts: Vec<MountEntry>,
}

impl MountTable {
    pub fn new() -> Self {
        MountTable {
            mounts: Vec::new(),
        }
    }

    /// Mount a filesystem at the given path.
    pub fn mount(&mut self, path: &str, fs: Arc<dyn Filesystem>) {
        // Replace existing mount at the same path.
        if let Some(existing) = self.mounts.iter_mut().find(|m| m.path == path) {
            crate::serial::serial_println!(
                "[   VFS   ] Replacing mount at {} with '{}'",
                path,
                fs.name()
            );
            existing.fs = fs;
            return;
        }
        crate::serial::serial_println!(
            "[   VFS   ] Mounting '{}' at {}",
            fs.name(),
            path
        );
        self.mounts.push(MountEntry {
            path: String::from(path),
            fs,
        });
    }

    /// Unmount filesystem at an exact mount path.
    pub fn umount(&mut self, path: &str) -> VfsResult<()> {
        if path == "/" {
            return Err(VfsError::InvalidArgument);
        }
        if let Some(idx) = self.mounts.iter().position(|m| m.path == path) {
            let fs_name = String::from(self.mounts[idx].fs.name());
            self.mounts.remove(idx);
            crate::serial::serial_println!(
                "[   VFS   ] Unmounted '{}' from {}",
                fs_name,
                path
            );
            Ok(())
        } else {
            Err(VfsError::NotFound)
        }
    }

    /// Check whether a mount exists at the exact path.
    pub fn is_mounted(&self, path: &str) -> bool {
        self.mounts.iter().any(|m| m.path == path)
    }

    /// Snapshot of all active mount entries (for /proc/mounts).
    pub fn entries(&self) -> &[MountEntry] {
        &self.mounts
    }
}

/// Flush every block-backed mount in the global mount table.
/// Returns the number of mounts successfully synced. Errors are swallowed
/// because partial progress is still useful for crash safety.
///
/// # Safety
/// Caller must ensure the global mount table has been initialised.
pub unsafe fn flush_all() -> usize {
    let mt = mount_table();
    let mut count = 0;
    for entry in mt.mounts.iter() {
        let any = entry.fs.as_any();
        if let Some(racfs_fs) = any.downcast_ref::<super::racfs::RacfsFilesystem>() {
            if racfs_fs.inner().sync().is_ok() { count += 1; }
        }
    }
    count
}

impl MountTable {
    // Re-open the impl block so subsequent methods (if any) compile.
    #[allow(dead_code)]
    fn _flush_stub(&self) {}

    /// Resolve a path to a filesystem and relative path.
    /// Returns the longest-prefix matching mount and the remainder of the path.
    pub fn resolve<'a, 'b>(&'a self, path: &'b str) -> Option<(&'a MountEntry, &'b str)> {
        let mut best_idx: Option<(usize, usize)> = None; // (mount_len, mount_index)

        for (i, entry) in self.mounts.iter().enumerate() {
            let mpath = entry.path.as_str();
            let is_match = if mpath == "/" {
                path.starts_with('/')
            } else if path == mpath {
                true
            } else {
                path.starts_with(mpath)
                    && path.as_bytes().get(mpath.len()).copied() == Some(b'/')
            };
            if is_match {
                let len = entry.path.len();
                match best_idx {
                    Some((best_len, _)) if len > best_len => {
                        best_idx = Some((len, i));
                    }
                    None => {
                        best_idx = Some((len, i));
                    }
                    _ => {}
                }
            }
        }

        best_idx.map(|(len, idx)| {
            let remainder = &path[len..];
            let remainder = if remainder.starts_with('/') {
                &remainder[1..]
            } else {
                remainder
            };
            (&self.mounts[idx], remainder)
        })
    }

    /// Look up a path, walking through mount points and directories.
    pub fn lookup_path(&self, path: &str) -> VfsResult<(Arc<dyn Filesystem>, InodeNum)> {
        let (mount, remainder) = self.resolve(path).ok_or(VfsError::NotFound)?;
        let fs = &mount.fs;

        if remainder.is_empty() {
            // Root of the mounted filesystem
            let root = fs.root_inode();
            let meta = root.metadata()?;
            return Ok((fs.clone(), meta.ino));
        }

        // Walk the path components
        let mut current_inode = fs.root_inode();
        for component in remainder.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            let ino = current_inode.lookup(component)?;
            current_inode = fs.get_inode(ino)?;
        }

        let meta = current_inode.metadata()?;
        Ok((fs.clone(), meta.ino))
    }
}

/// Global mount table (protected by CLI/STI for MVP).
static mut MOUNT_TABLE: Option<MountTable> = None;

/// Initialize the global mount table.
///
/// # Safety
/// Must be called once with interrupts disabled.
pub unsafe fn init() {
    let mt = &mut *core::ptr::addr_of_mut!(MOUNT_TABLE);
    *mt = Some(MountTable::new());
    crate::serial::serial_println!("[  0.000250] RACORE: VFS mount table initialized");
}

/// Get a mutable reference to the mount table.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn mount_table() -> &'static mut MountTable {
    (*core::ptr::addr_of_mut!(MOUNT_TABLE)).as_mut().unwrap()
}
