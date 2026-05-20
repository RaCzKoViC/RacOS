// RaCore — Signal model
//
// Signals are asynchronous notifications sent to processes.
// Design (ADR-005, ADR-006):
// - Signals are represented as a bitmask (pending: u32)
// - Delivery happens on return from syscall or interrupt
// - Handlers are SIG_DFL or SIG_IGN (custom handlers: post-MVP)
//
// Priority signals for MVP:
//   SIGINT  ( 2) — Ctrl-C: terminate foreground process
//   SIGTERM ( 9) — polite termination request
//   SIGKILL (15) — unconditional kill (cannot be caught)
//   SIGCHLD (17) — child process state change
//   SIGWINCH(28) — terminal window resize

/// Signal numbers (POSIX-compatible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Signal {
    SIGHUP  =  1,
    SIGINT  =  2,
    SIGQUIT =  3,
    SIGILL  =  4,
    SIGTRAP =  5,
    SIGABRT =  6,
    SIGFPE  =  8,
    SIGKILL =  9,
    SIGSEGV = 11,
    SIGPIPE = 13,
    SIGALRM = 14,
    SIGTERM = 15,
    SIGCHLD = 17,
    SIGCONT = 18,
    SIGSTOP = 19,
    SIGTSTP = 20,
    SIGTTIN = 21,
    SIGTTOU = 22,
    SIGWINCH = 28,
}

impl Signal {
    /// Convert from raw u8.
    pub fn from_u8(n: u8) -> Option<Self> {
        match n {
             1 => Some(Self::SIGHUP),
             2 => Some(Self::SIGINT),
             3 => Some(Self::SIGQUIT),
             4 => Some(Self::SIGILL),
             5 => Some(Self::SIGTRAP),
             6 => Some(Self::SIGABRT),
             8 => Some(Self::SIGFPE),
             9 => Some(Self::SIGKILL),
            11 => Some(Self::SIGSEGV),
            13 => Some(Self::SIGPIPE),
            14 => Some(Self::SIGALRM),
            15 => Some(Self::SIGTERM),
            17 => Some(Self::SIGCHLD),
            18 => Some(Self::SIGCONT),
            19 => Some(Self::SIGSTOP),
            20 => Some(Self::SIGTSTP),
            21 => Some(Self::SIGTTIN),
            22 => Some(Self::SIGTTOU),
            28 => Some(Self::SIGWINCH),
            _  => None,
        }
    }

    /// Bitmask for this signal (1 << signal_number).
    #[inline]
    pub fn mask(self) -> u32 {
        1u32 << (self as u8)
    }
}

/// SIG_DFL — use the default action.
pub const SIG_DFL: u64 = 0;
/// SIG_IGN — ignore the signal.
pub const SIG_IGN: u64 = 1;

/// Per-process signal state.
#[derive(Debug, Clone, Copy)]
pub struct SignalState {
    /// Bitmask of pending signals.
    pub pending: u32,
    /// Bitmask of blocked (masked) signals.
    pub blocked: u32,
    /// Per-signal handler addresses (0 = SIG_DFL, 1 = SIG_IGN, other = user fn).
    pub handlers: [u64; 32],
}

impl SignalState {
    pub const fn new() -> Self {
        SignalState { pending: 0, blocked: 0, handlers: [0; 32] }
    }

    /// Post a signal to this process.
    #[inline]
    pub fn send(&mut self, sig: Signal) {
        self.pending |= sig.mask();
    }

    /// Return true if any unblocked signal is pending.
    #[inline]
    pub fn has_pending(&self) -> bool {
        self.pending & !self.blocked != 0
    }

    /// Consume and return the lowest-numbered pending unblocked signal, if any.
    pub fn take_pending(&mut self) -> Option<Signal> {
        let deliverable = self.pending & !self.blocked;
        if deliverable == 0 {
            return None;
        }
        let bit = deliverable.trailing_zeros() as u8;
        let sig = Signal::from_u8(bit)?;
        self.pending &= !sig.mask();
        Some(sig)
    }

    /// Get the handler address for a signal number.
    pub fn get_handler(&self, sig_num: u8) -> u64 {
        if (sig_num as usize) < 32 {
            self.handlers[sig_num as usize]
        } else {
            SIG_DFL
        }
    }

    /// Set the handler address for a signal number.
    pub fn set_handler(&mut self, sig_num: u8, handler: u64) {
        if (sig_num as usize) < 32 {
            self.handlers[sig_num as usize] = handler;
        }
    }

    /// Default action for a signal.
    pub fn default_action(sig: Signal) -> SignalAction {
        match sig {
            Signal::SIGCHLD | Signal::SIGCONT | Signal::SIGWINCH => SignalAction::Ignore,
            Signal::SIGSTOP | Signal::SIGTSTP | Signal::SIGTTIN | Signal::SIGTTOU => SignalAction::Stop,
            _ => SignalAction::Terminate,
        }
    }
}

/// Default signal disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    Terminate,
    Ignore,
    Stop,
    Continue,
}

/// Frame written to the user stack when delivering a signal with a user
/// handler. `sys_sigreturn` consumes it to restore the interrupted context.
///
/// Layout is `#[repr(C)]` because user-space VDSO code may read it.
///
/// Field order intentionally mirrors the order syscall/interrupt entry
/// pushes general-purpose registers, then the interrupted instruction
/// state, then signal bookkeeping. Sizes (with `#[repr(C)]`):
///   - 15 × u64 GPRs                              = 120 bytes  (offsets   0..120)
///   - rip, rsp, rflags  (3 × u64)                =  24 bytes  (offsets 120..144)
///   - saved_sigmask     (u64)                    =   8 bytes  (offsets 144..152)
///   - signal_number     (u32)                    =   4 bytes  (offsets 152..156)
///   - _pad              (u32)                    =   4 bytes  (offsets 156..160)
/// Total: 160 bytes, already 16-byte aligned.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct SignalFrame {
    // Saved GPRs in the order pushed by the syscall entry path
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8:  u64,
    pub r9:  u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    // Interrupted instruction state
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
    // Signal bookkeeping
    pub saved_sigmask: u64,
    pub signal_number: u32,
    pub _pad: u32,
}

impl SignalFrame {
    /// Total bytes a `SignalFrame` occupies on the user stack, rounded up
    /// to 16 bytes so the C ABI's pre-`call` stack alignment guarantee is
    /// satisfied when the kernel hands control to the user handler.
    pub const fn aligned_size() -> usize {
        (core::mem::size_of::<SignalFrame>() + 15) / 16 * 16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_frame_size_is_160() {
        // 18 × u64 (GPRs + rip/rsp/rflags) + 1 × u64 (saved_sigmask)
        //   + 1 × u32 (signal_number) + 1 × u32 (_pad) = 160 bytes.
        assert_eq!(core::mem::size_of::<SignalFrame>(), 160);
    }

    #[test]
    fn signal_frame_aligned_size_is_160() {
        // 160 is already a multiple of 16, so aligned_size == size_of.
        assert_eq!(SignalFrame::aligned_size(), 160);
    }

    #[test]
    fn signal_frame_alignment_is_at_least_8() {
        // `#[repr(C)]` on a struct whose largest field is u64 yields 8-byte
        // alignment. The 16-byte runtime alignment is achieved by stack
        // adjustment (see `aligned_size`), not by the struct itself.
        assert_eq!(core::mem::align_of::<SignalFrame>(), 8);
    }

    #[test]
    fn signal_frame_field_offsets() {
        let f = SignalFrame::default();
        let base = &f as *const SignalFrame as usize;
        let off = |addr: usize| addr - base;

        // GPR block: rax..r15 are 15 consecutive u64s starting at 0.
        assert_eq!(off(&f.rax as *const u64 as usize), 0);
        assert_eq!(off(&f.rbx as *const u64 as usize), 8);
        assert_eq!(off(&f.r15 as *const u64 as usize), 14 * 8);

        // Interrupted instruction state.
        assert_eq!(off(&f.rip    as *const u64 as usize), 15 * 8); // 120
        assert_eq!(off(&f.rsp    as *const u64 as usize), 16 * 8); // 128
        assert_eq!(off(&f.rflags as *const u64 as usize), 17 * 8); // 136

        // Signal bookkeeping.
        assert_eq!(off(&f.saved_sigmask as *const u64 as usize), 18 * 8); // 144
        assert_eq!(off(&f.signal_number as *const u32 as usize), 152);
        assert_eq!(off(&f._pad          as *const u32 as usize), 156);
    }

    #[test]
    fn signal_frame_default_is_zeroed() {
        let f = SignalFrame::default();
        assert_eq!(f.rax, 0);
        assert_eq!(f.rip, 0);
        assert_eq!(f.rsp, 0);
        assert_eq!(f.rflags, 0);
        assert_eq!(f.saved_sigmask, 0);
        assert_eq!(f.signal_number, 0);
        assert_eq!(f._pad, 0);
    }
}
