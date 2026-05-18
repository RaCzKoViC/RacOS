// RaCore — VFS Inode abstraction
//
// An Inode represents a file system object (file, directory, device, etc.).
// Each filesystem implements the InodeOps trait.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Inode number type.
pub type InodeNum = u64;

/// File type flags (compatible with st_mode in StatBuf).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FileType {
    Regular    = 0o100000,
    Directory  = 0o040000,
    CharDevice = 0o020000,
    BlockDevice= 0o060000,
    Pipe       = 0o010000,
    Socket     = 0o140000,
    Symlink    = 0o120000,
}

/// File permissions.
#[derive(Debug, Clone, Copy)]
pub struct FileMode(pub u32);

impl FileMode {
    pub const fn new(mode: u32) -> Self {
        FileMode(mode & 0o7777)
    }

    pub fn owner_read(&self) -> bool { self.0 & 0o400 != 0 }
    pub fn owner_write(&self) -> bool { self.0 & 0o200 != 0 }
    pub fn owner_exec(&self) -> bool { self.0 & 0o100 != 0 }
}

/// Inode metadata.
#[derive(Debug, Clone)]
pub struct InodeMetadata {
    pub ino: InodeNum,
    pub file_type: FileType,
    pub mode: FileMode,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub nlink: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    /// For device nodes: major number.
    pub dev_major: u32,
    /// For device nodes: minor number.
    pub dev_minor: u32,
}

impl InodeMetadata {
    pub fn new(ino: InodeNum, file_type: FileType) -> Self {
        InodeMetadata {
            ino,
            file_type,
            mode: FileMode::new(0o644),
            uid: 0,
            gid: 0,
            size: 0,
            nlink: 1,
            atime: 0,
            mtime: 0,
            ctime: 0,
            dev_major: 0,
            dev_minor: 0,
        }
    }
}

impl Default for InodeMetadata {
    fn default() -> Self {
        InodeMetadata::new(0, FileType::Regular)
    }
}

/// VFS error type.
#[derive(Debug)]
pub enum VfsError {
    NotFound,
    PermissionDenied,
    NotADirectory,
    IsADirectory,
    AlreadyExists,
    NoSpace,
    IoError,
    NotImplemented,
    InvalidArgument,
    BadFileDescriptor,
    TooManyOpenFiles,
    BrokenPipe,
    WouldBlock,
}

/// Result type for VFS operations.
pub type VfsResult<T> = Result<T, VfsError>;

/// Operations that a filesystem inode must support.
pub trait InodeOps: Send + Sync {
    /// Read data at offset into buffer. Returns bytes read.
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize>;

    /// Write data at offset from buffer. Returns bytes written.
    fn write(&self, offset: u64, buf: &[u8]) -> VfsResult<usize>;

    /// Get inode metadata.
    fn metadata(&self) -> VfsResult<InodeMetadata>;

    /// Update inode metadata fields (mode/uid/gid where supported).
    fn set_metadata(&self, _meta: &InodeMetadata) -> VfsResult<()> {
        Err(VfsError::NotImplemented)
    }

    /// Look up a child entry by name (for directories).
    fn lookup(&self, _name: &str) -> VfsResult<InodeNum> {
        Err(VfsError::NotADirectory)
    }

    /// List directory entries (for directories).
    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        Err(VfsError::NotADirectory)
    }

    /// Device ioctl (for device nodes).
    fn ioctl(&self, _request: u64, _arg: u64) -> VfsResult<i64> {
        Err(VfsError::NotImplemented)
    }

    /// Flush pending file metadata/data to backing storage.
    fn sync(&self) -> VfsResult<()> {
        Err(VfsError::NotImplemented)
    }
}

/// A directory entry.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub ino: InodeNum,
    pub file_type: FileType,
}
