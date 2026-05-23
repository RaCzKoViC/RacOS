// RaCore — Process filesystem (procfs)
//
// Provides /proc with per-process info and system-wide info files.
// Read-only virtual filesystem.
//
// Layout:
//   /proc/self/       → symlink to /proc/<current_pid>
//   /proc/<pid>/status → process status
//   /proc/<pid>/cmdline → command name
//   /proc/uptime      → system uptime
//   /proc/meminfo     → memory info
//   /proc/version     → kernel version
//   /proc/cpuinfo     → CPU info

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::inode::{
    DirEntry, FileMode, FileType, InodeMetadata, InodeNum, InodeOps, VfsError, VfsResult,
};
use super::mount::Filesystem;

// Inode numbering scheme:
//   0       = /proc (root dir)
//   1       = /proc/uptime
//   2       = /proc/meminfo
//   3       = /proc/version
//   4       = /proc/cpuinfo
//   5       = /proc/stat
//   6       = /proc/loadavg
//   1000+pid*10    = /proc/<pid> directory
//   1000+pid*10+1  = /proc/<pid>/status
//   1000+pid*10+2  = /proc/<pid>/cmdline
//   1000+pid*10+3  = /proc/<pid>/environ
//   1000+pid*10+4  = /proc/<pid>/maps

const INO_ROOT: InodeNum = 0;
const INO_UPTIME: InodeNum = 1;
const INO_MEMINFO: InodeNum = 2;
const INO_VERSION: InodeNum = 3;
const INO_CPUINFO: InodeNum = 4;
const INO_STAT: InodeNum = 5;
const INO_LOADAVG: InodeNum = 6;
const INO_SELF: InodeNum = 7;
const INO_MOUNTS: InodeNum = 8;
const INO_DISKSTATS: InodeNum = 9;
const INO_CACHESTATS: InodeNum = 10;

fn pid_dir_ino(pid: u32) -> InodeNum {
    1000 + pid as u64 * 10
}
fn pid_status_ino(pid: u32) -> InodeNum {
    1000 + pid as u64 * 10 + 1
}
fn pid_cmdline_ino(pid: u32) -> InodeNum {
    1000 + pid as u64 * 10 + 2
}

fn ino_to_pid(ino: InodeNum) -> Option<u32> {
    if ino >= 1000 {
        Some(((ino - 1000) / 10) as u32)
    } else {
        None
    }
}

fn ino_subfile(ino: InodeNum) -> u64 {
    if ino >= 1000 {
        (ino - 1000) % 10
    } else {
        0
    }
}

/// The procfs "filesystem".
pub struct Procfs;

impl Procfs {
    pub fn new() -> Arc<Self> {
        Arc::new(Procfs)
    }
}

// ── Root directory ─────────────────────────────────────

struct ProcRootInode;

impl InodeOps for ProcRootInode {
    fn read(&self, _off: u64, _buf: &mut [u8]) -> VfsResult<usize> {
        Err(VfsError::IsADirectory)
    }
    fn write(&self, _off: u64, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::IsADirectory)
    }
    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let mut m = InodeMetadata::new(INO_ROOT, FileType::Directory);
        m.mode = FileMode::new(0o555);
        Ok(m)
    }
    fn lookup(&self, name: &str) -> VfsResult<InodeNum> {
        match name {
            "uptime" => Ok(INO_UPTIME),
            "meminfo" => Ok(INO_MEMINFO),
            "version" => Ok(INO_VERSION),
            "cpuinfo" => Ok(INO_CPUINFO),
            "stat" => Ok(INO_STAT),
            "loadavg" => Ok(INO_LOADAVG),
            "self" => Ok(INO_SELF),
            "mounts" => Ok(INO_MOUNTS),
            "diskstats" => Ok(INO_DISKSTATS),
            "cachestats" => Ok(INO_CACHESTATS),
            _ => {
                // Try to parse as PID
                if let Ok(pid) = name.parse::<u32>() {
                    Ok(pid_dir_ino(pid))
                } else {
                    Err(VfsError::NotFound)
                }
            }
        }
    }
    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        let mut entries = Vec::new();
        entries.push(DirEntry {
            name: String::from("uptime"),
            ino: INO_UPTIME,
            file_type: FileType::Regular,
        });
        entries.push(DirEntry {
            name: String::from("meminfo"),
            ino: INO_MEMINFO,
            file_type: FileType::Regular,
        });
        entries.push(DirEntry {
            name: String::from("version"),
            ino: INO_VERSION,
            file_type: FileType::Regular,
        });
        entries.push(DirEntry {
            name: String::from("cpuinfo"),
            ino: INO_CPUINFO,
            file_type: FileType::Regular,
        });
        entries.push(DirEntry {
            name: String::from("stat"),
            ino: INO_STAT,
            file_type: FileType::Regular,
        });
        entries.push(DirEntry {
            name: String::from("loadavg"),
            ino: INO_LOADAVG,
            file_type: FileType::Regular,
        });
        entries.push(DirEntry {
            name: String::from("self"),
            ino: INO_SELF,
            file_type: FileType::Directory,
        });
        entries.push(DirEntry {
            name: String::from("mounts"),
            ino: INO_MOUNTS,
            file_type: FileType::Regular,
        });
        entries.push(DirEntry {
            name: String::from("diskstats"),
            ino: INO_DISKSTATS,
            file_type: FileType::Regular,
        });
        entries.push(DirEntry {
            name: String::from("cachestats"),
            ino: INO_CACHESTATS,
            file_type: FileType::Regular,
        });
        // Add entries for known PIDs
        // We scan the scheduler for live tasks
        Ok(entries)
    }
}

// ── System-wide files ──────────────────────────────────

struct ProcFileInode {
    ino: InodeNum,
}

impl ProcFileInode {
    fn generate_content(&self) -> String {
        match self.ino {
            INO_UPTIME => {
                let ms = crate::interrupts::pit::uptime_ms();
                let secs = ms / 1000;
                let frac = ms % 1000;
                format!("{}.{:03} {}.{:03}\n", secs, frac, secs, frac)
            }
            INO_MEMINFO => {
                let total = crate::mm::phys::total_count() * 4; // KiB
                let free = crate::mm::phys::free_count() * 4;
                let used = total.saturating_sub(free);
                format!(
                    "MemTotal:    {} kB\nMemFree:     {} kB\nMemUsed:     {} kB\nBuffers:     0 kB\nCached:      0 kB\n",
                    total, free, used,
                )
            }
            INO_VERSION => {
                format!("RacOS version 0.1.0 (racore) #1\n")
            }
            INO_CPUINFO => {
                format!(
                    "processor\t: 0\nvendor_id\t: RacOS\nmodel name\t: x86_64 Virtual CPU\ncpu MHz\t\t: 1000.000\ncache size\t: 0 KB\nflags\t\t: fpu sse sse2 syscall nx\n"
                )
            }
            INO_STAT => {
                let ms = crate::interrupts::pit::uptime_ms();
                let ticks = ms / 10; // ~100Hz ticks
                format!(
                    "cpu  {} {} {} {} 0 0 0 0 0 0\nprocesses {}\n",
                    ticks / 4,
                    ticks / 4,
                    ticks / 4,
                    ticks / 4,
                    crate::task::scheduler::current_pid(),
                )
            }
            INO_LOADAVG => {
                format!(
                    "0.00 0.00 0.00 1/1 {}\n",
                    crate::task::scheduler::current_pid()
                )
            }
            INO_MOUNTS => {
                // device mountpoint fstype options 0 0
                let mut out = String::new();
                unsafe {
                    let mt = super::mount::mount_table();
                    for m in mt.entries() {
                        let dev = match m.fs.name() {
                            "racfs" => {
                                // Distinguish ram0 racfs (/var) from sda racfs (/mnt) by path.
                                if m.path == "/mnt" {
                                    "/dev/sda"
                                } else {
                                    "/dev/ram0"
                                }
                            }
                            "initramfs" => "initramfs",
                            "tmpfs" => "tmpfs",
                            "devfs" => "devfs",
                            "proc" => "proc",
                            _ => "none",
                        };
                        out.push_str(&format!("{} {} {} rw 0 0\n", dev, m.path, m.fs.name()));
                    }
                }
                out
            }
            INO_CACHESTATS => {
                let mut out =
                    String::from("# mount hits misses cached_entries dirty_entries hit_rate%\n");
                unsafe {
                    let mt = super::mount::mount_table();
                    for m in mt.entries() {
                        let any = m.fs.as_any();
                        if let Some(racfs_fs) = any.downcast_ref::<super::racfs::RacfsFilesystem>()
                        {
                            let (hits, misses, entries, dirty) = racfs_fs.inner().cache_stats();
                            let total = hits + misses;
                            let pct = if total > 0 { (hits * 100) / total } else { 0 };
                            out.push_str(&format!(
                                "{} {} {} {} {} {}\n",
                                m.path, hits, misses, entries, dirty, pct
                            ));
                        }
                    }
                }
                out
            }
            INO_DISKSTATS => {
                // Lines: "<mountpoint> <total_blocks> <used_blocks> <free_blocks> <total_inodes> <free_inodes>"
                // Block size is 512 B. Only racfs mounts report real numbers — other
                // filesystems are reported as 0 since they aren't block-backed.
                let mut out = String::from("# mount total_blocks used_blocks free_blocks total_inodes free_inodes (block=512B)\n");
                unsafe {
                    let mt = super::mount::mount_table();
                    for m in mt.entries() {
                        let any = m.fs.as_any();
                        if let Some(racfs_fs) = any.downcast_ref::<super::racfs::RacfsFilesystem>()
                        {
                            let (tb, fb, ti, fi) = racfs_fs.inner().stats();
                            let used = tb.saturating_sub(fb);
                            out.push_str(&format!(
                                "{} {} {} {} {} {}\n",
                                m.path, tb, used, fb, ti, fi
                            ));
                        } else {
                            out.push_str(&format!("{} 0 0 0 0 0\n", m.path));
                        }
                    }
                }
                out
            }
            _ => String::from(""),
        }
    }
}

impl InodeOps for ProcFileInode {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let content = self.generate_content();
        let bytes = content.as_bytes();
        if offset as usize >= bytes.len() {
            return Ok(0);
        }
        let remaining = &bytes[offset as usize..];
        let n = remaining.len().min(buf.len());
        buf[..n].copy_from_slice(&remaining[..n]);
        Ok(n)
    }
    fn write(&self, _off: u64, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::PermissionDenied)
    }
    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let mut m = InodeMetadata::new(self.ino, FileType::Regular);
        m.mode = FileMode::new(0o444);
        Ok(m)
    }
}

// ── Per-PID directory ──────────────────────────────────

struct ProcPidDirInode {
    pid: u32,
}

impl InodeOps for ProcPidDirInode {
    fn read(&self, _off: u64, _buf: &mut [u8]) -> VfsResult<usize> {
        Err(VfsError::IsADirectory)
    }
    fn write(&self, _off: u64, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::IsADirectory)
    }
    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let mut m = InodeMetadata::new(pid_dir_ino(self.pid), FileType::Directory);
        m.mode = FileMode::new(0o555);
        Ok(m)
    }
    fn lookup(&self, name: &str) -> VfsResult<InodeNum> {
        match name {
            "status" => Ok(pid_status_ino(self.pid)),
            "cmdline" => Ok(pid_cmdline_ino(self.pid)),
            _ => Err(VfsError::NotFound),
        }
    }
    fn readdir(&self) -> VfsResult<Vec<DirEntry>> {
        Ok(alloc::vec![
            DirEntry {
                name: String::from("status"),
                ino: pid_status_ino(self.pid),
                file_type: FileType::Regular
            },
            DirEntry {
                name: String::from("cmdline"),
                ino: pid_cmdline_ino(self.pid),
                file_type: FileType::Regular
            },
        ])
    }
}

// ── Per-PID files ──────────────────────────────────────

struct ProcPidFileInode {
    ino: InodeNum,
    pid: u32,
    sub: u64, // 1=status, 2=cmdline
}

impl ProcPidFileInode {
    fn generate_content(&self) -> String {
        match self.sub {
            1 => {
                // status
                unsafe {
                    core::arch::asm!("cli", options(nomem, nostack));
                    let info = crate::task::scheduler::with_task_by_pid(self.pid, |t| {
                        let name = String::from(
                            core::str::from_utf8(&t.name[..t.name_len]).unwrap_or("?"),
                        );
                        let state = match t.state {
                            crate::task::task::TaskState::Created => "created",
                            crate::task::task::TaskState::Ready => "ready",
                            crate::task::task::TaskState::Running => "running",
                            crate::task::task::TaskState::Blocked => "sleeping",
                            crate::task::task::TaskState::Zombie => "zombie",
                        };
                        (name, state, t.parent_pid)
                    });
                    core::arch::asm!("sti", options(nomem, nostack));

                    if let Some((name, state, ppid)) = info {
                        format!(
                            "Name:\t{}\nState:\t{}\nPid:\t{}\nPPid:\t{}\nUid:\t0\nGid:\t0\n",
                            name, state, self.pid, ppid,
                        )
                    } else {
                        String::from("")
                    }
                }
            }
            2 => {
                // cmdline
                unsafe {
                    core::arch::asm!("cli", options(nomem, nostack));
                    let result = crate::task::scheduler::with_task_by_pid(self.pid, |t| {
                        String::from(core::str::from_utf8(&t.name[..t.name_len]).unwrap_or(""))
                    });
                    core::arch::asm!("sti", options(nomem, nostack));
                    result.unwrap_or_default()
                }
            }
            _ => String::from(""),
        }
    }
}

impl InodeOps for ProcPidFileInode {
    fn read(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let content = self.generate_content();
        let bytes = content.as_bytes();
        if offset as usize >= bytes.len() {
            return Ok(0);
        }
        let remaining = &bytes[offset as usize..];
        let n = remaining.len().min(buf.len());
        buf[..n].copy_from_slice(&remaining[..n]);
        Ok(n)
    }
    fn write(&self, _off: u64, _buf: &[u8]) -> VfsResult<usize> {
        Err(VfsError::PermissionDenied)
    }
    fn metadata(&self) -> VfsResult<InodeMetadata> {
        let mut m = InodeMetadata::new(self.ino, FileType::Regular);
        m.mode = FileMode::new(0o444);
        Ok(m)
    }
}

// ── Filesystem trait impl ──────────────────────────────

pub struct ProcFilesystem {
    _inner: Arc<Procfs>,
}

impl ProcFilesystem {
    pub fn new(procfs: Arc<Procfs>) -> Arc<Self> {
        Arc::new(ProcFilesystem { _inner: procfs })
    }
}

impl Filesystem for ProcFilesystem {
    fn root_inode(&self) -> Arc<dyn InodeOps> {
        Arc::new(ProcRootInode)
    }

    fn get_inode(&self, ino: InodeNum) -> VfsResult<Arc<dyn InodeOps>> {
        match ino {
            INO_ROOT => Ok(Arc::new(ProcRootInode)),
            INO_UPTIME | INO_MEMINFO | INO_VERSION | INO_CPUINFO | INO_STAT | INO_LOADAVG
            | INO_MOUNTS | INO_DISKSTATS | INO_CACHESTATS => Ok(Arc::new(ProcFileInode { ino })),
            INO_SELF => {
                let pid = crate::task::scheduler::current_pid();
                Ok(Arc::new(ProcPidDirInode { pid }))
            }
            _ => {
                if let Some(pid) = ino_to_pid(ino) {
                    let sub = ino_subfile(ino);
                    if sub == 0 {
                        Ok(Arc::new(ProcPidDirInode { pid }))
                    } else {
                        Ok(Arc::new(ProcPidFileInode { ino, pid, sub }))
                    }
                } else {
                    Err(VfsError::NotFound)
                }
            }
        }
    }

    fn name(&self) -> &str {
        "proc"
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
