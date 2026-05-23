// RaCore — Virtual File System (VFS) layer
//
// ADR-009: VFS abstracts filesystem implementations from the kernel and user space.
// All file operations go through VFS → filesystem driver.
//
// Core abstractions:
// - Inode: metadata + ops for a file/directory
// - File: open file state (offset, operations)
// - FileDescriptor: per-process fd table entry
// - MountPoint: filesystem mount tracking
// - Dentry: directory entry cache

pub mod devfs;
pub mod fat32;
pub mod file;
pub mod initramfs;
pub mod inode;
pub mod mount;
pub mod pipe;
pub mod procfs;
pub mod racfs;
pub mod socket;
pub mod tmpfs;
