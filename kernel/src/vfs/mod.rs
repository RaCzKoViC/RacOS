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

pub mod socket;
pub mod fat32;
pub mod inode;
pub mod file;
pub mod mount;
pub mod procfs;
pub mod devfs;
pub mod initramfs;
pub mod pipe;
pub mod tmpfs;
pub mod racfs;
