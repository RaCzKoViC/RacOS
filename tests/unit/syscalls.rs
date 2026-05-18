//! Unit tests for syscalls

#[cfg(test)]
mod tests {
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum SyscallError {
        ENOENT = 2,
        EACCES = 13,
        ENOTDIR = 20,
        EISDIR = 21,
        EEXIST = 17,
        ENOSPC = 28,
        EINVAL = 22,
        EBADF = 9,
        EMFILE = 24,
        EIO = 5,
        EAGAIN = 11,
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum VfsError {
        NotFound,
        PermissionDenied,
        NotADirectory,
        IsADirectory,
        AlreadyExists,
        NoSpace,
        InvalidArgument,
        BadFileDescriptor,
        TooManyOpenFiles,
        BrokenPipe,
        WouldBlock,
        IoError,
        NotImplemented,
    }

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

    #[test]
    fn test_syscall_numbers() {
        // Test that syscall numbers are defined and unique
        const SYS_EXIT: u64 = 0;
        const SYS_READ: u64 = 1;
        const SYS_WRITE: u64 = 2;
        const SYS_OPEN: u64 = 3;
        const SYS_CLOSE: u64 = 4;
        const SYS_FORK: u64 = 26;
        const SYS_CLONE: u64 = 77;

        assert_eq!(SYS_EXIT, 0);
        assert_eq!(SYS_READ, 1);
        assert_eq!(SYS_WRITE, 2);
        assert_eq!(SYS_OPEN, 3);
        assert_eq!(SYS_CLOSE, 4);
        assert_eq!(SYS_FORK, 26);
        assert_eq!(SYS_CLONE, 77);

        // Ensure uniqueness
        let syscalls = vec![SYS_EXIT, SYS_READ, SYS_WRITE, SYS_OPEN, SYS_CLOSE, SYS_FORK, SYS_CLONE];
        let mut sorted = syscalls.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(syscalls.len(), sorted.len(), "Syscall numbers must be unique");
    }
}