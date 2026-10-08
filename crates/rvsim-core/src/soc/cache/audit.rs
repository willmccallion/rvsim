//! A cache's own bookkeeping, for the system audit: what must hold of its
//! tags, MSHRs and writeback buffer between any two events.

use crate::common::LineAddr;
use crate::config::InclusionPolicy;
use crate::sim::components::ComponentId;

use super::Cache;

impl Cache {
    /// Every way this cache's bookkeeping is inconsistent: a line in two
    /// ways of a set, two MSHRs fetching one line, more MSHRs or
    /// writebacks than there is room for, an MSHR holding more requests
    /// than its target limit, a line written back twice at once.
    #[must_use]
    pub fn bookkeeping_faults(&self) -> Vec<(LineAddr, String)> {
        let mut faults: Vec<(LineAddr, String)> = self
            .duplicate_lines()
            .into_iter()
            .map(|line| (line, "held in two ways of its set".to_owned()))
            .collect();
        let mut fetching: Vec<LineAddr> = Vec::new();
        for mshr in self.mshrs.iter() {
            if fetching.contains(&mshr.line) {
                faults.push((mshr.line, "fetched by two MSHRs".to_owned()));
            }
            fetching.push(mshr.line);
            if mshr.target_count() > self.targets_per_mshr {
                let targets = mshr.target_count();
                let limit = self.targets_per_mshr;
                faults.push((mshr.line, format!("an MSHR holds {targets} targets of {limit}")));
            }
        }
        if fetching.len() > self.mshrs.capacity() {
            let (held, room) = (fetching.len(), self.mshrs.capacity());
            let line = fetching[0];
            faults.push((line, format!("{held} MSHRs allocated of {room}")));
        }
        let mut written: Vec<LineAddr> = Vec::new();
        for line in self.writebacks.lines() {
            if written.contains(&line) {
                faults.push((line, "written back twice at once".to_owned()));
            }
            written.push(line);
        }
        let taken = self.writebacks.slots_taken();
        if let Some(&line) = written.first()
            && taken > self.writebacks.capacity()
        {
            let room = self.writebacks.capacity();
            faults.push((line, format!("{taken} writeback slots taken of {room}")));
        }
        faults
    }

    /// The caches above that this one records as holding `line`; empty
    /// when it does not hold the line.
    #[must_use]
    pub fn recorded_holders(&self, line: LineAddr) -> Vec<ComponentId> {
        if !self.enabled || self.find_way(line.val()).is_none() {
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
