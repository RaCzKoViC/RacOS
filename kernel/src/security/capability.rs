// RaCore — Capability bitmask system (Phase C2)

use crate::task::task::Credentials;

pub const CAP_CHOWN: u8 = 0;
pub const CAP_DAC_OVERRIDE: u8 = 1;
pub const CAP_FOWNER: u8 = 2;
pub const CAP_SETUID: u8 = 3;
pub const CAP_SETGID: u8 = 4;
pub const CAP_SYS_ADMIN: u8 = 5;
pub const CAP_SYS_BOOT: u8 = 6;

#[inline]
pub const fn cap_mask(cap: u8) -> u64 {
    1u64 << cap
}

#[inline]
pub fn has_cap(creds: &Credentials, cap: u8) -> bool {
    // Root UID is still an unconditional capability superset in MVP.
    if creds.euid == 0 {
        return true;
    }
    (creds.cap_effective & cap_mask(cap)) != 0
}
