//! A cache's own bookkeeping, for the system audit: what must hold of its
//! tags, MSHRs and writeback buffer between any two events.

use std::fmt;

use crate::common::LineAddr;
use crate::config::InclusionPolicy;
use crate::sim::components::ComponentId;

use super::Cache;

/// One way a cache's own bookkeeping is inconsistent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookkeepingFault {
    /// The line is held in two ways of its set.
    DuplicateTag,
    /// Two MSHRs are fetching the line.
    FetchedByTwoMshrs,
    /// An MSHR holds more requests than its target limit.
    MshrOverTargets {
        /// Requests the MSHR holds.
        targets: usize,
        /// Requests an MSHR may hold.
        limit: usize,
    },
    /// More MSHRs are allocated than the cache has.
    MshrsOverCapacity {
        /// MSHRs allocated.
        held: usize,
        /// MSHRs the cache has.
        room: usize,
    },
    /// The line is being written back twice at once.
    WrittenBackTwice,
    /// More writeback slots are taken than the buffer has.
    WritebacksOverCapacity {
        /// Slots taken.
        taken: usize,
        /// Slots the buffer has.
        room: usize,
    },
}

impl fmt::Display for BookkeepingFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTag => write!(f, "held in two ways of its set"),
            Self::FetchedByTwoMshrs => write!(f, "fetched by two MSHRs"),
            Self::MshrOverTargets { targets, limit } => {
                write!(f, "an MSHR holds {targets} targets of {limit}")
            }
            Self::MshrsOverCapacity { held, room } => write!(f, "{held} MSHRs allocated of {room}"),
            Self::WrittenBackTwice => write!(f, "written back twice at once"),
            Self::WritebacksOverCapacity { taken, room } => {
                write!(f, "{taken} writeback slots taken of {room}")
            }
        }
    }
}

impl Cache {
    /// Every way this cache's bookkeeping is inconsistent: a line in two
    /// ways of a set, two MSHRs fetching one line, more MSHRs or
    /// writebacks than there is room for, an MSHR holding more requests
    /// than its target limit, a line written back twice at once.
    #[must_use]
    pub fn bookkeeping_faults(&self) -> Vec<(LineAddr, BookkeepingFault)> {
        let mut faults: Vec<(LineAddr, BookkeepingFault)> = self
            .duplicate_lines()
            .into_iter()
            .map(|line| (line, BookkeepingFault::DuplicateTag))
            .collect();
        let mut fetching: Vec<LineAddr> = Vec::new();
        for mshr in self.mshrs.iter() {
            if fetching.contains(&mshr.line) {
                faults.push((mshr.line, BookkeepingFault::FetchedByTwoMshrs));
            }
            fetching.push(mshr.line);
            if mshr.target_count() > self.targets_per_mshr {
                let (targets, limit) = (mshr.target_count(), self.targets_per_mshr);
                faults.push((mshr.line, BookkeepingFault::MshrOverTargets { targets, limit }));
            }
        }
        if fetching.len() > self.mshrs.capacity() {
            let (held, room) = (fetching.len(), self.mshrs.capacity());
            let line = fetching[0];
            faults.push((line, BookkeepingFault::MshrsOverCapacity { held, room }));
        }
        let mut written: Vec<LineAddr> = Vec::new();
        for line in self.writebacks.lines() {
            if written.contains(&line) {
                faults.push((line, BookkeepingFault::WrittenBackTwice));
            }
            written.push(line);
        }
        let taken = self.writebacks.slots_taken();
        if let Some(&line) = written.first()
            && taken > self.writebacks.capacity()
        {
            let room = self.writebacks.capacity();
            faults.push((line, BookkeepingFault::WritebacksOverCapacity { taken, room }));
        }
        faults
    }

    /// The caches above that this one records as holding `line`; empty
    /// when it does not hold the line.
    #[must_use]
    pub fn recorded_holders(&self, line: LineAddr) -> Vec<ComponentId> {
        if !self.enabled || self.find_way(line).is_none() {
            return Vec::new();
        }
        self.upper_holders(line)
    }

    /// What this cache keeps of the lines the caches above it hold.
    #[must_use]
    pub const fn inclusion_of_upper_levels(&self) -> InclusionPolicy {
        self.upstream_inclusion
    }

    /// This cache's id as a component.
    #[must_use]
    pub const fn component(&self) -> ComponentId {
        ComponentId::Cache(self.id)
    }
}
