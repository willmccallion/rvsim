//! The code a run ends with, which a device or the harness sets and the
//! simulator takes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A slot for the run's exit code, shared by every clone: the devices that
/// end a run (HTIF, `SysCon`) signal into it and the simulator takes from
/// it. Any `u64` is a code; whether one is pending is kept apart.
#[derive(Clone, Debug, Default)]
pub struct ExitSignal(Arc<Slot>);

#[derive(Debug, Default)]
struct Slot {
    code: AtomicU64,
    pending: AtomicBool,
}

impl ExitSignal {
    /// A slot with no exit pending.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks for the run to end with `code`.
    pub fn signal(&self, code: u64) {
        self.0.code.store(code, Ordering::Relaxed);
        self.0.pending.store(true, Ordering::Release);
    }

    /// The pending exit code, left pending.
    #[must_use]
    pub fn peek(&self) -> Option<u64> {
        self.0.pending.load(Ordering::Acquire).then(|| self.0.code.load(Ordering::Relaxed))
    }

    /// The pending exit code, which no longer is.
    pub fn take(&self) -> Option<u64> {
        self.0.pending.swap(false, Ordering::AcqRel).then(|| self.0.code.load(Ordering::Relaxed))
    }
}
