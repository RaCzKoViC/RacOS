// RaCore — Unix Domain Sockets (IPC)
//
// ADR-010: IPC via Sockets.
// - AF_UNIX / AF_LOCAL support.
// - SOCK_STREAM (byte stream) and SOCK_DGRAM (datagram) support.
// - Integrated into VFS via dedicated inodes.
//
// Design:
// Unix domain sockets use the VFS for binding to a file path.
// Once bound, they behave like pipes but with addressable endpoints.

use crate::sync::SpinLock;
use crate::vfs::inode::{
    DirEntry, FileMode, FileType, InodeMetadata, InodeNum, InodeOps, VfsError, VfsResult,
};
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Socket state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Unbound,
    Bound,
    Listening,
    Connected,
    Closed,
}

/// A Unix Domain Socket buffer for stream data.
pub struct SocketBuffer {
    data: VecDeque<u8>,
    capacity: usize,
}

impl SocketBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            data: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn write(&mut self, buf: &[u8]) -> usize {
        let space = self.capacity - self.data.len();
        let to_write = core::cmp::min(buf.len(), space);
        for &b in &buf[..to_write] {
            self.data.push_back(b);
        }
        to_write
    }

    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = core::cmp::min(buf.len(), self.data.len());
        for i in 0..to_read {
            buf[i] = self.data.pop_front().unwrap();
        }
        to_read
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// The internal shared state between two endpoints or a listener/client.
pub struct UnixSocketInternal {
    pub state: SocketState,
    pub receive_queue: SocketBuffer,
    pub peer: Option<Arc<SpinLock<UnixSocketInternal>>>,
    pub listen_backlog: VecDeque<Arc<SpinLock<UnixSocketInternal>>>,
    pub max_backlog: usize,
}

impl UnixSocketInternal {
    pub fn new() -> Self {
        Self {
            state: SocketState::Unbound,
            receive_queue: SocketBuffer::new(65536), // 64KB buffer
            peer: None,
            listen_backlog: VecDeque::new(),
            max_backlog: 0,
        }
    }
}

/// The VFS Inode representaton of a Unix Domain Socket.
pub struct UnixSocketInode {
    pub internal: Arc<SpinLock<UnixSocketInternal>>,
    pub metadata: InodeMetadata,
}

impl UnixSocketInode {
    pub fn new() -> Self {
        Self {
            internal: Arc::new(SpinLock::new(UnixSocketInternal::new())),
            metadata: InodeMetadata {
                ino: 0,
                file_type: FileType::Socket,
                mode: FileMode::new(0o777),
                uid: 0,
                gid: 0,
                size: 0,
                atime: 0,
                mtime: 0,
                ctime: 0,
                nlink: 1,
                dev_major: 0,
                dev_minor: 0,
            },
        }
    }
}

impl InodeOps for UnixSocketInode {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let mut inner = self.internal.lock();

        if inner.state == SocketState::Connected || inner.state == SocketState::Closed {
            if inner.receive_queue.is_empty() && inner.state == SocketState::Closed {
                return Ok(0);
            }
            let n = inner.receive_queue.read(buf);
            return Ok(n);
        }

        Err(VfsError::NotImplemented)
    }

    fn write(&self, _offset: u64, buf: &[u8]) -> VfsResult<usize> {
        let inner = self.internal.lock();

        if inner.state == SocketState::Connected {
            if let Some(ref peer_inner_locked) = inner.peer {
                let mut peer_inner = peer_inner_locked.lock();
                let n = peer_inner.receive_queue.write(buf);
                return Ok(n);
            }
        }

        if inner.state == SocketState::Closed {
            return Err(VfsError::BrokenPipe);
        }

        Err(VfsError::NotImplemented)
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        Ok(self.metadata.clone())
    }
}
