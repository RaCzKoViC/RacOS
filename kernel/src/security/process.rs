// RaCore - authorization between processes (signals)

use crate::task::task::Credentials;

/// May `sender` signal `target`?
///
/// The POSIX rule, minus the saved UID we do not keep: the real or
/// effective UID of the sender must match the real or effective UID of
/// the target, unless the sender holds CAP_KILL. SIGCONT is the usual
/// exception - any process in the same session may continue another,
/// which is what lets a shell resume a job it did not start.
///
/// Until 2026-09 there was no check at all: any process could signal any
/// other, init included.
pub fn can_signal(
    sender: &Credentials,
    target: &Credentials,
    same_session: bool,
    sig: crate::task::signal::Signal,
) -> bool {
    if crate::security::capability::has_cap(sender, crate::security::capability::CAP_KILL) {
        return true;
    }
    if sender.uid == target.uid
        || sender.uid == target.euid
        || sender.euid == target.uid
        || sender.euid == target.euid
    {
        return true;
    }
    same_session && matches!(sig, crate::task::signal::Signal::SIGCONT)
}
