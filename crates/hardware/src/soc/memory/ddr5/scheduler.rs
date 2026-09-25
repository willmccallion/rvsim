//! Request selection policies for a DDR5 subchannel.
//!
//! The controller summarises every queued request into a [`Candidate`]
//! (does its row sit open in the bank, and when could its next command
//! issue) and asks the [`MemScheduler`] which one to advance this clock.

use std::fmt::Debug;

/// What the scheduler knows about one queued request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The request's row is open in its bank, so only a column command is
    /// needed.
    pub row_hit: bool,
    /// Earliest DRAM clock at which the request's column command could
    /// issue, given the bank's state and the rank / subchannel constraints.
    pub ready_at: u64,
}

/// Chooses which queued request a subchannel works on next.
pub trait MemScheduler: Debug + Send + Sync {
    /// Index into `candidates` (queue order is arrival order) of the request
    /// to advance, or `None` when the queue is empty.
    fn pick(&self, candidates: &[Candidate], now: u64) -> Option<usize>;
}

/// First-come first-served: always the oldest request, whatever its state.
#[derive(Debug, Default)]
pub struct Fcfs;

impl MemScheduler for Fcfs {
    fn pick(&self, candidates: &[Candidate], _now: u64) -> Option<usize> {
        if candidates.is_empty() { None } else { Some(0) }
    }
}

/// First-ready first-come first-served, gem5's default.
///
/// A row hit whose column command can issue now wins; otherwise the
/// request that becomes ready soonest, preferring row hits on a tie; ties
/// beyond that go to the oldest request.
#[derive(Debug, Default)]
pub struct FrFcfs;

impl MemScheduler for FrFcfs {
    fn pick(&self, candidates: &[Candidate], now: u64) -> Option<usize> {
        if let Some(seamless) = candidates.iter().position(|c| c.row_hit && c.ready_at <= now) {
            return Some(seamless);
        }
        let mut best: Option<(usize, Candidate)> = None;
        for (index, candidate) in candidates.iter().enumerate() {
            let better = match best {
                None => true,
                Some((_, current)) => {
                    candidate.ready_at < current.ready_at
                        || (candidate.ready_at == current.ready_at
                            && candidate.row_hit
                            && !current.row_hit)
                }
            };
            if better {
                best = Some((index, *candidate));
            }
        }
        best.map(|(index, _)| index)
    }
}

/// Which [`MemScheduler`] a controller is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Deserialize)]
pub enum SchedulerKind {
    /// [`Fcfs`].
    Fcfs,
    /// [`FrFcfs`].
    #[default]
    FrFcfs,
}

impl SchedulerKind {
    /// Instantiates the policy.
    #[must_use]
    pub fn build(self) -> Box<dyn MemScheduler> {
        match self {
            Self::Fcfs => Box::new(Fcfs),
            Self::FrFcfs => Box::new(FrFcfs),
        }
    }
}
