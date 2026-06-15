// RaCore — Anonymous pipe (in-kernel ring buffer)
//
// A pipe consists of a shared ring buffer protected by a simple spinlock-free
// design (UP kernel — no SMP needed for MVP).
//
// Design:
// - 4 KiB ring buffer per pipe
// - Two InodeOps adaptors: PipeReadEnd / PipeWriteEnd share one PipeBuffer via Arc<Mutex>
// - read() blocks (via scheduler) when empty; write() returns EAGAIN when full

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use super::inode::{FileType, InodeMetadata, InodeNum, InodeOps, VfsError, VfsResult};

/// Ring buffer capacity (4 KiB).
const PIPE_CAPACITY: usize = 4096;

/// Shared pipe state.
pub struct PipeBuffer {
    buf: [u8; PIPE_CAPACITY],
    /// Read position (consume from here).
    read_pos: usize,
    /// Write position (produce here).
    write_pos: usize,
    /// Number of bytes currently in the buffer.
    len: usize,
    /// True when all write ends have been dropped.
    write_closed: AtomicBool,
    /// True when all read ends have been dropped.
    read_closed: AtomicBool,
}

impl PipeBuffer {
    pub const fn new() -> Self {
        PipeBuffer {
            buf: [0u8; PIPE_CAPACITY],
            read_pos: 0,
            write_pos: 0,
            len: 0,
            write_closed: AtomicBool::new(false),
            read_closed: AtomicBool::new(false),
        }
    }

    /// Available readable bytes.
    pub fn readable(&self) -> usize {
        self.len
    }

    /// Available writable space.
    pub fn writable(&self) -> usize {
        PIPE_CAPACITY - self.len
    }

    /// Write bytes. Returns bytes consumed from `data`.
    pub fn write_bytes(&mut self, data: &[u8]) -> usize {
        let n = data.len().min(self.writable());
        for i in 0..n {
            self.buf[self.write_pos] = data[i];
            self.write_pos = (self.write_pos + 1) % PIPE_CAPACITY;
        }
        self.len += n;
        n
    }

    /// Read bytes into `buf`. Returns bytes produced into `buf`.
    pub fn read_bytes(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.readable());
        for i in 0..n {
            buf[i] = self.buf[self.read_pos];
            self.read_pos = (self.read_pos + 1) % PIPE_CAPACITY;
        }
        self.len -= n;
        n
    }
}

// SAFETY: UP kernel — no concurrent access; PipeBuffer is only accessed with
// interrupts disabled (single CPU, no SMP).
unsafe impl Send for PipeBuffer {}
unsafe impl Sync for PipeBuffer {}

/// Newtype around a raw pointer so we can share it between two InodeOps impls.
/// The Arc<PipeShared> keeps the buffer alive.
pub struct PipeShared(core::cell::UnsafeCell<PipeBuffer>);

impl PipeShared {
    pub fn new() -> Arc<Self> {
        Arc::new(PipeShared(core::cell::UnsafeCell::new(PipeBuffer::new())))
    }

    /// SAFETY: caller must ensure UP + interrupts disabled.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_mut(&self) -> &mut PipeBuffer {
        &mut *self.0.get()
    }
}

// SAFETY: UP kernel.
unsafe impl Send for PipeShared {}
unsafe impl Sync for PipeShared {}

// ─── Read end ──────────────────────────────────────────────────────────────

pub struct PipeReadEnd {
    shared: Arc<PipeShared>,
    ino: InodeNum,
}

impl Drop for PipeReadEnd {
    fn drop(&mut self) {
        unsafe {
            self.shared
                .get_mut()
                .read_closed
                .store(true, Ordering::Relaxed);
        }
    }
}

impl InodeOps for PipeReadEnd {
    fn read(&self, _offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            // SAFETY: UP, interrupts disabled by scheduler during syscall context.
            let n = unsafe {
                let pb = self.shared.get_mut();
                if pb.readable() > 0 {
                    pb.read_bytes(buf)
                } else if pb.write_closed.load(Ordering::Relaxed) {
                    return Ok(0); // EOF
                } else {
                    0
                }
            };
            if n > 0 {
                return Ok(n);
            }
            // No data yet — yield until woken.
            crate::task::scheduler::yield_now();
        }
    }

    fn write(&self, _offset: u64, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::PermissionDenied)
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let mut m = InodeMetadata::default();
        m.ino = self.ino;
        m.file_type = FileType::Regular;
        Ok(m)
    }
}

// ─── Write end ─────────────────────────────────────────────────────────────

pub struct PipeWriteEnd {
    shared: Arc<PipeShared>,
    ino: InodeNum,
}

impl Drop for PipeWriteEnd {
    fn drop(&mut self) {
        unsafe {
            self.shared
                .get_mut()
                .write_closed
                .store(true, Ordering::Relaxed);
        }
    }
}

impl InodeOps for PipeWriteEnd {
    fn read(&self, _offset: u64, _buf: &mut [u8]) -> VfsResult<usize> {
        Err(VfsError::PermissionDenied)
    }

    fn write(&self, _offset: u64, buf: &[u8]) -> VfsResult<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // SAFETY: UP.
        unsafe {
            let pb = self.shared.get_mut();
            if pb.read_closed.load(Ordering::Relaxed) {
                return Err(VfsError::BrokenPipe);
            }
            if pb.writable() == 0 {
                return Err(VfsError::WouldBlock);
            }
            Ok(pb.write_bytes(buf))
        }
    }

    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let mut m = InodeMetadata::default();
        m.ino = self.ino;
        m.file_type = FileType::Regular;
        Ok(m)
    }
}

/// Create a new anonymous pipe.
/// Returns `(read_inode, write_inode)`.
pub fn create_pipe() -> (Arc<dyn InodeOps>, Arc<dyn InodeOps>) {
    use core::sync::atomic::AtomicU64;
    static NEXT_PIPE_INO: AtomicU64 = AtomicU64::new(0xF000_0000);

    let shared = PipeShared::new();
    let ino_r = NEXT_PIPE_INO.fetch_add(1, Ordering::Relaxed) as InodeNum;
    let ino_w = NEXT_PIPE_INO.fetch_add(1, Ordering::Relaxed) as InodeNum;

    let read_end = Arc::new(PipeReadEnd {
        shared: shared.clone(),
        ino: ino_r,
    });
    let write_end = Arc::new(PipeWriteEnd { shared, ino: ino_w });

    (read_end, write_end)
}
