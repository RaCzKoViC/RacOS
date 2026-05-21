// RaCore — Syscall handler implementations
//
// Each handler validates arguments and performs the requested operation.
// User-space pointers are validated before dereference.
// See SYSCALL_SPEC.md for detailed behavior.

extern crate alloc;

use super::error::{SyscallError, SyscallResult};
use crate::vfs::inode::VfsError;

fn map_vfs_error(err: VfsError) -> SyscallError {
    match err {
        VfsError::NotFound => SyscallError::ENOENT,
        VfsError::PermissionDenied => SyscallError::EACCES,
        VfsError::NotADirectory => SyscallError::ENOTDIR,
        VfsError::IsADirectory => SyscallError::EISDIR,
        VfsError::AlreadyExists => SyscallError::EEXIST,
        VfsError::NoSpace => SyscallError::ENOSPC,
        VfsError::InvalidArgument => SyscallError::EINVAL,
        VfsError::BadFileDescriptor => SyscallError::EBADF,
        VfsError::TooManyOpenFiles => SyscallError::EMFILE,
        VfsError::BrokenPipe => SyscallError::EIO,
        VfsError::WouldBlock => SyscallError::EAGAIN,
        VfsError::IoError | VfsError::NotImplemented => SyscallError::EIO,
    }
}

/// Resolve the writable backing store for a mount entry. Uses Any-based
/// downcast so that multiple mounts of the same FS (e.g. racfs on ram0 and
/// racfs on sda) each return their *own* concrete backing instance, instead
/// of a single global singleton.
fn writable_store_from_mount(
    mount: &crate::vfs::mount::MountEntry,
) -> Option<WritableStore> {
    let any = mount.fs.as_any();
    if let Some(racfs_fs) = any.downcast_ref::<crate::vfs::racfs::RacfsFilesystem>() {
        return Some(WritableStore::Racfs(racfs_fs.inner()));
    }
    if let Some(tmpfs_fs) = any.downcast_ref::<crate::vfs::tmpfs::TmpfsFilesystem>() {
        return Some(WritableStore::Tmpfs(tmpfs_fs.inner()));
    }
    None
}

/// Abstraction over writable filesystem stores (tmpfs and racfs).
enum WritableStore {
    Tmpfs(alloc::sync::Arc<crate::vfs::tmpfs::Tmpfs>),
    Racfs(alloc::sync::Arc<crate::vfs::racfs::Racfs>),
}

impl WritableStore {
    fn split_parent_leaf<'a>(&self, path: &'a str) -> crate::vfs::inode::VfsResult<(u64, &'a str)> {
        match self {
            WritableStore::Tmpfs(t) => t.split_parent_leaf(path),
            WritableStore::Racfs(r) => {
                let (ino, leaf) = r.split_parent_leaf(path)?;
                Ok((ino as u64, leaf))
            }
        }
    }

    fn create_file(&self, parent_ino: u64, name: &str) -> crate::vfs::inode::VfsResult<u64> {
        match self {
            WritableStore::Tmpfs(t) => t.create_file(parent_ino, name),
            WritableStore::Racfs(r) => r.create_file(parent_ino as u32, name).map(|i| i as u64),
        }
    }

    fn create_dir(&self, parent_ino: u64, name: &str) -> crate::vfs::inode::VfsResult<u64> {
        match self {
            WritableStore::Tmpfs(t) => t.create_dir(parent_ino, name),
            WritableStore::Racfs(r) => r.create_dir(parent_ino as u32, name).map(|i| i as u64),
        }
    }

    fn unlink(&self, parent_ino: u64, name: &str) -> crate::vfs::inode::VfsResult<()> {
        match self {
            WritableStore::Tmpfs(t) => t.unlink(parent_ino, name),
            WritableStore::Racfs(r) => r.unlink(parent_ino as u32, name),
        }
    }
}

fn current_creds() -> crate::task::task::Credentials {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let creds = crate::task::scheduler::with_current_task(|t| t.creds)
            .unwrap_or(crate::task::task::Credentials::root());
        core::arch::asm!("sti", options(nomem, nostack));
        creds
    }
}

fn current_umask() -> u32 {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let mask = crate::task::scheduler::with_current_task(|t| t.umask).unwrap_or(0o022);
        core::arch::asm!("sti", options(nomem, nostack));
        mask
    }
}

fn require_cap(cap: u8) -> Result<(), SyscallError> {
    let creds = current_creds();
    if crate::security::capability::has_cap(&creds, cap) {
        Ok(())
    } else {
        Err(SyscallError::EPERM)
    }
}

/// SYSCALL 0x400: pthread_create(routine_ptr, arg_ptr)
pub fn sys_pthread_create(routine: u64, arg: u64) -> SyscallResult {
    crate::serial::serial_println!("[ SYSC ] pthread_create(routine: 0x{:X}, arg: 0x{:X})", routine, arg);
    
    // SAFETY: cli/sti during scheduler access
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let sched = crate::task::scheduler::get_instance();
        
        let res = crate::task::scheduler::with_current_task(|parent| {
            sched.spawn_thread(routine, arg, parent)
        });

        core::arch::asm!("sti", options(nomem, nostack));

        match res {
            Some(Ok(tid)) => Ok(tid as i64),
            Some(Err(e)) => {
                crate::serial::serial_println!("[ SYSC ] pthread_create FAILED: {}", e);
                Err(SyscallError::ENOMEM)
            }
            None => Err(SyscallError::ESRCH),
        }
    }
}



fn require_dac_access(
    meta: &crate::vfs::inode::InodeMetadata,
    access: crate::security::dac::Access,
) -> Result<(), SyscallError> {
    let creds = current_creds();
    if crate::security::dac::can_access(&creds, meta, access) {
        Ok(())
    } else {
        Err(SyscallError::EACCES)
    }
}

fn map_net_error(err: crate::net::NetError) -> SyscallError {
    match err {
        crate::net::NetError::Inval => SyscallError::EINVAL,
        crate::net::NetError::NotSup => SyscallError::ENOSYS,
        crate::net::NetError::BadFd => SyscallError::EBADF,
        crate::net::NetError::AddrInUse => SyscallError::EADDRINUSE,
        crate::net::NetError::NotConn => SyscallError::ENOTCONN,
        crate::net::NetError::ConnRefused => SyscallError::ECONNREFUSED,
        crate::net::NetError::Again => SyscallError::EAGAIN,
        crate::net::NetError::Pipe => SyscallError::EPIPE,
    }
}

fn parse_sockaddr_in(addr: *const u8, len: u32) -> Result<(u16, u32), SyscallError> {
    if len < 8 {
        return Err(SyscallError::EINVAL);
    }
    validate_user_ptr(addr as u64, len as usize)?;
    let b = unsafe { core::slice::from_raw_parts(addr, len as usize) };
    let family = u16::from_le_bytes([b[0], b[1]]);
    if family as i32 != crate::net::AF_INET {
        return Err(SyscallError::EINVAL);
    }
    let port = u16::from_be_bytes([b[2], b[3]]);
    let ip = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    Ok((port, ip))
}

fn write_sockaddr_in(addr: *mut u8, len_ptr: *mut u32, port: u16, ip: u32) -> Result<(), SyscallError> {
    if len_ptr.is_null() {
        return Ok(());
    }
    validate_user_ptr(len_ptr as u64, 4)?;
    let in_len = unsafe { *len_ptr };
    let out_len = 8u32;
    if !addr.is_null() && in_len >= out_len {
        validate_user_ptr(addr as u64, out_len as usize)?;
        let mut buf = [0u8; 8];
        let fam = (crate::net::AF_INET as u16).to_le_bytes();
        let p = port.to_be_bytes();
        let a = ip.to_be_bytes();
        buf[0] = fam[0];
        buf[1] = fam[1];
        buf[2] = p[0];
        buf[3] = p[1];
        buf[4] = a[0];
        buf[5] = a[1];
        buf[6] = a[2];
        buf[7] = a[3];
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), addr, out_len as usize);
        }
    }
    unsafe { *len_ptr = out_len; }
    Ok(())
}

fn alloc_fd_for_socket(sock_sid: usize) -> Result<i32, SyscallError> {
    let (fs, ino) = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path("/dev/null")
            .map_err(map_vfs_error)?
    };
    let inode = fs.get_inode(ino).map_err(map_vfs_error)?;
    let of = alloc::sync::Arc::new(crate::vfs::file::OpenFile::new(
        ino,
        inode,
        crate::vfs::file::flags::O_RDWR,
    ));

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let pid = crate::task::scheduler::current_pid();
        let fd = crate::task::scheduler::with_current_fd_table(|fds| {
            fds.alloc(of).map_err(map_vfs_error)
        })
        .unwrap_or(Err(SyscallError::EBADF))?;
        crate::net::bind_fd(pid, fd, sock_sid);
        core::arch::asm!("sti", options(nomem, nostack));
        Ok(fd)
    }
}

    fn install_fd_exact(
        fds: &mut crate::vfs::file::FdTable,
        target_fd: i32,
        file: alloc::sync::Arc<crate::vfs::file::OpenFile>,
    ) -> Result<(), SyscallError> {
        let _ = fds.close(target_fd);
        let allocated = fds.alloc(file).map_err(map_vfs_error)?;
        if allocated != target_fd {
            fds.dup2(allocated, target_fd).map_err(map_vfs_error)?;
            let _ = fds.close(allocated);
        }
        Ok(())
    }

    fn ensure_console_stdio(fds: &mut crate::vfs::file::FdTable) {
        let need_stdin = fds.get(0).is_err();
        let need_stdout = fds.get(1).is_err();
        let need_stderr = fds.get(2).is_err();

        if !(need_stdin || need_stdout || need_stderr) {
            return;
        }

        let (fs, ino) = match unsafe { crate::vfs::mount::mount_table().lookup_path("/dev/console") } {
            Ok(v) => v,
            Err(_) => return,
        };
        let inode = match fs.get_inode(ino) {
            Ok(i) => i,
            Err(_) => return,
        };

        if need_stdin {
            let stdin = alloc::sync::Arc::new(crate::vfs::file::OpenFile::new(
                ino,
                inode.clone(),
                crate::vfs::file::flags::O_RDONLY,
            ));
            let _ = install_fd_exact(fds, 0, stdin);
        }
        if need_stdout {
            let stdout = alloc::sync::Arc::new(crate::vfs::file::OpenFile::new(
                ino,
                inode.clone(),
                crate::vfs::file::flags::O_WRONLY,
            ));
            let _ = install_fd_exact(fds, 1, stdout);
        }
        if need_stderr {
            let stderr = alloc::sync::Arc::new(crate::vfs::file::OpenFile::new(
                ino,
                inode,
                crate::vfs::file::flags::O_WRONLY,
            ));
            let _ = install_fd_exact(fds, 2, stderr);
        }
    }

/// Maximum valid user-space address.
/// Anything above this is kernel space.
const USER_SPACE_MAX: u64 = 0x0000_7FFF_FFFF_FFFF;

/// Validate that a user-space pointer range is within user address space.
fn validate_user_ptr(ptr: u64, len: usize) -> Result<(), SyscallError> {
    if ptr == 0 {
        return Err(SyscallError::EFAULT);
    }
    if ptr > USER_SPACE_MAX {
        return Err(SyscallError::EFAULT);
    }
    let end = ptr.checked_add(len as u64).ok_or(SyscallError::EFAULT)?;
    if end > USER_SPACE_MAX {
        return Err(SyscallError::EFAULT);
    }
    Ok(())
}

/// Validate a null-terminated user string pointer.
/// Returns the string length (not including null terminator), max 4096.
fn validate_user_string(ptr: u64) -> Result<usize, SyscallError> {
    if ptr == 0 || ptr > USER_SPACE_MAX {
        return Err(SyscallError::EFAULT);
    }

    // Read byte-by-byte up to 4096 chars
    let max_len = 4096usize;
    for i in 0..max_len {
        let addr = ptr.checked_add(i as u64).ok_or(SyscallError::EFAULT)?;
        if addr > USER_SPACE_MAX {
            return Err(SyscallError::EFAULT);
        }
        // SAFETY: We validated the address is in user space
        let byte = unsafe { *(addr as *const u8) };
        if byte == 0 {
            return Ok(i);
        }
    }
    Err(SyscallError::ENAMETOOLONG)
}

// ─────────────────────────────────────────────────
// Helper: collect argv from user space
// ─────────────────────────────────────────────────

/// Read a null-terminated array of string pointers from user space.
/// If argv_ptr == 0, returns a single-element vec with just the path.
/// Maximum 64 arguments, each max 4096 bytes.
fn collect_user_argv(path: &str, argv_ptr: u64) -> Result<alloc::vec::Vec<alloc::vec::Vec<u8>>, SyscallError> {
    const MAX_ARGS: usize = 64;

    if argv_ptr == 0 {
        // No argv provided — use path as argv[0]
        return Ok(alloc::vec![alloc::vec::Vec::from(path.as_bytes())]);
    }

    validate_user_ptr(argv_ptr, 8)?;

    let mut args = alloc::vec::Vec::new();

    for i in 0..MAX_ARGS {
        let ptr_addr = argv_ptr + (i * 8) as u64;
        validate_user_ptr(ptr_addr, 8)?;
        let str_ptr = unsafe { *(ptr_addr as *const u64) };

        if str_ptr == 0 {
            break; // NULL terminator
        }

        let str_len = validate_user_string(str_ptr)?;
        let slice = unsafe { core::slice::from_raw_parts(str_ptr as *const u8, str_len) };
        args.push(alloc::vec::Vec::from(slice));
    }

    if args.is_empty() {
        args.push(alloc::vec::Vec::from(path.as_bytes()));
    }

    Ok(args)
}

// ─────────────────────────────────────────────────
// Syscall 0: sys_exit
// ─────────────────────────────────────────────────
/// Deliver any pending signals for the current task.
/// Called at the end of every syscall before returning to user space.
pub fn deliver_pending_signals() {
    use crate::task::signal::SignalAction;
    loop {
        // SAFETY: disabling interrupts while accessing the scheduler.
        let sig = unsafe {
            core::arch::asm!("cli", options(nomem, nostack));
            let s = crate::task::scheduler::take_pending_signal();
            core::arch::asm!("sti", options(nomem, nostack));
            s
        };
        let sig = match sig {
            None => break,
            Some(s) => s,
        };
        match crate::task::signal::SignalState::default_action(sig) {
            SignalAction::Terminate => {
                sys_exit(-1);
            }
            SignalAction::Ignore => {}
            SignalAction::Stop => {
                unsafe {
                    core::arch::asm!("cli", options(nomem, nostack));
                    crate::task::scheduler::block_and_reschedule();
                }
            }
            SignalAction::Continue => {
                // Task is already running; nothing to do.
            }
        }
    }
}

/// Terminate the calling process.
pub fn sys_exit(status: i32) -> ! {
    crate::serial::serial_println!(
        "[  SYSCALL] sys_exit(status={}) from PID {}",
        status,
        crate::task::scheduler::current_pid()
    );

    // Mark task as zombie and schedule away
    // TODO: proper process cleanup (release address space, fds, signal parent)
    unsafe { crate::task::scheduler::exit_current(status); }

    // Should not reach here — exit_current never returns
    loop {
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)); }
    }
}

// ─────────────────────────────────────────────────
// Syscall 1: sys_read
// ─────────────────────────────────────────────────

/// Read from a file descriptor.
pub fn sys_read(fd: i32, buf: *mut u8, count: usize) -> SyscallResult {
    validate_user_ptr(buf as u64, count)?;

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            let file = fds.get(fd).map_err(map_vfs_error)?;
            let out = core::slice::from_raw_parts_mut(buf, count);
            // Re-enable interrupts during the potentially blocking read
            core::arch::asm!("sti", options(nomem, nostack));
            let n = file.read(out).map_err(map_vfs_error)?;
            core::arch::asm!("cli", options(nomem, nostack));
            Ok(n as i64)
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}
// ─────────────────────────────────────────────────
// Syscall 2: sys_write
// ─────────────────────────────────────────────────

/// Write to a file descriptor.
pub fn sys_write(fd: i32, buf: *const u8, count: usize) -> SyscallResult {
    validate_user_ptr(buf as u64, count)?;

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            let file = fds.get(fd).map_err(map_vfs_error)?;
            let input = core::slice::from_raw_parts(buf, count);
            let n = file.write(input).map_err(map_vfs_error)?;
            Ok(n as i64)
        })
        .unwrap_or_else(|| {
            // Fallback: if FD table lookup fails for stdout/stderr, write to serial
            if fd == 1 || fd == 2 {
                for i in 0..count {
                    let byte = *buf.add(i);
                    crate::serial::serial_print!("{}", byte as char);
                }
                Ok(count as i64)
            } else {
                Err(SyscallError::EBADF)
            }
        });
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}

// ─────────────────────────────────────────────────
// Syscall 3: sys_open
// ─────────────────────────────────────────────────

/// Open a file or device.
pub fn sys_open(path: *const u8, flags: u32, _mode: u32) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    // SAFETY: validated by validate_user_string
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    let lookup_result = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(path_str)
    };

    let (fs, ino, created) = match lookup_result {
        Ok(pair) => (pair.0, pair.1, false),
        Err(crate::vfs::inode::VfsError::NotFound) if flags & crate::vfs::file::flags::O_CREAT != 0 => {
            let mt = unsafe { crate::vfs::mount::mount_table() };
            let (mount, remainder) = mt.resolve(path_str).ok_or(SyscallError::ENOENT)?;
            let store = writable_store_from_mount(mount).ok_or(SyscallError::EACCES)?;
            let (parent_ino, leaf) = store.split_parent_leaf(remainder).map_err(map_vfs_error)?;
            let parent_inode = mount.fs.get_inode(parent_ino).map_err(map_vfs_error)?;
            let parent_meta = parent_inode.metadata().map_err(map_vfs_error)?;
            require_dac_access(&parent_meta, crate::security::dac::Access::Write)?;
            require_dac_access(&parent_meta, crate::security::dac::Access::Execute)?;
            let new_ino = store.create_file(parent_ino, leaf).map_err(map_vfs_error)?;
            (mount.fs.clone(), new_ino, true)
        }
        Err(e) => return Err(map_vfs_error(e)),
    };

    let inode = fs.get_inode(ino).map_err(map_vfs_error)?;
    let mut meta = inode.metadata().map_err(map_vfs_error)?;
    if created {
        let creds = current_creds();
        let base_mode = if (_mode & 0o7777) != 0 { _mode & 0o7777 } else { 0o666 };
        meta.mode = crate::vfs::inode::FileMode::new(base_mode & !current_umask());
        meta.uid = creds.euid;
        meta.gid = creds.egid;
        let _ = inode.set_metadata(&meta);
    }
    let access_mode = flags & crate::vfs::file::flags::ACCESS_MODE_MASK;
    if access_mode == crate::vfs::file::flags::O_RDONLY {
        require_dac_access(&meta, crate::security::dac::Access::Read)?;
    } else if access_mode == crate::vfs::file::flags::O_WRONLY {
        require_dac_access(&meta, crate::security::dac::Access::Write)?;
    } else {
        require_dac_access(&meta, crate::security::dac::Access::Read)?;
        require_dac_access(&meta, crate::security::dac::Access::Write)?;
    }

    if flags & crate::vfs::file::flags::O_TRUNC != 0 {
        require_dac_access(&meta, crate::security::dac::Access::Write)?;
    }

    let of = alloc::sync::Arc::new(crate::vfs::file::OpenFile::new(ino, inode, flags));

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            fds.alloc(of).map(|fd| fd as i64).map_err(map_vfs_error)
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}

// ─────────────────────────────────────────────────
// Syscall 4: sys_close
// ─────────────────────────────────────────────────

/// Close a file descriptor.
pub fn sys_close(fd: i32) -> SyscallResult {
    let pid = crate::task::scheduler::current_pid();
    crate::net::close_fd(pid, fd);
    if let Some(conn_id) = crate::net::close_fd_tcp(pid, fd) {
        let _ = crate::net::tcp::close(conn_id);
    }
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            fds.close(fd).map(|_| 0).map_err(map_vfs_error)
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}

// ─────────────────────────────────────────────────
// Syscall 14: sys_getpid
// ─────────────────────────────────────────────────

/// Get current process ID.
pub fn sys_getpid() -> SyscallResult {
    Ok(crate::task::scheduler::current_pid() as i64)
}

// ─────────────────────────────────────────────────
// Syscall 6: sys_mmap
// ─────────────────────────────────────────────────

/// Map memory into the process address space.
pub fn sys_mmap(
    addr: u64,
    length: usize,
    _prot: u32,
    flags: u32,
    fd: i32,
    _offset: u64,
) -> SyscallResult {
    use crate::mm::{phys, virt};
    use crate::mm::virt::flags as vf;

    const MAP_ANONYMOUS: u32 = 0x20;

    if length == 0 {
        return Err(SyscallError::EINVAL);
    }

    if flags & MAP_ANONYMOUS == 0 {
        if fd < 0 {
            return Err(SyscallError::EBADF);
        }
        crate::serial::serial_println!(
            "[ SYSC ] mmap(file-backed) not implemented yet: fd={}, len={}",
            fd,
            length
        );
        return Err(SyscallError::ENOSYS); // File-backed mmap: Phase 2f
    }

    // Allocate physical frames for the anonymous mapping.
    let pages = (length + 0xFFF) / 0x1000;
    let frame = phys::alloc_contiguous(pages).map_err(|_| SyscallError::ENOMEM)?;
    let phys_addr = frame.addr();

    // Zero the allocation.
    unsafe {
        core::ptr::write_bytes(phys_addr as *mut u8, 0, pages * 0x1000);
    }

    let pt = crate::task::scheduler::current_page_table_phys();
    if pt != 0 {
        // Map into the user process page table.
        let virt_addr = if addr != 0 { addr } else {
            // Simple bump allocator for anonymous maps in user space.
            // Use a region just below the user stack (well-separated).
            static MMAP_BUMP: core::sync::atomic::AtomicU64 =
                core::sync::atomic::AtomicU64::new(0x0000_7FF0_0000_0000);
            let alloc_size = (pages * 0x1000) as u64;
            let prev = MMAP_BUMP.fetch_sub(
                alloc_size,
                core::sync::atomic::Ordering::Relaxed,
            );
            let v = prev - alloc_size;
            // Guard: prevent underflow into low memory / kernel space
            if v < 0x0000_1000_0000_0000 || v > prev {
                // Undo the bump
                MMAP_BUMP.fetch_add(alloc_size, core::sync::atomic::Ordering::Relaxed);
                // Free the allocated frames
                for i in 0..pages {
                    let _ = phys::free_frame(phys::PhysFrame::containing(phys_addr + (i * 0x1000) as u64));
                }
                return Err(SyscallError::ENOMEM);
            }
            v
        };

        if let Err(_) = unsafe {
            virt::map_range(pt, virt_addr, phys_addr, (pages * 0x1000) as u64, vf::USER_DATA)
        } {
            // Mapping failed — free frames and report error.
            for i in 0..pages {
                let _ = phys::free_frame(phys::PhysFrame::containing(phys_addr + (i * 0x1000) as u64));
            }
            return Err(SyscallError::ENOMEM);
        }
        Ok(virt_addr as i64)
    } else {
        // Kernel task: return physical address (identity-mapped).
        Ok(phys_addr as i64)
    }
}

// ─────────────────────────────────────────────────
// Syscall 7: sys_munmap
// ─────────────────────────────────────────────────

/// Unmap memory from the process address space.
pub fn sys_munmap(addr: u64, length: usize) -> SyscallResult {
    if addr & 0xFFF != 0 {
        return Err(SyscallError::EINVAL);
    }
    if length == 0 {
        return Err(SyscallError::EINVAL);
    }

    let pt = crate::task::scheduler::current_page_table_phys();
    if pt == 0 {
        return Ok(0); // Kernel task: no-op
    }

    let pages = (length + 0xFFF) / 0x1000;
    for i in 0..pages {
        let virt = addr + (i * 0x1000) as u64;
        // SAFETY: pt is the validated page table of the current process.
        unsafe {
            if let Ok(frame) = crate::mm::virt::unmap_page(pt, crate::mm::virt::VirtAddr(virt)) {
                let _ = crate::mm::phys::free_frame(frame);
            }
        }
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 8: sys_pipe
// ─────────────────────────────────────────────────

/// Create an anonymous pipe.
/// `fds_ptr` must point to two i32 slots: [read_fd, write_fd].
pub fn sys_pipe(fds_ptr: *mut i32) -> SyscallResult {
    validate_user_ptr(fds_ptr as u64, core::mem::size_of::<i32>() * 2)?;

    let (read_inode, write_inode) = crate::vfs::pipe::create_pipe();
    let read_file = alloc::sync::Arc::new(crate::vfs::file::OpenFile::new(
        0,
        read_inode,
        crate::vfs::file::flags::O_RDONLY,
    ));
    let write_file = alloc::sync::Arc::new(crate::vfs::file::OpenFile::new(
        0,
        write_inode,
        crate::vfs::file::flags::O_WRONLY,
    ));

    let (fd_r, fd_w) = unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let res = crate::task::scheduler::with_current_fd_table(|fds| {
            let r = fds.alloc(read_file).map_err(map_vfs_error)?;
            let w = fds.alloc(write_file).map_err(map_vfs_error)?;
            Ok((r, w))
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        res?
    };

    unsafe {
        *fds_ptr.add(0) = fd_r;
        *fds_ptr.add(1) = fd_w;
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 9: sys_dup
// ─────────────────────────────────────────────────

/// Duplicate a file descriptor.
pub fn sys_dup(oldfd: i32) -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            fds.dup(oldfd).map(|fd| fd as i64).map_err(map_vfs_error)
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}

// ─────────────────────────────────────────────────
// Syscall 10: sys_dup2
// ─────────────────────────────────────────────────

/// Duplicate `oldfd` to exactly `newfd`.
pub fn sys_dup2(oldfd: i32, newfd: i32) -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            fds.dup2(oldfd, newfd).map(|fd| fd as i64).map_err(map_vfs_error)
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}

// ─────────────────────────────────────────────────
// Syscall 11: sys_exec
// ─────────────────────────────────────────────────

/// Replace the current process with a new executable.
/// `path` is a null-terminated string in user space.
pub fn sys_exec(path: *const u8, _argv: u64, _envp: u64) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    // SAFETY: path is validated to be in user space with a null terminator.
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    // Look up the executable in the VFS.
    let (fs, ino) = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(path_str)
            .map_err(|_| SyscallError::ENOENT)?
    };
    let inode = fs.get_inode(ino).map_err(|_| SyscallError::ENOENT)?;
    let meta = inode.metadata().map_err(|_| SyscallError::EIO)?;
    let size = meta.size as usize;
    if size == 0 {
        return Err(SyscallError::ENOEXEC);
    }

    // Allocate a kernel buffer and read the ELF.
    let buf = alloc::vec![0u8; size];
    let mut buf = buf;
    let bytes_read = inode.read(0, &mut buf).map_err(|_| SyscallError::EIO)?;
    if bytes_read < size {
        return Err(SyscallError::EIO);
    }

    // Parse and load the ELF.
    let loaded = crate::elf::load_elf(&buf).map_err(|_| SyscallError::ENOEXEC)?;

    // Collect argv from user space (if provided).
    let argv_strs = collect_user_argv(path_str, _argv)?;
    let argv_refs: alloc::vec::Vec<&[u8]> = argv_strs.iter().map(|s| s.as_slice()).collect();

    // Build a new UserProcess from the loaded ELF.
    // from_elf creates new page table and maps segments + stack.
    let process = crate::task::process::UserProcess::from_elf(path_str, &loaded, &argv_refs)
        .map_err(|_| SyscallError::ENOMEM)?;

    // Replace the current task's state with the new process's state in-place.
    // This preserves PID, parent_pid, pgid, session_id and open fds (non-CLOEXEC).
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        crate::task::scheduler::replace_current_image(&process.task);
        // Switch to the new page table immediately.
        crate::mm::virt::write_cr3(process.task.page_table_phys);
        // Update syscall entry kernel RSP.
        let kstack_top = process.task.kernel_stack_base
            + (crate::task::task::KERNEL_STACK_PAGES * crate::mm::phys::FRAME_SIZE) as u64;
        crate::syscall::entry::set_kernel_rsp(kstack_top);
        // Context switch to the new entry point (IRETQ trampoline).
        // We set up a fresh context and jump — this does not return.
        let new_ctx = &process.task.context as *const crate::task::context::TaskContext;
        // Jump directly to the new task's saved RIP
        core::arch::asm!(
            "mov rsp, [{ctx} + 0x30]",  // restore RSP
            "jmp [{ctx} + 0x38]",       // jump to RIP
            ctx = in(reg) new_ctx,
            options(noreturn),
        );
    }
}

// ─────────────────────────────────────────────────
// Syscall 12: sys_spawn
// ─────────────────────────────────────────────────

/// Create a child process from an ELF path (posix_spawn style).
/// `path` is a null-terminated string in user space.
/// Returns child PID to the caller.
pub fn sys_spawn(path: *const u8, _argv: u64, _envp: u64) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    // Look up the executable in the VFS.
    let (fs, ino) = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(path_str)
            .map_err(|_| SyscallError::ENOENT)?
    };
    let inode = fs.get_inode(ino).map_err(|_| SyscallError::ENOENT)?;
    let meta = inode.metadata().map_err(|_| SyscallError::EIO)?;
    let size = meta.size as usize;
    if size == 0 {
        return Err(SyscallError::ENOEXEC);
    }

    let mut buf = alloc::vec![0u8; size];
    let bytes_read = inode.read(0, &mut buf).map_err(|_| SyscallError::EIO)?;
    if bytes_read < size {
        return Err(SyscallError::EIO);
    }

    let loaded = crate::elf::load_elf(&buf).map_err(|_| SyscallError::ENOEXEC)?;

    // Collect argv from user space (if provided).
    let argv_strs = collect_user_argv(path_str, _argv)?;
    let argv_refs: alloc::vec::Vec<&[u8]> = argv_strs.iter().map(|s| s.as_slice()).collect();

    let mut process = crate::task::process::UserProcess::from_elf(path_str, &loaded, &argv_refs)
        .map_err(|_| SyscallError::ENOMEM)?;

    // Snapshot inheritable parent state first, then apply it to the child.
    let (parent_fds, parent_creds, parent_umask, parent_cwd, parent_cwd_len) = unsafe {
        core::arch::asm!("cli", options(nomem, nostack));

        let fds = crate::task::scheduler::with_current_fd_table(|fds| fds.clone_fds())
            .unwrap_or_else(crate::vfs::file::FdTable::new);

        let (creds, umask) = crate::task::scheduler::with_current_task(|t| (t.creds, t.umask))
            .unwrap_or((crate::task::task::Credentials::root(), 0o022));

        let mut cwd = [0u8; 256];
        let cwd_len = crate::task::scheduler::get_cwd(&mut cwd);

        core::arch::asm!("sti", options(nomem, nostack));
        (fds, creds, umask, cwd, cwd_len)
    };

    process.task.fd_table = parent_fds;
    process.task.creds = parent_creds;
    process.task.umask = parent_umask;

    let had_stdio = (
        process.task.fd_table.get(0).is_ok(),
        process.task.fd_table.get(1).is_ok(),
        process.task.fd_table.get(2).is_ok(),
    );
    ensure_console_stdio(&mut process.task.fd_table);
    let has_stdio = (
        process.task.fd_table.get(0).is_ok(),
        process.task.fd_table.get(1).is_ok(),
        process.task.fd_table.get(2).is_ok(),
    );
    if had_stdio != has_stdio {
        crate::serial::serial_println!(
            "[  SYSC ] spawn('{}'): stdio before=({},{},{}) after=({},{},{})",
            path_str,
            had_stdio.0,
            had_stdio.1,
            had_stdio.2,
            has_stdio.0,
            has_stdio.1,
            has_stdio.2
        );
    }

    process.task.cwd[..parent_cwd_len].copy_from_slice(&parent_cwd[..parent_cwd_len]);
    process.task.cwd_len = parent_cwd_len;

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let pid = crate::task::scheduler::spawn_user(process)
            .map_err(|_| SyscallError::EAGAIN)?;
        core::arch::asm!("sti", options(nomem, nostack));
        Ok(pid as i64)
    }
}

// ─────────────────────────────────────────────────
// Syscall 13: sys_wait
// ─────────────────────────────────────────────────

/// Wait for a child process to exit.
/// `pid` follows waitpid semantics:
/// - `-1`: any child
/// - `0`: any child in caller's process group
/// - `>0`: exact child PID
/// - `<-1`: any child in process group `-pid`
/// Returns the child PID on success; writes exit status to `status_ptr` if non-null.
pub fn sys_wait(pid: i32, status_ptr: u64, options: u32) -> SyscallResult {
    const WNOHANG: u32 = 0x0001;

    if pid == i32::MIN {
        return Err(SyscallError::EINVAL);
    }
    if options & !WNOHANG != 0 {
        return Err(SyscallError::EINVAL);
    }

    let parent = crate::task::scheduler::current_pid();
    let parent_pgid = crate::task::scheduler::current_pgid();

    // Validate the status pointer when provided.
    if status_ptr != 0 {
        validate_user_ptr(status_ptr, 4)?;
    }

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));

        // Do we have any children at all?
        if !crate::task::scheduler::has_children_filtered(parent, pid, parent_pgid) {
            core::arch::asm!("sti", options(nomem, nostack));
            return Err(SyscallError::ECHILD);
        }

        // Spin-block until a zombie child is available.
        loop {
            if let Some((child_pid, exit_status)) =
                crate::task::scheduler::reap_zombie_child_filtered(parent, pid, parent_pgid)
            {
                core::arch::asm!("sti", options(nomem, nostack));
                // Write exit status to user-space pointer if supplied.
                if status_ptr != 0 {
                    *(status_ptr as *mut i32) = exit_status;
                }
                return Ok(child_pid as i64);
            }

            if options & WNOHANG != 0 {
                core::arch::asm!("sti", options(nomem, nostack));
                return Ok(0);
            }

            // No zombie yet: block and let the scheduler pick another task.
            // When a child exits it will unblock us (see exit_current).
            crate::task::scheduler::block_and_reschedule();

            // After being unblocked, re-check for ECHILD.
            if !crate::task::scheduler::has_children_filtered(parent, pid, parent_pgid) {
                core::arch::asm!("sti", options(nomem, nostack));
                return Err(SyscallError::ECHILD);
            }
        }
    }
}

// ─────────────────────────────────────────────────
// Syscall 15: sys_chdir
// ─────────────────────────────────────────────────

/// Change current working directory.
pub fn sys_chdir(path: *const u8) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    // Verify the path exists and is a directory
    let (_fs, _ino) = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(path_str)
            .map_err(|_| SyscallError::ENOENT)?
    };

    // Normalize: ensure no trailing slash (except root)
    let path_bytes = path_str.as_bytes();
    let store_len = if path_bytes.len() > 1 && path_bytes[path_bytes.len() - 1] == b'/' {
        path_bytes.len() - 1
    } else {
        path_bytes.len()
    };
    if store_len > 255 {
        return Err(SyscallError::ENAMETOOLONG);
    }

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        crate::task::scheduler::set_cwd(&path_bytes[..store_len]);
        core::arch::asm!("sti", options(nomem, nostack));
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 17: sys_kill
// ─────────────────────────────────────────────────

/// Send a signal to a process.
pub fn sys_kill(pid: i32, sig: i32) -> SyscallResult {
    if pid <= 0 {
        return Err(SyscallError::EINVAL);
    }
    let signal = crate::task::signal::Signal::from_u8(sig as u8)
        .ok_or(SyscallError::EINVAL)?;
    let found = unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let r = crate::task::scheduler::send_signal_to(pid as u32, signal);
        core::arch::asm!("sti", options(nomem, nostack));
        r
    };
    if found { Ok(0) } else { Err(SyscallError::ESRCH) }
}

// ─────────────────────────────────────────────────
// Syscall 18: sys_getcwd
// ─────────────────────────────────────────────────

/// Get current working directory path.
pub fn sys_getcwd(buf: *mut u8, size: usize) -> SyscallResult {
    if size == 0 {
        return Err(SyscallError::EINVAL);
    }
    validate_user_ptr(buf as u64, size)?;

    let mut tmp = [0u8; 256];
    let cwd_len = unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let len = crate::task::scheduler::get_cwd(&mut tmp);
        core::arch::asm!("sti", options(nomem, nostack));
        len
    };

    // Need space for path + null terminator
    if size < cwd_len + 1 {
        return Err(SyscallError::ERANGE);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(tmp.as_ptr(), buf, cwd_len);
        *buf.add(cwd_len) = 0; // null terminator
    }
    // Return the length, not the buffer pointer. libc-lite's wrapper treats
    // the syscall return value as a usize length; returning the pointer made
    // user-space see e.g. `n = 0x7FFFFFFEFE00`, and `&buf[..n]` panicked
    // out of bounds in racsh's builtin_pwd.
    Ok(cwd_len as i64)
}

// ─────────────────────────────────────────────────
// Syscall 19: sys_setpgid
// ─────────────────────────────────────────────────

/// Set the process group ID of a process.
/// If `pid` is 0, use the calling process. If `pgid` is 0, use `pid` as pgid.
pub fn sys_setpgid(mut pid: u32, mut pgid: u32) -> SyscallResult {
    let current = crate::task::scheduler::current_pid();
    if pid == 0 { pid = current; }
    if pgid == 0 { pgid = pid; }

    let found = unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let r = crate::task::scheduler::set_pgid(pid, pgid);
        core::arch::asm!("sti", options(nomem, nostack));
        r
    };
    if found { Ok(0) } else { Err(SyscallError::ESRCH) }
}

// ─────────────────────────────────────────────────
// Syscall 20: sys_getpgid
// ─────────────────────────────────────────────────

/// Get the process group ID of a process. If `pid` is 0, use the calling process.
pub fn sys_getpgid(mut pid: u32) -> SyscallResult {
    if pid == 0 { pid = crate::task::scheduler::current_pid(); }
    let result = unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let r = crate::task::scheduler::get_pgid(pid);
        core::arch::asm!("sti", options(nomem, nostack));
        r
    };
    match result {
        Some(pgid) => Ok(pgid as i64),
        None => Err(SyscallError::ESRCH),
    }
}

// ─────────────────────────────────────────────────
// Syscall 21: sys_setsid
// ─────────────────────────────────────────────────

/// Create a new session. The calling process becomes session leader and process group leader.
pub fn sys_setsid() -> SyscallResult {
    let sid = unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let s = crate::task::scheduler::create_session();
        core::arch::asm!("sti", options(nomem, nostack));
        s
    };
    Ok(sid as i64)
}

// ─────────────────────────────────────────────────
// Syscall 5: sys_stat
// ─────────────────────────────────────────────────

/// Stat buffer matching the kernel ABI layout.
#[repr(C)]
struct StatBuf {
    st_dev: u64,
    st_ino: u64,
    st_mode: u32,
    st_nlink: u32,
    st_uid: u32,
    st_gid: u32,
    st_size: u64,
    st_atime: u64,
    st_mtime: u64,
    st_ctime: u64,
    st_rdev_major: u32,
    st_rdev_minor: u32,
}

/// Get file status.
pub fn sys_stat(path: *const u8, buf: *mut u8) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    validate_user_ptr(buf as u64, core::mem::size_of::<StatBuf>())?;

    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    let (fs, ino) = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(path_str)
            .map_err(|_| SyscallError::ENOENT)?
    };
    let inode = fs.get_inode(ino).map_err(map_vfs_error)?;
    let meta = inode.metadata().map_err(map_vfs_error)?;

    let stat = StatBuf {
        st_dev: 0,
        st_ino: meta.ino,
        st_mode: meta.file_type as u32 | meta.mode.0,
        st_nlink: meta.nlink,
        st_uid: meta.uid,
        st_gid: meta.gid,
        st_size: meta.size,
        st_atime: meta.atime,
        st_mtime: meta.mtime,
        st_ctime: meta.ctime,
        st_rdev_major: meta.dev_major,
        st_rdev_minor: meta.dev_minor,
    };

    unsafe {
        core::ptr::copy_nonoverlapping(
            &stat as *const StatBuf as *const u8,
            buf,
            core::mem::size_of::<StatBuf>(),
        );
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 16: sys_ioctl
// ─────────────────────────────────────────────────

// TTY ioctl request codes
const TIOCGWINSZ: u32 = 0x5413;
const TIOCSWINSZ: u32 = 0x5414;
const TIOCGPGRP: u32 = 0x540F;
const TIOCSPGRP: u32 = 0x5410;

/// I/O control on a file descriptor.
pub fn sys_ioctl(fd: i32, request: u32, arg: u64) -> SyscallResult {
    match request {
        TIOCGWINSZ => {
            // Get window size — return default 80x25 for now
            validate_user_ptr(arg, 4)?;
            let ws = crate::tty::line_discipline::WinSize::default();
            unsafe {
                let ptr = arg as *mut u16;
                *ptr = ws.rows;
                *ptr.add(1) = ws.cols;
            }
            Ok(0)
        }
        TIOCSWINSZ => {
            // Set window size
            validate_user_ptr(arg, 4)?;
            let (rows, cols) = unsafe {
                let ptr = arg as *const u16;
                (*ptr, *ptr.add(1))
            };
            // TODO: find the TTY associated with this fd and update winsize
            // For now, just deliver SIGWINCH to current process group
            let pgid = crate::task::scheduler::current_pgid();
            if pgid != 0 {
                unsafe {
                    core::arch::asm!("cli", options(nomem, nostack));
                    crate::task::scheduler::send_signal_to_group(
                        pgid,
                        crate::task::signal::Signal::SIGWINCH,
                    );
                    core::arch::asm!("sti", options(nomem, nostack));
                }
            }
            let _ = (rows, cols);
            Ok(0)
        }
        TIOCGPGRP => {
            // Get foreground process group
            validate_user_ptr(arg, 4)?;
            let pgid = crate::task::scheduler::current_pgid();
            unsafe { *(arg as *mut u32) = pgid; }
            Ok(0)
        }
        TIOCSPGRP => {
            // Set foreground process group
            validate_user_ptr(arg, 4)?;
            let pgid = unsafe { *(arg as *const u32) };
            // TODO: update the TTY's foreground pgid
            let _ = pgid;
            Ok(0)
        }
        _ => {
            // Try VFS-level ioctl
            if fd < 0 { return Err(SyscallError::EBADF); }
            unsafe {
                core::arch::asm!("cli", options(nomem, nostack));
                let result = crate::task::scheduler::with_current_fd_table(|fds| {
                    let file = fds.get(fd).map_err(map_vfs_error)?;
                    file.inode.ioctl(request as u64, arg).map(|v| v as i64).map_err(map_vfs_error)
                })
                .unwrap_or(Err(SyscallError::EBADF));
                core::arch::asm!("sti", options(nomem, nostack));
                result
            }
        }
    }
}

// ─────────────────────────────────────────────────
// Syscall 22: sys_clock_gettime
// ─────────────────────────────────────────────────

/// Timespec layout matching the kernel ABI.
#[repr(C)]
struct Timespec {
    tv_sec: u64,
    tv_nsec: u64,
}

const CLOCK_REALTIME: u32 = 0;
const CLOCK_MONOTONIC: u32 = 1;

/// Get the current time for the given clock.
pub fn sys_clock_gettime(clock_id: u32, tp: *mut u8) -> SyscallResult {
    validate_user_ptr(tp as u64, core::mem::size_of::<Timespec>())?;

    let ms = crate::interrupts::pit::uptime_ms();
    let ts = match clock_id {
        CLOCK_REALTIME | CLOCK_MONOTONIC => {
            // No RTC—both clocks return uptime
            Timespec {
                tv_sec: ms / 1000,
                tv_nsec: (ms % 1000) * 1_000_000,
            }
        }
        _ => return Err(SyscallError::EINVAL),
    };

    unsafe {
        core::ptr::copy_nonoverlapping(
            &ts as *const Timespec as *const u8,
            tp,
            core::mem::size_of::<Timespec>(),
        );
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 23: sys_getdents
// ─────────────────────────────────────────────────

/// Directory entry layout in user buffer.
/// Each entry: [ino: u64][type: u8][name_len: u8][name: name_len bytes]
/// Total entry size = 10 + name_len.
pub fn sys_getdents(fd: i32, buf: *mut u8, buf_size: usize) -> SyscallResult {
    validate_user_ptr(buf as u64, buf_size)?;

    // Get the inode from the fd
    let entries = unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            let file = fds.get(fd).map_err(map_vfs_error)?;
            file.inode.readdir().map_err(map_vfs_error)
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result?
    };

    // Serialize entries into user buffer
    let mut offset = 0usize;
    for entry in &entries {
        let name_bytes = entry.name.as_bytes();
        let name_len = name_bytes.len().min(255);
        let entry_size = 10 + name_len; // ino(8) + type(1) + name_len(1) + name
        if offset + entry_size > buf_size {
            break;
        }
        unsafe {
            // ino: u64
            let dst = buf.add(offset);
            core::ptr::copy_nonoverlapping(
                &entry.ino as *const u64 as *const u8,
                dst,
                8,
            );
            // file_type: u8
            *dst.add(8) = entry.file_type as u8;
            // name_len: u8
            *dst.add(9) = name_len as u8;
            // name bytes
            core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), dst.add(10), name_len);
        }
        offset += entry_size;
    }

    Ok(offset as i64)
}

// ─────────────────────────────────────────────────
// Syscall 24: sys_mkdir
// ─────────────────────────────────────────────────

/// Create a directory.
pub fn sys_mkdir(path: *const u8, _mode: u32) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    // Resolve through mount table to find the right FS and relative path
    let mt = unsafe { crate::vfs::mount::mount_table() };
    let (mount, remainder) = mt.resolve(path_str).ok_or(SyscallError::ENOENT)?;
    let store = writable_store_from_mount(mount).ok_or(SyscallError::EACCES)?;

    let (parent_ino, leaf) = store.split_parent_leaf(remainder).map_err(map_vfs_error)?;
    let parent_inode = mount.fs.get_inode(parent_ino).map_err(map_vfs_error)?;
    let parent_meta = parent_inode.metadata().map_err(map_vfs_error)?;
    require_dac_access(&parent_meta, crate::security::dac::Access::Write)?;
    require_dac_access(&parent_meta, crate::security::dac::Access::Execute)?;
    let new_ino = store.create_dir(parent_ino, leaf).map_err(map_vfs_error)?;
    if let Ok(new_inode) = mount.fs.get_inode(new_ino) {
        if let Ok(mut meta) = new_inode.metadata() {
            let creds = current_creds();
            let base_mode = if (_mode & 0o7777) != 0 { _mode & 0o7777 } else { 0o777 };
            meta.mode = crate::vfs::inode::FileMode::new(base_mode & !current_umask());
            meta.uid = creds.euid;
            meta.gid = creds.egid;
            let _ = new_inode.set_metadata(&meta);
        }
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 25: sys_unlink
// ─────────────────────────────────────────────────

/// Remove a file or empty directory.
pub fn sys_unlink(path: *const u8) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    let mt = unsafe { crate::vfs::mount::mount_table() };
    let (mount, remainder) = mt.resolve(path_str).ok_or(SyscallError::ENOENT)?;
    let store = writable_store_from_mount(mount).ok_or(SyscallError::EACCES)?;

    let (parent_ino, leaf) = store.split_parent_leaf(remainder)
        .map_err(map_vfs_error)?;
    let parent_inode = mount.fs.get_inode(parent_ino).map_err(map_vfs_error)?;
    let parent_meta = parent_inode.metadata().map_err(map_vfs_error)?;
    require_dac_access(&parent_meta, crate::security::dac::Access::Write)?;
    require_dac_access(&parent_meta, crate::security::dac::Access::Execute)?;
    store.unlink(parent_ino, leaf)
        .map_err(map_vfs_error)?;
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 26: sys_fork
// ─────────────────────────────────────────────────

/// Fork the current process.
///
/// Returns child PID to parent, 0 to child.
/// The child gets a full copy of the parent's address space and FD table.
pub fn sys_fork() -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));

        // Only user processes can fork.
        let parent_pt = crate::task::scheduler::current_page_table_phys();
        if parent_pt == 0 {
            core::arch::asm!("sti", options(nomem, nostack));
            return Err(SyscallError::EPERM);
        }

        // Clone the user address space.
        let child_pt = match crate::mm::virt::clone_user_page_table(parent_pt) {
            Ok(pt) => pt,
            Err(_) => {
                core::arch::asm!("sti", options(nomem, nostack));
                return Err(SyscallError::ENOMEM);
            }
        };

        // Allocate child kernel stack + guard page. See process::from_elf
        // for the layout invariant the scheduler's guard check relies on.
        let total_pages = crate::task::task::KERNEL_STACK_PAGES
            + crate::task::task::KERNEL_STACK_GUARD_PAGES;
        let child_alloc_frame = match crate::mm::phys::alloc_contiguous(total_pages) {
            Ok(f) => f,
            Err(_) => {
                crate::mm::virt::free_page_table(child_pt, true);
                core::arch::asm!("sti", options(nomem, nostack));
                return Err(SyscallError::ENOMEM);
            }
        };
        let child_alloc_base = child_alloc_frame.addr();
        let child_stack_base = child_alloc_base
            + (crate::task::task::KERNEL_STACK_GUARD_PAGES * crate::mm::phys::FRAME_SIZE) as u64;
        let child_stack_top =
            child_stack_base + crate::task::task::KERNEL_STACK_SIZE as u64;

        // Guard page sentinel + zero usable stack.
        core::ptr::write_bytes(
            child_alloc_base as *mut u8,
            crate::task::task::KERNEL_STACK_GUARD_BYTE,
            crate::task::task::KERNEL_STACK_GUARD_PAGES * crate::mm::phys::FRAME_SIZE,
        );
        core::ptr::write_bytes(
            child_stack_base as *mut u8,
            0,
            crate::task::task::KERNEL_STACK_SIZE,
        );

        // Copy the syscall return frame (80 bytes at top of parent's kernel stack)
        // to the child's kernel stack so fork_child_return can SYSRET properly.
        let parent_stack_top = crate::task::scheduler::current_kernel_stack_top();
        const SYSRET_FRAME_SIZE: u64 = 80;
        core::ptr::copy_nonoverlapping(
            (parent_stack_top - SYSRET_FRAME_SIZE) as *const u8,
            (child_stack_top - SYSRET_FRAME_SIZE) as *mut u8,
            SYSRET_FRAME_SIZE as usize,
        );

        // Gather parent state.
        let parent_pid = crate::task::scheduler::current_pid();
        let (pgid, session_id, creds, umask, name, name_len, cwd, cwd_len, fd_table) =
            crate::task::scheduler::with_current_task(|t| {
                (
                    t.pgid,
                    t.session_id,
                    t.creds,
                    t.umask,
                    t.name,
                    t.name_len,
                    t.cwd,
                    t.cwd_len,
                    t.fd_table.clone_fds(),
                )
            })
            .unwrap();

        let child_pid = crate::task::process::alloc_user_pid();

        // Build child TaskContext: when context-switched to, it enters
        // fork_child_return with RSP pointing at the SYSRET frame.
        let mut ctx = crate::task::context::TaskContext::new();
        ctx.rip = fork_child_return as u64;
        ctx.rsp = child_stack_top - SYSRET_FRAME_SIZE;

        let child_task = crate::task::task::Task {
            pid: child_pid,
            parent_pid: parent_pid,
            state: crate::task::task::TaskState::Created,
            context: ctx,
            kernel_stack_base: child_stack_base,
            page_table_phys: child_pt,
            exit_status: 0,
            signals: crate::task::signal::SignalState::new(),
            fd_table,
            pgid,
            session_id,
            creds,
            umask,
            name,
            name_len,
            cwd,
            cwd_len,
        };

        match crate::task::scheduler::spawn_forked(child_task) {
            Ok(pid) => {
                core::arch::asm!("sti", options(nomem, nostack));
                Ok(pid as i64)
            }
            Err(_) => {
                // Cleanup on failure: free the entire (guard + stack) alloc.
                crate::mm::virt::free_page_table(child_pt, true);
                let total_pages = crate::task::task::KERNEL_STACK_PAGES
                    + crate::task::task::KERNEL_STACK_GUARD_PAGES;
                for i in 0..total_pages {
                    let addr = child_alloc_base
                        + (i * crate::mm::phys::FRAME_SIZE) as u64;
                    let _ = crate::mm::phys::free_frame(
                        crate::mm::phys::PhysFrame::containing(addr),
                    );
                }
                core::arch::asm!("sti", options(nomem, nostack));
                Err(SyscallError::EAGAIN)
            }
        }
    }
}

/// Trampoline for the forked child process.
///
/// When context_switch first switches to the child, it jumps here.
/// RSP points to the copied SYSRET frame on the child's kernel stack.
/// We set RAX=0 (child return value), enable interrupts, then replay
/// the syscall_entry exit sequence.
#[unsafe(naked)]
unsafe extern "C" fn fork_child_return() {
    core::arch::naked_asm!(
        "xor eax, eax",   // RAX = 0 (fork return for child)
        "sti",            // Re-enable interrupts
        "add rsp, 8",     // Skip arg6 (r9)
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "pop rcx",        // User RIP
        "pop r11",        // User RFLAGS
        "pop rsp",        // User RSP
        "swapgs",
        "sysretq",
    );
}

// ─────────────────────────────────────────────────
// Syscall 77: sys_clone
// ─────────────────────────────────────────────────

/// Clone flags for sys_clone.
pub const CLONE_VM: u32 = 0x00000100;     // Share address space
pub const CLONE_THREAD: u32 = 0x00010000; // Thread group

/// Clone the current process.
/// Similar to fork, but with flags for sharing resources.
/// For threads: CLONE_VM | CLONE_THREAD
pub fn sys_clone(flags: u32, stack: *mut u8, ptid: i32, tls: i32, ctid: *mut u8) -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));

        // Only user processes can clone.
        let parent_pt = crate::task::scheduler::current_page_table_phys();
        if parent_pt == 0 {
            core::arch::asm!("sti", options(nomem, nostack));
            return Err(SyscallError::EPERM);
        }

        // For threads, share the address space (don't clone page table).
        let child_pt = if flags & CLONE_VM != 0 {
            parent_pt // Share page table
        } else {
            // Clone the user address space (like fork).
            match crate::mm::virt::clone_user_page_table(parent_pt) {
                Ok(pt) => pt,
                Err(_) => {
                    core::arch::asm!("sti", options(nomem, nostack));
                    return Err(SyscallError::ENOMEM);
                }
            }
        };

        // Allocate child kernel stack + guard page.
        let total_pages = crate::task::task::KERNEL_STACK_PAGES
            + crate::task::task::KERNEL_STACK_GUARD_PAGES;
        let child_alloc_frame = match crate::mm::phys::alloc_contiguous(total_pages) {
            Ok(f) => f,
            Err(_) => {
                if flags & CLONE_VM == 0 {
                    crate::mm::virt::free_page_table(child_pt, true);
                }
                core::arch::asm!("sti", options(nomem, nostack));
                return Err(SyscallError::ENOMEM);
            }
        };
        let child_alloc_base = child_alloc_frame.addr();
        let child_stack_base = child_alloc_base
            + (crate::task::task::KERNEL_STACK_GUARD_PAGES * crate::mm::phys::FRAME_SIZE) as u64;
        let child_stack_top =
            child_stack_base + crate::task::task::KERNEL_STACK_SIZE as u64;

        // Guard sentinel + zero usable stack.
        core::ptr::write_bytes(
            child_alloc_base as *mut u8,
            crate::task::task::KERNEL_STACK_GUARD_BYTE,
            crate::task::task::KERNEL_STACK_GUARD_PAGES * crate::mm::phys::FRAME_SIZE,
        );
        core::ptr::write_bytes(
            child_stack_base as *mut u8,
            0,
            crate::task::task::KERNEL_STACK_SIZE,
        );

        // Copy the syscall return frame (80 bytes at top of parent's kernel stack)
        // to the child's kernel stack so clone_child_return can SYSRET properly.
        let parent_stack_top = crate::task::scheduler::current_kernel_stack_top();
        const SYSRET_FRAME_SIZE: u64 = 80;
        core::ptr::write_bytes(
            (child_stack_top - SYSRET_FRAME_SIZE) as *mut u8,
            0,
            SYSRET_FRAME_SIZE as usize,
        );
        core::ptr::copy_nonoverlapping(
            (parent_stack_top - SYSRET_FRAME_SIZE) as *const u8,
            (child_stack_top - SYSRET_FRAME_SIZE) as *mut u8,
            SYSRET_FRAME_SIZE as usize,
        );

        // For threads, use the provided stack if given.
        if flags & CLONE_VM != 0 && !stack.is_null() {
            // Modify the copied SYSRET frame to use the new stack.
            // The SYSRET frame has user RSP at offset 16 (after pop rcx, r11).
            // Let's adjust the user RSP in the frame.
            let user_rsp_offset = 16;
            let frame_ptr = (child_stack_top - SYSRET_FRAME_SIZE + user_rsp_offset) as *mut u64;
            *frame_ptr = stack as u64;
        }

        // Gather parent state.
        let parent_pid = crate::task::scheduler::current_pid();
        let (pgid, session_id, creds, umask, name, name_len, cwd, cwd_len, fd_table) =
            crate::task::scheduler::with_current_task(|t| {
                (
                    t.pgid,
                    t.session_id,
                    t.creds,
                    t.umask,
                    t.name,
                    t.name_len,
                    t.cwd,
                    t.cwd_len,
                    if flags & CLONE_THREAD != 0 {
                        t.fd_table.clone_fds() // Share FDs for threads
                    } else {
                        t.fd_table.clone_fds() // For now, clone FDs
                    },
                )
            })
            .unwrap();

        let child_pid = if flags & CLONE_THREAD != 0 {
            // For threads, use same PID as parent (threads share PID)
            parent_pid
        } else {
            crate::task::process::alloc_user_pid()
        };

        // Build child TaskContext: when context-switched to, it enters
        // clone_child_return with RSP pointing at the SYSRET frame.
        let mut ctx = crate::task::context::TaskContext::new();
        ctx.rip = clone_child_return as u64;
        ctx.rsp = child_stack_top - SYSRET_FRAME_SIZE;

        let child_task = crate::task::task::Task {
            pid: child_pid,
            parent_pid: if flags & CLONE_THREAD != 0 { parent_pid } else { parent_pid },
            state: crate::task::task::TaskState::Created,
            context: ctx,
            kernel_stack_base: child_stack_base,
            page_table_phys: child_pt,
            exit_status: 0,
            signals: crate::task::signal::SignalState::new(),
            fd_table,
            pgid,
            session_id,
            creds,
            umask,
            name,
            name_len,
            cwd,
            cwd_len,
        };

        match crate::task::scheduler::spawn_forked(child_task) {
            Ok(pid) => {
                core::arch::asm!("sti", options(nomem, nostack));
                Ok(pid as i64)
            }
            Err(_) => {
                // Cleanup on failure: free guard + stack pages.
                if flags & CLONE_VM == 0 {
                    crate::mm::virt::free_page_table(child_pt, true);
                }
                let total_pages = crate::task::task::KERNEL_STACK_PAGES
                    + crate::task::task::KERNEL_STACK_GUARD_PAGES;
                for i in 0..total_pages {
                    let addr = child_alloc_base
                        + (i * crate::mm::phys::FRAME_SIZE) as u64;
                    let _ = crate::mm::phys::free_frame(
                        crate::mm::phys::PhysFrame::containing(addr),
                    );
                }
                core::arch::asm!("sti", options(nomem, nostack));
                Err(SyscallError::EAGAIN)
            }
        }
    }
}

/// Trampoline for the cloned child process/thread.
///
/// When context_switch first switches to the child, it jumps here.
/// RSP points to the copied SYSRET frame on the child's kernel stack.
/// We set RAX=0 (child return value), enable interrupts, then replay
/// the syscall_entry exit sequence.
#[unsafe(naked)]
unsafe extern "C" fn clone_child_return() {
    core::arch::naked_asm!(
        "xor eax, eax",   // RAX = 0 (clone return for child)
        "sti",            // Re-enable interrupts
        "add rsp, 8",     // Skip arg6 (r9)
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "pop rcx",        // User RIP
        "pop r11",        // User RFLAGS
        "pop rsp",        // User RSP
        "swapgs",
        "sysretq",
    );
}

// ─────────────────────────────────────────────────
// Syscall 27: sys_sigaction
// ─────────────────────────────────────────────────

/// Kernel representation of a signal action.
#[repr(C)]
pub struct KSigAction {
    pub handler: u64,   // SIG_DFL=0, SIG_IGN=1, or function pointer
    pub flags: u32,
    pub mask: u32,
}

/// Install or query a signal handler.
pub fn sys_sigaction(signum: i32, act: *const u8, oldact: *mut u8) -> SyscallResult {
    if signum <= 0 || signum > 31 {
        return Err(SyscallError::EINVAL);
    }
    // SIGKILL and SIGSTOP cannot be caught
    if signum == 9 || signum == 19 {
        return Err(SyscallError::EINVAL);
    }

    let sa_size = core::mem::size_of::<KSigAction>();

    if !oldact.is_null() {
        validate_user_ptr(oldact as u64, sa_size)?;
    }
    if !act.is_null() {
        validate_user_ptr(act as u64, sa_size)?;
    }

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));

        // Read old action
        if !oldact.is_null() {
            let old = crate::task::scheduler::with_current_task(|t| {
                t.signals.get_handler(signum as u8)
            })
            .unwrap_or(0);
            let old_sa = KSigAction {
                handler: old,
                flags: 0,
                mask: 0,
            };
            core::ptr::copy_nonoverlapping(
                &old_sa as *const KSigAction as *const u8,
                oldact,
                sa_size,
            );
        }

        // Set new action
        if !act.is_null() {
            let new_sa = &*(act as *const KSigAction);
            crate::task::scheduler::with_current_task_mut(|t| {
                t.signals.set_handler(signum as u8, new_sa.handler);
            });
        }

        core::arch::asm!("sti", options(nomem, nostack));
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 28: sys_sigreturn
// ─────────────────────────────────────────────────

/// Return from a signal handler (restores pre-signal context).
/// For now, this is a stub — signal delivery is default-action only.
pub fn sys_sigreturn() -> SyscallResult {
    // TODO: restore saved user context from signal frame
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 29: sys_poll
// ─────────────────────────────────────────────────

/// PollFd structure matching user-space layout.
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLERR: i16 = 0x0008;
const POLLHUP: i16 = 0x0010;
const POLLNVAL: i16 = 0x0020;

/// Poll file descriptors for readiness.
pub fn sys_poll(fds_ptr: *mut u8, nfds: u32, timeout_ms: i32) -> SyscallResult {
    if nfds > 256 {
        return Err(SyscallError::EINVAL);
    }
    let size = nfds as usize * core::mem::size_of::<PollFd>();
    if size > 0 {
        validate_user_ptr(fds_ptr as u64, size)?;
    }

    // Simple implementation: check all fds once, mark readable/writable.
    let mut ready = 0i64;
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        for i in 0..nfds as usize {
            let pfd = &mut *((fds_ptr as *mut PollFd).add(i));
            pfd.revents = 0;

            if pfd.fd < 0 {
                continue;
            }

            let fd_ok = crate::task::scheduler::with_current_fd_table(|fdt| {
                fdt.get(pfd.fd).is_ok()
            })
            .unwrap_or(false);

            if !fd_ok {
                pfd.revents = POLLNVAL;
                ready += 1;
                continue;
            }

            // For MVP: pipes/files are always ready for read/write.
            if pfd.events & POLLIN != 0 {
                pfd.revents |= POLLIN;
            }
            if pfd.events & POLLOUT != 0 {
                pfd.revents |= POLLOUT;
            }
            if pfd.revents != 0 {
                ready += 1;
            }
        }
        core::arch::asm!("sti", options(nomem, nostack));
    }

    let _ = timeout_ms; // TODO: blocking poll with timeout
    Ok(ready)
}

// ─────────────────────────────────────────────────
// Syscall 30: sys_getppid
// ─────────────────────────────────────────────────

/// Get the parent process ID.
pub fn sys_getppid() -> SyscallResult {
    Ok(crate::task::scheduler::current_parent_pid() as i64)
}

// ─────────────────────────────────────────────────
// Syscalls 31-36: UID/GID
// ─────────────────────────────────────────────────

/// Get real user ID.
pub fn sys_getuid() -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let uid = crate::task::scheduler::with_current_task(|t| t.creds.uid).unwrap_or(0);
        core::arch::asm!("sti", options(nomem, nostack));
        Ok(uid as i64)
    }
}
/// Get real group ID.
pub fn sys_getgid() -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let gid = crate::task::scheduler::with_current_task(|t| t.creds.gid).unwrap_or(0);
        core::arch::asm!("sti", options(nomem, nostack));
        Ok(gid as i64)
    }
}
/// Set user ID.
pub fn sys_setuid(uid: u32) -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let res = crate::task::scheduler::with_current_task_mut(|t| {
            if crate::security::capability::has_cap(
                &t.creds,
                crate::security::capability::CAP_SETUID,
            ) {
                t.creds.uid = uid;
                t.creds.euid = uid;
                return Ok(0);
            }
            // Non-root may only switch effective UID between current real/effective.
            if uid == t.creds.uid || uid == t.creds.euid {
                t.creds.euid = uid;
                Ok(0)
            } else {
                Err(SyscallError::EPERM)
            }
        })
        .unwrap_or(Err(SyscallError::ESRCH));
        core::arch::asm!("sti", options(nomem, nostack));
        res
    }
}
/// Set group ID.
pub fn sys_setgid(gid: u32) -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let res = crate::task::scheduler::with_current_task_mut(|t| {
            if crate::security::capability::has_cap(
                &t.creds,
                crate::security::capability::CAP_SETGID,
            ) {
                t.creds.gid = gid;
                t.creds.egid = gid;
                return Ok(0);
            }
            if gid == t.creds.gid || gid == t.creds.egid {
                t.creds.egid = gid;
                Ok(0)
            } else {
                Err(SyscallError::EPERM)
            }
        })
        .unwrap_or(Err(SyscallError::ESRCH));
        core::arch::asm!("sti", options(nomem, nostack));
        res
    }
}
/// Get effective user ID.
pub fn sys_geteuid() -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let euid = crate::task::scheduler::with_current_task(|t| t.creds.euid).unwrap_or(0);
        core::arch::asm!("sti", options(nomem, nostack));
        Ok(euid as i64)
    }
}
/// Get effective group ID.
pub fn sys_getegid() -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let egid = crate::task::scheduler::with_current_task(|t| t.creds.egid).unwrap_or(0);
        core::arch::asm!("sti", options(nomem, nostack));
        Ok(egid as i64)
    }
}

// ─────────────────────────────────────────────────
// Syscall 37: sys_nanosleep
// ─────────────────────────────────────────────────

/// Sleep for the specified duration.
pub fn sys_nanosleep(req: *const u8, _rem: *mut u8) -> SyscallResult {
    validate_user_ptr(req as u64, 16)?;
    let ts = unsafe { &*(req as *const Timespec) };
    let ms = ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000;
    let target = crate::interrupts::pit::uptime_ms() + ms;

    // Simple busy-yield sleep.
    while crate::interrupts::pit::uptime_ms() < target {
        crate::task::scheduler::yield_now();
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 38: sys_truncate
// ─────────────────────────────────────────────────

pub fn sys_truncate(_path: *const u8, _length: u64) -> SyscallResult {
    crate::serial::serial_println!("[ SYSC ] truncate() not implemented yet");
    Err(SyscallError::ENOSYS)
}

// ─────────────────────────────────────────────────
// Syscall 39: sys_fstat
// ─────────────────────────────────────────────────

/// Get file status by fd.
pub fn sys_fstat(fd: i32, buf: *mut u8) -> SyscallResult {
    validate_user_ptr(buf as u64, core::mem::size_of::<StatBuf>())?;

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            let file = fds.get(fd).map_err(map_vfs_error)?;
            let meta = file.inode.metadata().map_err(map_vfs_error)?;
            let stat = StatBuf {
                st_dev: 0,
                st_ino: meta.ino,
                st_mode: meta.file_type as u32 | meta.mode.0,
                st_nlink: meta.nlink,
                st_uid: meta.uid,
                st_gid: meta.gid,
                st_size: meta.size,
                st_atime: meta.atime,
                st_mtime: meta.mtime,
                st_ctime: meta.ctime,
                st_rdev_major: meta.dev_major,
                st_rdev_minor: meta.dev_minor,
            };
            core::ptr::copy_nonoverlapping(
                &stat as *const StatBuf as *const u8,
                buf,
                core::mem::size_of::<StatBuf>(),
            );
            Ok(0i64)
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}

// ─────────────────────────────────────────────────
// Syscall 40: sys_lseek
// ─────────────────────────────────────────────────

const SEEK_SET: i32 = 0;
const SEEK_CUR: i32 = 1;
const SEEK_END: i32 = 2;

/// Reposition read/write offset.
pub fn sys_lseek(fd: i32, offset: i64, whence: i32) -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            let file = fds.get(fd).map_err(map_vfs_error)?;
            let current = file.offset.load(core::sync::atomic::Ordering::Relaxed);
            let size = file.inode.metadata().map(|m| m.size).unwrap_or(0);

            let new_offset = match whence {
                SEEK_SET => {
                    if offset < 0 { return Err(SyscallError::EINVAL); }
                    offset as u64
                }
                SEEK_CUR => {
                    let r = current as i64 + offset;
                    if r < 0 { return Err(SyscallError::EINVAL); }
                    r as u64
                }
                SEEK_END => {
                    let r = size as i64 + offset;
                    if r < 0 { return Err(SyscallError::EINVAL); }
                    r as u64
                }
                _ => return Err(SyscallError::EINVAL),
            };
            file.offset.store(new_offset, core::sync::atomic::Ordering::Relaxed);
            Ok(new_offset as i64)
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}

// ─────────────────────────────────────────────────
// Syscall 41: sys_access
// ─────────────────────────────────────────────────

/// Check file accessibility.
pub fn sys_access(path: *const u8, _mode: u32) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    let (fs, ino) = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(path_str)
            .map_err(|_| SyscallError::ENOENT)?
    };
    if _mode == 0 {
        return Ok(0); // F_OK
    }

    let inode = fs.get_inode(ino).map_err(map_vfs_error)?;
    let meta = inode.metadata().map_err(map_vfs_error)?;
    if (_mode & 0b100) != 0 {
        require_dac_access(&meta, crate::security::dac::Access::Read)?;
    }
    if (_mode & 0b010) != 0 {
        require_dac_access(&meta, crate::security::dac::Access::Write)?;
    }
    if (_mode & 0b001) != 0 {
        require_dac_access(&meta, crate::security::dac::Access::Execute)?;
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscalls 42-44: chmod, chown, umask
// ─────────────────────────────────────────────────

pub fn sys_chmod(path: *const u8, mode: u32) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    let (fs, ino) = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(path_str)
            .map_err(map_vfs_error)?
    };
    let inode = fs.get_inode(ino).map_err(map_vfs_error)?;
    let mut meta = inode.metadata().map_err(map_vfs_error)?;
    let creds = current_creds();

    if creds.euid != meta.uid {
        require_cap(crate::security::capability::CAP_FOWNER)?;
    }

    meta.mode = crate::vfs::inode::FileMode::new(mode & 0o7777);
    inode.set_metadata(&meta).map_err(map_vfs_error)?;
    Ok(0)
}

pub fn sys_chown(path: *const u8, uid: u32, gid: u32) -> SyscallResult {
    let path_len = validate_user_string(path as u64)?;
    let path_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(path, path_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    let (fs, ino) = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(path_str)
            .map_err(map_vfs_error)?
    };
    let inode = fs.get_inode(ino).map_err(map_vfs_error)?;
    let mut meta = inode.metadata().map_err(map_vfs_error)?;

    require_cap(crate::security::capability::CAP_CHOWN)?;
    meta.uid = uid;
    meta.gid = gid;
    inode.set_metadata(&meta).map_err(map_vfs_error)?;
    Ok(0)
}

pub fn sys_umask(mask: u32) -> SyscallResult {
    let new_mask = mask & 0o777;
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let old = crate::task::scheduler::with_current_task_mut(|t| {
            let old = t.umask;
            t.umask = new_mask;
            old
        })
        .unwrap_or(0o022);
        core::arch::asm!("sti", options(nomem, nostack));
        Ok(old as i64)
    }
}

// ─────────────────────────────────────────────────
// Syscalls 45-48: link, symlink, readlink, rename (stubs/minimal)
// ─────────────────────────────────────────────────

pub fn sys_link(_old: *const u8, _new: *const u8) -> SyscallResult {
    crate::serial::serial_println!("[ SYSC ] link() not implemented yet");
    Err(SyscallError::ENOSYS)
}

pub fn sys_symlink(_target: *const u8, _linkpath: *const u8) -> SyscallResult {
    crate::serial::serial_println!("[ SYSC ] symlink() not implemented yet");
    Err(SyscallError::ENOSYS)
}

pub fn sys_readlink(_path: *const u8, _buf: *mut u8, _bufsiz: usize) -> SyscallResult {
    crate::serial::serial_println!("[ SYSC ] readlink() not implemented yet");
    Err(SyscallError::ENOSYS)
}

pub fn sys_rename(old: *const u8, new: *const u8) -> SyscallResult {
    let old_len = validate_user_string(old as u64)?;
    let new_len = validate_user_string(new as u64)?;
    let old_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(old, old_len))
            .map_err(|_| SyscallError::EINVAL)?
    };
    let new_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(new, new_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    // Only writable tmpfs/racfs filesystems supported.
    let mt = unsafe { crate::vfs::mount::mount_table() };
    let (mount, _) = mt.resolve(old_str).ok_or(SyscallError::ENOENT)?;
    let store = writable_store_from_mount(mount).ok_or(SyscallError::EACCES)?;

    let (old_parent_ino, _old_leaf) = store.split_parent_leaf(old_str).map_err(map_vfs_error)?;
    let old_parent_inode = mount.fs.get_inode(old_parent_ino).map_err(map_vfs_error)?;
    let old_parent_meta = old_parent_inode.metadata().map_err(map_vfs_error)?;
    require_dac_access(&old_parent_meta, crate::security::dac::Access::Write)?;
    require_dac_access(&old_parent_meta, crate::security::dac::Access::Execute)?;

    // Read old file content
    let (fs, ino) = {
        mt.lookup_path(old_str).map_err(|_| SyscallError::ENOENT)?
    };
    let inode = fs.get_inode(ino).map_err(map_vfs_error)?;
    let meta = inode.metadata().map_err(map_vfs_error)?;
    require_dac_access(&meta, crate::security::dac::Access::Read)?;
    let size = meta.size as usize;

    if size > 0 {
        let mut buf = alloc::vec![0u8; size];
        inode.read(0, &mut buf).map_err(map_vfs_error)?;

        // Create new file with same content.
        let (new_parent, new_leaf) = store.split_parent_leaf(new_str).map_err(map_vfs_error)?;
        let new_parent_inode = mount.fs.get_inode(new_parent).map_err(map_vfs_error)?;
        let new_parent_meta = new_parent_inode.metadata().map_err(map_vfs_error)?;
        require_dac_access(&new_parent_meta, crate::security::dac::Access::Write)?;
        require_dac_access(&new_parent_meta, crate::security::dac::Access::Execute)?;
        let _new_ino = store.create_file(new_parent, new_leaf).map_err(map_vfs_error)?;
        // Write via mount table lookup for the new path
        let (new_fs, new_ino_found) = {
            mt.lookup_path(new_str).map_err(|_| SyscallError::ENOENT)?
        };
        let new_inode = new_fs.get_inode(new_ino_found).map_err(map_vfs_error)?;
        new_inode.write(0, &buf).map_err(map_vfs_error)?;

        // Unlink old
        let (old_parent, old_leaf) = store.split_parent_leaf(old_str).map_err(map_vfs_error)?;
        store.unlink(old_parent, old_leaf).map_err(map_vfs_error)?;
    }

    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 49: sys_fcntl
// ─────────────────────────────────────────────────

pub fn sys_fcntl(fd: i32, cmd: i32, _arg: u64) -> SyscallResult {
    const F_GETFD: i32 = 1;
    const F_SETFD: i32 = 2;
    const F_GETFL: i32 = 3;
    const F_SETFL: i32 = 4;
    const F_DUPFD: i32 = 0;

    match cmd {
        F_DUPFD => {
            unsafe {
                core::arch::asm!("cli", options(nomem, nostack));
                let result = crate::task::scheduler::with_current_fd_table(|fds| {
                    fds.dup(fd).map(|fd| fd as i64).map_err(map_vfs_error)
                })
                .unwrap_or(Err(SyscallError::EBADF));
                core::arch::asm!("sti", options(nomem, nostack));
                result
            }
        }
        F_GETFD | F_GETFL => Ok(0),
        F_SETFD | F_SETFL => Ok(0),
        _ => Err(SyscallError::EINVAL),
    }
}

// ─────────────────────────────────────────────────
// Syscall 50: sys_isatty
// ─────────────────────────────────────────────────

pub fn sys_isatty(fd: i32) -> SyscallResult {
    // fd 0, 1, 2 are TTY
    if fd >= 0 && fd <= 2 {
        Ok(1)
    } else {
        Err(SyscallError::ENOTTY)
    }
}

// ─────────────────────────────────────────────────
// Syscalls 51-62: Socket stubs
// ─────────────────────────────────────────────────

pub fn sys_socket(domain: i32, stype: i32, protocol: i32) -> SyscallResult {
    let sid = crate::net::create_socket(domain, stype, protocol).map_err(map_net_error)?;
    let fd = alloc_fd_for_socket(sid)?;
    Ok(fd as i64)
}

pub fn sys_bind(fd: i32, addr: *const u8, len: u32) -> SyscallResult {
    let (port, ip) = parse_sockaddr_in(addr, len)?;
    if ip != 0 && ip != 0x7F00_0001 {
        return Err(SyscallError::EADDRINUSE);
    }
    let pid = crate::task::scheduler::current_pid();
    crate::net::bind(pid, fd, port).map_err(map_net_error)?;
    Ok(0)
}

pub fn sys_listen(fd: i32, backlog: i32) -> SyscallResult {
    let pid = crate::task::scheduler::current_pid();
    crate::net::listen(pid, fd, backlog).map_err(map_net_error)?;
    Ok(0)
}

pub fn sys_accept(fd: i32, addr: *mut u8, len: *mut u32) -> SyscallResult {
    let pid = crate::task::scheduler::current_pid();
    let accepted_sid = crate::net::accept(pid, fd).map_err(map_net_error)?;
    let accepted_fd = alloc_fd_for_socket(accepted_sid)?;
    let (port, ip) = crate::net::peername(pid, accepted_fd).map_err(map_net_error)?;
    write_sockaddr_in(addr, len, port, ip)?;
    Ok(accepted_fd as i64)
}

pub fn sys_connect(fd: i32, addr: *const u8, len: u32) -> SyscallResult {
    let (port, ip) = parse_sockaddr_in(addr, len)?;
    let pid = crate::task::scheduler::current_pid();

    // Loopback path: existing in-kernel SOCK_STREAM emulation.
    if ip == 0x7F00_0001 {
        crate::net::connect(pid, fd, port).map_err(map_net_error)?;
        return Ok(0);
    }

    // Real TCP. Resolve next-hop MAC (gateway for off-subnet).
    let ip_bytes = [(ip >> 24) as u8, (ip >> 16) as u8, (ip >> 8) as u8, ip as u8];
    let peer_mac = crate::net::stack::next_hop_mac(ip_bytes)
        .ok_or(SyscallError::ECONNREFUSED)?;
    let conn_id = crate::net::tcp::connect(ip_bytes, port, peer_mac)
        .map_err(|_| SyscallError::ECONNREFUSED)?;

    // Synchronous wait for the handshake to complete (timer IRQ drains RX
    // and feeds the state machine in the meantime). Re-enable interrupts —
    // SYSCALL entry zeroed IF, but we must let PIT fire to make progress.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
    let start = crate::interrupts::pit::ticks();
    let outcome: Result<(), SyscallError> = loop {
        match crate::net::tcp::state(conn_id) {
            Some(crate::net::tcp::State::Established) => break Ok(()),
            Some(crate::net::tcp::State::Closed) | None => break Err(SyscallError::ECONNREFUSED),
            _ => {
                if crate::interrupts::pit::ticks().saturating_sub(start) > 5000 {
                    break Err(SyscallError::ETIMEDOUT);
                }
                crate::net::stack::poll();
                core::hint::spin_loop();
            }
        }
    };
    unsafe { core::arch::asm!("cli", options(nomem, nostack)); }
    outcome?;
    crate::net::bind_fd_tcp(pid, fd, conn_id);
    Ok(0)
}

pub fn sys_send(fd: i32, buf: *const u8, len: usize, _flags: u32) -> SyscallResult {
    validate_user_ptr(buf as u64, len)?;
    let pid = crate::task::scheduler::current_pid();
    let data = unsafe { core::slice::from_raw_parts(buf, len) };
    if let Some(conn_id) = crate::net::tcp_id_by_fd(pid, fd) {
        crate::net::tcp::send(conn_id, data).map_err(|_| SyscallError::EPIPE)?;
        return Ok(len as i64);
    }
    let n = crate::net::send(pid, fd, data).map_err(map_net_error)?;
    Ok(n as i64)
}

pub fn sys_recv(fd: i32, buf: *mut u8, len: usize, _flags: u32) -> SyscallResult {
    validate_user_ptr(buf as u64, len)?;
    let pid = crate::task::scheduler::current_pid();
    let out = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    if let Some(conn_id) = crate::net::tcp_id_by_fd(pid, fd) {
        // Block until something arrives, EOF is observed, or 5 s elapse.
        // PIT must be running so timer_handler drains the NIC RX queue.
        unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
        let start = crate::interrupts::pit::ticks();
        let result: SyscallResult = loop {
            let n = crate::net::tcp::read(conn_id, out);
            if n > 0 { break Ok(n as i64); }
            match crate::net::tcp::state(conn_id) {
                None
                | Some(crate::net::tcp::State::Closed)
                | Some(crate::net::tcp::State::TimeWait)
                | Some(crate::net::tcp::State::CloseWait)
                | Some(crate::net::tcp::State::LastAck) => break Ok(0),
                _ => {}
            }
            if crate::interrupts::pit::ticks().saturating_sub(start) > 5000 {
                break Err(SyscallError::EAGAIN);
            }
            crate::net::stack::poll();
            core::hint::spin_loop();
        };
        unsafe { core::arch::asm!("cli", options(nomem, nostack)); }
        return result;
    }
    let n = crate::net::recv(pid, fd, out).map_err(map_net_error)?;
    Ok(n as i64)
}

pub fn sys_gethostbyname(name_ptr: *const u8, name_len: usize, ip_out: *mut u8) -> SyscallResult {
    if name_len == 0 || name_len > 253 { return Err(SyscallError::EINVAL); }
    validate_user_ptr(name_ptr as u64, name_len)?;
    validate_user_ptr(ip_out as u64, 4)?;
    let bytes = unsafe { core::slice::from_raw_parts(name_ptr, name_len) };
    let name = core::str::from_utf8(bytes).map_err(|_| SyscallError::EINVAL)?;
    match crate::net::stack::resolve(name) {
        Some(ip) => {
            unsafe { core::ptr::copy_nonoverlapping(ip.as_ptr(), ip_out, 4); }
            Ok(0)
        }
        None => Err(SyscallError::ETIMEDOUT),
    }
}

pub fn sys_shutdown(fd: i32, how: i32) -> SyscallResult {
    let pid = crate::task::scheduler::current_pid();
    crate::net::shutdown(pid, fd, how).map_err(map_net_error)?;
    Ok(0)
}

pub fn sys_getsockname(fd: i32, addr: *mut u8, len: *mut u32) -> SyscallResult {
    let pid = crate::task::scheduler::current_pid();
    let (port, ip) = crate::net::sockname(pid, fd).map_err(map_net_error)?;
    write_sockaddr_in(addr, len, port, ip)?;
    Ok(0)
}

pub fn sys_getpeername(fd: i32, addr: *mut u8, len: *mut u32) -> SyscallResult {
    let pid = crate::task::scheduler::current_pid();
    let (port, ip) = crate::net::peername(pid, fd).map_err(map_net_error)?;
    write_sockaddr_in(addr, len, port, ip)?;
    Ok(0)
}

pub fn sys_setsockopt(_fd: i32, _level: i32, _optname: i32, _optval: *const u8, _optlen: u32) -> SyscallResult {
    Ok(0)
}

pub fn sys_getsockopt(_fd: i32, _level: i32, _optname: i32, _optval: *mut u8, _optlen: *mut u32) -> SyscallResult {
    crate::serial::serial_println!(
        "[ SYSC ] getsockopt() not implemented yet: fd={}, level={}, opt={}",
        _fd,
        _level,
        _optname
    );
    Err(SyscallError::ENOSYS)
}

// ─────────────────────────────────────────────────
// Syscall 63: sys_waitpid
// ─────────────────────────────────────────────────

/// Wait for a specific child (or any child if pid == -1).
pub fn sys_waitpid(pid: i32, status_ptr: *mut i32, options: u32) -> SyscallResult {
    if !status_ptr.is_null() {
        validate_user_ptr(status_ptr as u64, 4)?;
    }
    sys_wait(pid, status_ptr as u64, options)
}

// ─────────────────────────────────────────────────
// Syscall 64: sys_pipe2
// ─────────────────────────────────────────────────

pub fn sys_pipe2(fds: *mut i32, _flags: u32) -> SyscallResult {
    sys_pipe(fds)
}

// ─────────────────────────────────────────────────
// Syscall 65: sys_uname
// ─────────────────────────────────────────────────

/// UTS name structure: 5 fields × 65 bytes each = 325 bytes.
pub fn sys_uname(buf: *mut u8) -> SyscallResult {
    validate_user_ptr(buf as u64, 325)?;
    unsafe {
        core::ptr::write_bytes(buf, 0, 325);
        // sysname
        let sysname = b"RacOS";
        core::ptr::copy_nonoverlapping(sysname.as_ptr(), buf, sysname.len());
        // nodename
        let nodename = b"racos";
        core::ptr::copy_nonoverlapping(nodename.as_ptr(), buf.add(65), nodename.len());
        // release
        let release = b"0.1.0";
        core::ptr::copy_nonoverlapping(release.as_ptr(), buf.add(130), release.len());
        // version
        let version = b"#1 RacOS";
        core::ptr::copy_nonoverlapping(version.as_ptr(), buf.add(195), version.len());
        // machine
        let machine = b"x86_64";
        core::ptr::copy_nonoverlapping(machine.as_ptr(), buf.add(260), machine.len());
    }
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 66-67: mount/umount
// ─────────────────────────────────────────────────

pub fn sys_mount(src: *const u8, target: *const u8, fstype: *const u8, _flags: u64, _data: *const u8) -> SyscallResult {
    require_cap(crate::security::capability::CAP_SYS_ADMIN)?;

    let src_str = if src.is_null() {
        None
    } else {
        let src_len = validate_user_string(src as u64)?;
        let src = unsafe {
            core::str::from_utf8(core::slice::from_raw_parts(src, src_len))
                .map_err(|_| SyscallError::EINVAL)?
        };
        Some(src)
    };

    let target_len = validate_user_string(target as u64)?;
    let fstype_len = validate_user_string(fstype as u64)?;

    let target_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(target, target_len))
            .map_err(|_| SyscallError::EINVAL)?
    };
    let fstype_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(fstype, fstype_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    if !target_str.starts_with('/') {
        return Err(SyscallError::EINVAL);
    }

    // Normalize target: drop trailing slash except for root.
    let target_norm = if target_str.len() > 1 {
        target_str.trim_end_matches('/')
    } else {
        target_str
    };

    // Mount point must exist prior to mount.
    let _ = unsafe {
        crate::vfs::mount::mount_table()
            .lookup_path(target_norm)
            .map_err(map_vfs_error)?
    };

    let fs: alloc::sync::Arc<dyn crate::vfs::mount::Filesystem> = match fstype_str {
        "tmpfs" => {
            // Reuse the global tmpfs instance to keep writable syscall paths coherent.
            let tmpfs = unsafe { crate::vfs::tmpfs::instance().clone() };
            crate::vfs::tmpfs::TmpfsFilesystem::new(tmpfs)
                as alloc::sync::Arc<dyn crate::vfs::mount::Filesystem>
        }
        "racfs" => {
            // Reuse the global racfs instance (block-device-backed).
            let racfs = unsafe { crate::vfs::racfs::instance().clone() };
            crate::vfs::racfs::RacfsFilesystem::new(racfs)
                as alloc::sync::Arc<dyn crate::vfs::mount::Filesystem>
        }
        "proc" | "procfs" => {
            let procfs = crate::vfs::procfs::Procfs::new();
            crate::vfs::procfs::ProcFilesystem::new(procfs)
                as alloc::sync::Arc<dyn crate::vfs::mount::Filesystem>
        }
        "dev" | "devfs" => {
            let mut devfs = crate::vfs::devfs::Devfs::new();
            devfs.register_defaults();
            crate::vfs::devfs::DevfsFilesystem::new(devfs)
                as alloc::sync::Arc<dyn crate::vfs::mount::Filesystem>
        }
        "fat" | "fat32" => {
            let src = src_str.ok_or(SyscallError::EINVAL)?;
            if src.is_empty() {
                return Err(SyscallError::EINVAL);
            }

            let dev_name = src.strip_prefix("/dev/").unwrap_or(src);
            let dev = crate::drivers::block::find(dev_name).ok_or(SyscallError::ENOENT)?;
            let fat32 = crate::vfs::fat32::Fat32Fs::new(dev).map_err(map_vfs_error)?;
            crate::vfs::fat32::Fat32Filesystem::new(fat32)
                as alloc::sync::Arc<dyn crate::vfs::mount::Filesystem>
        }
        _ => return Err(SyscallError::EINVAL),
    };

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        crate::vfs::mount::mount_table().mount(target_norm, fs);
        core::arch::asm!("sti", options(nomem, nostack));
    }
    Ok(0)
}

pub fn sys_umount(target: *const u8) -> SyscallResult {
    require_cap(crate::security::capability::CAP_SYS_ADMIN)?;

    let target_len = validate_user_string(target as u64)?;
    let target_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(target, target_len))
            .map_err(|_| SyscallError::EINVAL)?
    };
    if !target_str.starts_with('/') {
        return Err(SyscallError::EINVAL);
    }

    let target_norm = if target_str.len() > 1 {
        target_str.trim_end_matches('/')
    } else {
        target_str
    };

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::vfs::mount::mount_table()
            .umount(target_norm)
            .map_err(map_vfs_error)
            .map(|_| 0i64);
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}

// ─────────────────────────────────────────────────
// Syscall 79: sys_mkfs
// ─────────────────────────────────────────────────

/// Format a block device with the given filesystem type.
///
/// `src` is a block device name (raw "sda" or path "/dev/sda"); `fstype`
/// is currently only "racfs". Refuses if the device backs an active mount —
/// the caller must `umount` first.
pub fn sys_mkfs(
    src: *const u8, src_len: usize,
    fstype: *const u8, fstype_len: usize,
) -> SyscallResult {
    require_cap(crate::security::capability::CAP_SYS_ADMIN)?;

    if src_len == 0 || src_len > 64 || fstype_len == 0 || fstype_len > 16 {
        return Err(SyscallError::EINVAL);
    }
    validate_user_ptr(src as u64, src_len)?;
    validate_user_ptr(fstype as u64, fstype_len)?;
    let src_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(src, src_len))
            .map_err(|_| SyscallError::EINVAL)?
    };
    let fstype_str = unsafe {
        core::str::from_utf8(core::slice::from_raw_parts(fstype, fstype_len))
            .map_err(|_| SyscallError::EINVAL)?
    };

    let dev_name = src_str.strip_prefix("/dev/").unwrap_or(src_str);

    // Safety: refuse if the device is currently mounted somewhere. Without a
    // proper device→mount map we use a small hard-coded table; the kernel
    // mounts ram0 at /var and sda at /mnt by convention.
    let mt = unsafe { crate::vfs::mount::mount_table() };
    let busy_path = match dev_name {
        "sda"  => Some("/mnt"),
        "ram0" => Some("/var"),
        _      => None,
    };
    if let Some(path) = busy_path {
        if mt.is_mounted(path) {
            return Err(SyscallError::EADDRINUSE); // best fit for "device busy"
        }
    }

    let dev = crate::drivers::block::find(dev_name).ok_or(SyscallError::ENOENT)?;

    match fstype_str {
        "racfs" => {
            crate::vfs::racfs::Racfs::format_and_new(dev)
                .map(|_| 0i64)
                .map_err(|_| SyscallError::EIO)
        }
        _ => Err(SyscallError::EINVAL),
    }
}

// ─────────────────────────────────────────────────
// Syscall 68: sys_mprotect
// ─────────────────────────────────────────────────

pub fn sys_mprotect(_addr: u64, _len: usize, _prot: u32) -> SyscallResult {
    // Stub: succeed silently
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 69-70: fsync, ftruncate (stubs)
// ─────────────────────────────────────────────────

pub fn sys_fsync(fd: i32) -> SyscallResult {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let result = crate::task::scheduler::with_current_fd_table(|fds| {
            let file = fds.get(fd).map_err(map_vfs_error)?;
            match file.inode.sync() {
                Ok(()) => Ok(0i64),
                // In-memory and virtual filesystems can report no-op sync.
                Err(VfsError::NotImplemented) => Ok(0i64),
                Err(e) => Err(map_vfs_error(e)),
            }
        })
        .unwrap_or(Err(SyscallError::EBADF));
        core::arch::asm!("sti", options(nomem, nostack));
        result
    }
}
pub fn sys_ftruncate(_fd: i32, _length: u64) -> SyscallResult { Ok(0) }

// ─────────────────────────────────────────────────
// Syscalls 71-72: writev, readv
// ─────────────────────────────────────────────────

/// iovec structure for scatter/gather I/O.
#[repr(C)]
struct IoVec {
    iov_base: u64,
    iov_len: u64,
}

pub fn sys_writev(fd: i32, iov_ptr: *const u8, iovcnt: i32) -> SyscallResult {
    if iovcnt <= 0 || iovcnt > 1024 {
        return Err(SyscallError::EINVAL);
    }
    let iov_size = iovcnt as usize * core::mem::size_of::<IoVec>();
    validate_user_ptr(iov_ptr as u64, iov_size)?;

    let mut total = 0i64;
    for i in 0..iovcnt as usize {
        let iov = unsafe { &*((iov_ptr as *const IoVec).add(i)) };
        if iov.iov_len == 0 {
            continue;
        }
        validate_user_ptr(iov.iov_base, iov.iov_len as usize)?;
        let buf = unsafe {
            core::slice::from_raw_parts(iov.iov_base as *const u8, iov.iov_len as usize)
        };
        match sys_write(fd, buf.as_ptr(), buf.len()) {
            Ok(n) => total += n,
            Err(e) => {
                if total > 0 {
                    return Ok(total);
                }
                return Err(e);
            }
        }
    }
    Ok(total)
}

pub fn sys_readv(fd: i32, iov_ptr: *const u8, iovcnt: i32) -> SyscallResult {
    if iovcnt <= 0 || iovcnt > 1024 {
        return Err(SyscallError::EINVAL);
    }
    let iov_size = iovcnt as usize * core::mem::size_of::<IoVec>();
    validate_user_ptr(iov_ptr as u64, iov_size)?;

    let mut total = 0i64;
    for i in 0..iovcnt as usize {
        let iov = unsafe { &*((iov_ptr as *const IoVec).add(i)) };
        if iov.iov_len == 0 {
            continue;
        }
        validate_user_ptr(iov.iov_base, iov.iov_len as usize)?;
        match sys_read(fd, iov.iov_base as *mut u8, iov.iov_len as usize) {
            Ok(n) => {
                total += n;
                if (n as u64) < iov.iov_len {
                    break; // Short read
                }
            }
            Err(e) => {
                if total > 0 {
                    return Ok(total);
                }
                return Err(e);
            }
        }
    }
    Ok(total)
}

// ─────────────────────────────────────────────────
// Syscall 73: sys_sched_yield
// ─────────────────────────────────────────────────

pub fn sys_sched_yield() -> SyscallResult {
    crate::task::scheduler::yield_now();
    Ok(0)
}

// ─────────────────────────────────────────────────
// Syscall 74: sys_reboot
// ─────────────────────────────────────────────────

pub fn sys_reboot(cmd: u32) -> SyscallResult {
    require_cap(crate::security::capability::CAP_SYS_BOOT)?;

    const REBOOT_POWER_OFF: u32 = 0x4321;
    const REBOOT_RESTART: u32 = 0x1234;

    match cmd {
        REBOOT_POWER_OFF => {
            crate::serial::serial_println!("[REBOOT] Power-off requested");
            // Use QEMU-specific port for shutdown
            unsafe {
                core::arch::asm!(
                    "out dx, ax",
                    in("dx") 0x604u16,
                    in("ax") 0x2000u16,
                    options(nomem, nostack),
                );
            }
            loop {
                unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
            }
        }
        REBOOT_RESTART => {
            crate::serial::serial_println!("[REBOOT] Restart requested");
            // Triple fault for reboot
            unsafe {
                core::arch::asm!(
                    "lidt [rax]",
                    in("rax") 0u64,
                    options(nostack),
                );
                core::arch::asm!("int3", options(nomem, nostack));
            }
            loop {
                unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
            }
        }
        _ => Err(SyscallError::EINVAL),
    }
}

// ─────────────────────────────────────────────────
// Syscall 75: sys_hostname
// ─────────────────────────────────────────────────

static mut HOSTNAME: [u8; 256] = {
    let mut buf = [0u8; 256];
    buf[0] = b'r'; buf[1] = b'a'; buf[2] = b'c'; buf[3] = b'o'; buf[4] = b's';
    buf
};
static mut HOSTNAME_LEN: usize = 5;

pub fn sys_hostname(buf: *mut u8, len: usize, set: u32) -> SyscallResult {
    if set != 0 {
        // Set hostname — requires CAP_SYS_ADMIN
        require_cap(crate::security::capability::CAP_SYS_ADMIN)?;
        validate_user_ptr(buf as u64, len)?;
        let new_len = len.min(255);
        unsafe {
            let hname = &mut *core::ptr::addr_of_mut!(HOSTNAME);
            core::ptr::copy_nonoverlapping(buf as *const u8, hname.as_mut_ptr(), new_len);
            *core::ptr::addr_of_mut!(HOSTNAME_LEN) = new_len;
        }
        Ok(0)
    } else {
        // Get hostname
        validate_user_ptr(buf as u64, len)?;
        unsafe {
            let hname = &*core::ptr::addr_of!(HOSTNAME);
            let hlen = *core::ptr::addr_of!(HOSTNAME_LEN);
            let copy_len = hlen.min(len.saturating_sub(1));
            core::ptr::copy_nonoverlapping(hname.as_ptr(), buf, copy_len);
            *buf.add(copy_len) = 0;
            Ok(copy_len as i64)
        }
    }
}

// ─────────────────────────────────────────────────
// Syscall 76: sys_getrandom
// ─────────────────────────────────────────────────

/// Fill buffer with pseudo-random bytes.
///
/// Uses a per-call seeded LCG. This is NOT cryptographically secure,
/// but is adequate for the kernel MVP. Mixes TSC, PIT, and PID for entropy.
pub fn sys_getrandom(buf: *mut u8, len: usize, _flags: u32) -> SyscallResult {
    validate_user_ptr(buf as u64, len)?;
    // Mix multiple entropy sources
    let tsc: u64;
    unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi);
        tsc = (hi as u64) << 32 | lo as u64;
    }
    let pit = crate::interrupts::pit::uptime_ms();
    let pid = crate::task::scheduler::current_pid() as u64;
    let mut state = tsc
        .wrapping_mul(6364136223846793005)
        .wrapping_add(pit)
        .wrapping_mul(2862933555777941757)
        .wrapping_add(pid);
    unsafe {
        for i in 0..len {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *buf.add(i) = (state >> 33) as u8;
        }
    }
    Ok(len as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::inode::VfsError;

    #[test]
    fn test_map_vfs_error() {
        assert_eq!(map_vfs_error(VfsError::NotFound), SyscallError::ENOENT);
        assert_eq!(map_vfs_error(VfsError::PermissionDenied), SyscallError::EACCES);
        assert_eq!(map_vfs_error(VfsError::NotADirectory), SyscallError::ENOTDIR);
        assert_eq!(map_vfs_error(VfsError::IsADirectory), SyscallError::EISDIR);
        assert_eq!(map_vfs_error(VfsError::AlreadyExists), SyscallError::EEXIST);
        assert_eq!(map_vfs_error(VfsError::NoSpace), SyscallError::ENOSPC);
        assert_eq!(map_vfs_error(VfsError::InvalidArgument), SyscallError::EINVAL);
        assert_eq!(map_vfs_error(VfsError::BadFileDescriptor), SyscallError::EBADF);
        assert_eq!(map_vfs_error(VfsError::TooManyOpenFiles), SyscallError::EMFILE);
        assert_eq!(map_vfs_error(VfsError::BrokenPipe), SyscallError::EIO);
        assert_eq!(map_vfs_error(VfsError::WouldBlock), SyscallError::EAGAIN);
        assert_eq!(map_vfs_error(VfsError::IoError), SyscallError::EIO);
        assert_eq!(map_vfs_error(VfsError::NotImplemented), SyscallError::EIO);
    }
}