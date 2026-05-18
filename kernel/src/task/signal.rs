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
