// RaCore — VFS Mount table
//
// Tracks filesystem mounts and routes path lookups to the correct filesystem.

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::inode::{DirEntry, InodeNum, InodeOps, VfsError, VfsResult};

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
        MountTable { mounts: Vec::new() }
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
        crate::serial::serial_println!("[   VFS   ] Mounting '{}' at {}", fs.name(), path);
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
            crate::serial::serial_println!("[   VFS   ] Unmounted '{}' from {}", fs_name, path);
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

    /// The `st_dev` a file on `fs` reports: the mount's 1-based position in
    /// this table, 0 if `fs` is not mounted (pipes, sockets).
    ///
    /// Together with the inode number this is what identifies a file:
    /// tmpfs inode 5 and racfs inode 5 are different files, and two names of
    /// one file on one mount agree on both numbers. Stable for the life of
    /// a boot, which is what `mv`/`cp` need to refuse to copy a file onto
    /// itself; it is not a persistent device number and is not meant as one.
    pub fn device_id(&self, fs: &Arc<dyn Filesystem>) -> u64 {
        let want = Arc::as_ptr(fs) as *const () as usize;
        self.mounts
            .iter()
            .position(|m| Arc::as_ptr(&m.fs) as *const () as usize == want)
            .map(|i| i as u64 + 1)
            .unwrap_or(0)
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
            if racfs_fs.inner().sync().is_ok() {
                count += 1;
            }
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
                path.starts_with(mpath) && path.as_bytes().get(mpath.len()).copied() == Some(b'/')
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

    /// Look up a path, walking through mount points and directories,
    /// without asking whether the caller may traverse them. This is the
    /// kernel's own lookup: boot, /dev/console, fsck. Syscalls on behalf
    /// of a process use `lookup_path_as`.
    pub fn lookup_path(&self, path: &str) -> VfsResult<(Arc<dyn Filesystem>, InodeNum)> {
        self.walk(path, None)
    }

    /// Look up a path as `creds` would see it: every directory walked
    /// through must grant that caller search (execute) permission.
    ///
    /// Without this a file's own mode was the only thing protecting it -
    /// a 0666 file inside a 0700 directory was readable by anyone who
    /// knew its name, which is exactly what a private directory is meant
    /// to prevent.
    ///
    /// What is checked: the root of the filesystem the path resolves into,
    /// and each directory below it on the way to the last component. What
    /// is not, and cannot be: the components ABOVE a mount point. The
    /// resolver jumps straight to the deepest mount, and a mount point
    /// need not exist in the filesystem underneath - /tmp, /dev, /proc,
    /// /mnt, /var and /fat have no directory of their own in the
    /// initramfs. Making mount points real directories is its own change;
    /// until then a path's prefix above a mount is not a barrier.
    pub fn lookup_path_as(
        &self,
        path: &str,
        creds: &crate::task::task::Credentials,
    ) -> VfsResult<(Arc<dyn Filesystem>, InodeNum)> {
        self.walk(path, Some(creds))
    }

    fn walk(
        &self,
        path: &str,
        creds: Option<&crate::task::task::Credentials>,
    ) -> VfsResult<(Arc<dyn Filesystem>, InodeNum)> {
        let (mount, remainder) = self.resolve(path).ok_or(VfsError::NotFound)?;
        let fs = &mount.fs;

        if remainder.is_empty() {
            // Root of the mounted filesystem
            let root = fs.root_inode();
            let meta = root.metadata()?;
            return Ok((fs.clone(), meta.ino));
        }

        // Walk the path components. Every directory entered on the way -
        // the filesystem root included - has to grant search permission;
        // the last component is the target and is judged by the caller.
        //
        // A caller holding CAP_DAC_OVERRIDE passes every directory whatever
        // its mode, so the answer is settled before the walk starts and the
        // metadata read per component is skipped: on a disk-backed
        // filesystem that read is the expensive part, and today every
        // process still runs as root.
        let checking = creds.filter(|c| {
            !crate::security::capability::has_cap(c, crate::security::capability::CAP_DAC_OVERRIDE)
        });
        let mut current_inode = fs.root_inode();
        let mut rest = remainder.split('/').filter(|c| !c.is_empty() && *c != ".");
        let mut pending = rest.next();
        while let Some(component) = pending {
            let next = rest.next();
            if let Some(creds) = checking {
                let dir_meta = current_inode.metadata()?;
                if !crate::security::dac::can_access(
                    creds,
                    &dir_meta,
                    crate::security::dac::Access::Execute,
                ) {
                    return Err(VfsError::PermissionDenied);
                }
            }
            let ino = current_inode.lookup(component)?;
            current_inode = fs.get_inode(ino)?;
            pending = next;
        }

        let meta = current_inode.metadata()?;
        Ok((fs.clone(), meta.ino))
    }

    /// Search permission for every directory of `path` up to and including
    /// the one that will hold its last component - the check a creating or
    /// removing syscall needs before it reaches for the parent directly
    /// (`split_parent_leaf` walks the filesystem on its own).
    ///
    /// A path whose parent does not exist is left to the caller's own
    /// error handling: this answers about permission, not existence.
    pub fn require_search_to_parent(
        &self,
        path: &str,
        creds: &crate::task::task::Credentials,
    ) -> VfsResult<()> {
        // As in `walk`: CAP_DAC_OVERRIDE settles it without reading a
        // single directory's metadata.
        if crate::security::capability::has_cap(
            creds,
            crate::security::capability::CAP_DAC_OVERRIDE,
        ) {
            return Ok(());
        }
        let (mount, remainder) = self.resolve(path).ok_or(VfsError::NotFound)?;
        let fs = &mount.fs;
        let mut current_inode = fs.root_inode();
        let components: alloc::vec::Vec<&str> = remainder
            .split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .collect();
        let parents = components.len().saturating_sub(1);
        for component in components.iter().take(parents) {
            let dir_meta = current_inode.metadata()?;
            if !crate::security::dac::can_access(
                creds,
                &dir_meta,
                crate::security::dac::Access::Execute,
            ) {
                return Err(VfsError::PermissionDenied);
            }
            let ino = current_inode.lookup(component)?;
            current_inode = fs.get_inode(ino)?;
        }
        let dir_meta = current_inode.metadata()?;
        if !crate::security::dac::can_access(
            creds,
            &dir_meta,
            crate::security::dac::Access::Execute,
        ) {
            return Err(VfsError::PermissionDenied);
        }
        Ok(())
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
