// RaCore — Discretionary access control checks (Phase C3)

use crate::task::task::Credentials;
use crate::vfs::inode::InodeMetadata;

#[derive(Clone, Copy)]
pub enum Access {
    Read,
    Write,
    Execute,
}

/// May `creds` execute this inode?
///
/// Only a regular file with at least one execute bit runs. The second half
/// is why this is not simply `can_access(.., Execute)`: CAP_DAC_OVERRIDE
/// lets a process read and write anything, but a file nobody may execute
/// is not executable for anyone - the same rule Linux applies, and the
/// reason `chmod -x` means something even to root.
#[inline]
pub fn can_exec(creds: &Credentials, meta: &InodeMetadata) -> bool {
    if meta.file_type != crate::vfs::inode::FileType::Regular {
        return false;
    }
    if (meta.mode.0 & 0o111) == 0 {
        return false;
    }
    can_access(creds, meta, Access::Execute)
}

#[inline]
pub fn can_access(creds: &Credentials, meta: &InodeMetadata, access: Access) -> bool {
    if crate::security::capability::has_cap(creds, crate::security::capability::CAP_DAC_OVERRIDE) {
        if matches!(access, Access::Execute)
            && meta.file_type == crate::vfs::inode::FileType::Regular
        {
            return (meta.mode.0 & 0o111) != 0;
        }
        return true;
    }

    let perm_bits = if creds.euid == meta.uid {
        (meta.mode.0 >> 6) & 0b111
    } else if creds.egid == meta.gid || creds.gid == meta.gid {
        (meta.mode.0 >> 3) & 0b111
    } else {
        meta.mode.0 & 0b111
    };

    match access {
        Access::Read => (perm_bits & 0b100) != 0,
        Access::Write => (perm_bits & 0b010) != 0,
        Access::Execute => (perm_bits & 0b001) != 0,
    }
}
