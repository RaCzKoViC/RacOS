// RaCore — Discretionary access control checks (Phase C3)

use crate::task::task::Credentials;
use crate::vfs::inode::InodeMetadata;

#[derive(Clone, Copy)]
pub enum Access {
    Read,
    Write,
    Execute,
}

#[inline]
pub fn can_access(creds: &Credentials, meta: &InodeMetadata, access: Access) -> bool {
    if crate::security::capability::has_cap(creds, crate::security::capability::CAP_DAC_OVERRIDE)
    {
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
