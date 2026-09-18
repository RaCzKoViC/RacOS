// RaCore — Capability bitmask system (Phase C2)

use crate::task::task::Credentials;

pub const CAP_CHOWN: u8 = 0;
pub const CAP_DAC_OVERRIDE: u8 = 1;
pub const CAP_FOWNER: u8 = 2;
pub const CAP_SETUID: u8 = 3;
pub const CAP_SETGID: u8 = 4;
pub const CAP_SYS_ADMIN: u8 = 5;
pub const CAP_SYS_BOOT: u8 = 6;
/// Signal a process whose real/effective UID differs from the sender's.
pub const CAP_KILL: u8 = 7;

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

/// Reduce `creds`' capability masks for a UID change from `old_uid` /
/// `old_euid` to the UIDs already written into `creds`.
///
/// This is the policy Linux calls `cap_emulate_setxuid`, minus the saved
/// UID, which RaCore does not keep:
///
/// * leaving root entirely (the real UID stops being 0) clears the
///   permitted set, and with it everything the process could ever regain;
/// * dropping the effective UID from root clears the effective set;
/// * raising the effective UID back to root restores effective from
///   permitted - which, after the first rule, is empty for a process that
///   left root for good.
///
/// Without this a process that "dropped privileges" kept every
/// capability, because `has_cap` reads the mask: `setuid(1000)` followed
/// by `setuid(0)` was a round trip.
pub fn drop_for_uid_change(old_uid: u32, old_euid: u32, creds: &mut Credentials) {
    if old_uid == 0 && creds.uid != 0 {
        creds.cap_permitted = 0;
        creds.cap_inheritable = 0;
        creds.cap_effective = 0;
        return;
    }
    if old_euid == 0 && creds.euid != 0 {
        creds.cap_effective = 0;
    } else if old_euid != 0 && creds.euid == 0 {
        creds.cap_effective = creds.cap_permitted;
    }
}
