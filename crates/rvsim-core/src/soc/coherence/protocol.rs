//! Coherence protocols as pure state machines.
//!
//! A protocol decides who must be snooped for a request, what a snooped
//! holder ends up in, and what the requester is granted. It sees no
//! timing; the home agent and the interconnect supply that.

use crate::common::CoreId;
use crate::sim::packet::Maintenance;
use crate::sim::packet::MesiState;
use crate::sim::packet::coherence::{ReqKind, SnoopKind};

/// A set of cores, as a bitmap (at most 64 cores).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreSet(u64);

impl CoreSet {
    /// The empty set.
    pub const EMPTY: Self = Self(0);

    /// The set holding only `core`.
    #[must_use]
    pub const fn single(core: CoreId) -> Self {
        Self(1 << core.val())
    }

    /// Adds `core`.
    pub const fn insert(&mut self, core: CoreId) {
        self.0 |= 1 << core.val();
    }

    /// Removes `core`.
    pub const fn remove(&mut self, core: CoreId) {
        self.0 &= !(1 << core.val());
    }

    /// True when `core` is a member.
    #[must_use]
    pub const fn contains(self, core: CoreId) -> bool {
        (self.0 >> core.val()) & 1 == 1
    }

    /// True when no core is a member.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[cfg(test)]
    /// Number of members.
    #[must_use]
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// The set without `core`.
    #[must_use]
    pub const fn without(self, core: CoreId) -> Self {
        Self(self.0 & !(1 << core.val()))
    }

    /// Members in ascending order.
    pub fn iter(self) -> impl Iterator<Item = CoreId> {
        (0..64u32).filter(move |bit| (self.0 >> bit) & 1 == 1).map(CoreId::new)
    }
}

/// Who holds a line, as far as the home agent knows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Holders {
    /// Every core with a copy (in any state).
    pub sharers: CoreSet,
    /// The core holding the line Modified or Exclusive, if any; always a
    /// member of `sharers`.
    pub owner: Option<CoreId>,
}

/// A pure coherence state machine.
pub trait CoherenceProtocol: Send + Sync + std::fmt::Debug {
    /// Snoops the home must issue so that `requester` can be granted
    /// `kind`, given who holds the line now. Cores are snooped at most
    /// once and never the requester.
    fn snoops_for(
        &self,
        kind: ReqKind,
        requester: CoreId,
        holders: Holders,
    ) -> Vec<(CoreId, SnoopKind)>;

    /// State the requester installs once the snoops are done.
    /// `others_remain` says some other core keeps a (uncore) copy.
    fn grant(&self, kind: ReqKind, others_remain: bool) -> MesiState;

    /// Name for stats and traces.
    fn name(&self) -> &'static str;
}

/// Classic MESI: one owner (M or E) or any number of sharers (S).
#[derive(Clone, Copy, Debug, Default)]
pub struct Mesi;

impl CoherenceProtocol for Mesi {
    fn snoops_for(
        &self,
        kind: ReqKind,
        requester: CoreId,
        holders: Holders,
    ) -> Vec<(CoreId, SnoopKind)> {
        let others = holders.sharers.without(requester);
        match kind {
            ReqKind::ReadShared => match holders.owner {
                // Only an owner can hold data the memory side lacks, or an
                // exclusive right that must become uncore.
                Some(owner) if owner != requester => vec![(owner, SnoopKind::Shared)],
                _ => Vec::new(),
            },
            ReqKind::ReadUnique | ReqKind::CleanUnique => {
                others.iter().map(|core| (core, SnoopKind::Unique)).collect()
            }
            ReqKind::Maintain { op: Maintenance::Clean, .. } => match holders.owner {
                // Only an owner can hold modified data.
                Some(owner) if owner != requester => vec![(owner, SnoopKind::Clean)],
                _ => Vec::new(),
            },
            ReqKind::Maintain { op: Maintenance::Flush, .. } => {
                others.iter().map(|core| (core, SnoopKind::Unique)).collect()
            }
            ReqKind::Maintain { op: Maintenance::Invalidate, .. } => {
                others.iter().map(|core| (core, SnoopKind::MakeInvalid)).collect()
            }
            ReqKind::WriteBack { .. } | ReqKind::Evict => Vec::new(),
        }
    }

    fn grant(&self, kind: ReqKind, others_remain: bool) -> MesiState {
        match kind {
            ReqKind::ReadShared if others_remain => MesiState::Shared,
            ReqKind::ReadShared => MesiState::Exclusive,
            ReqKind::ReadUnique | ReqKind::CleanUnique => MesiState::Modified,
            ReqKind::WriteBack { .. } | ReqKind::Evict | ReqKind::Maintain { .. } => {
                MesiState::Invalid
            }
        }
    }

    fn name(&self) -> &'static str {
        "MESI"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C0: CoreId = CoreId::new(0);
    const C1: CoreId = CoreId::new(1);
    const C2: CoreId = CoreId::new(2);

    fn holders(sharers: &[CoreId], owner: Option<CoreId>) -> Holders {
        let mut set = CoreSet::EMPTY;
        for c in sharers {
            set.insert(*c);
        }
        Holders { sharers: set, owner }
    }

    #[test]
    fn read_shared_snoops_only_an_owner_and_grants_exclusive_when_alone() {
        let mesi = Mesi;
        assert!(mesi.snoops_for(ReqKind::ReadShared, C0, Holders::default()).is_empty());
        assert!(
            mesi.snoops_for(ReqKind::ReadShared, C0, holders(&[C1, C2], None)).is_empty(),
            "sharers keep S"
        );
        assert_eq!(
            mesi.snoops_for(ReqKind::ReadShared, C0, holders(&[C1], Some(C1))),
            vec![(C1, SnoopKind::Shared)]
        );
        assert!(
            mesi.snoops_for(ReqKind::ReadShared, C1, holders(&[C1], Some(C1))).is_empty(),
            "never snoop the requester"
        );
        assert_eq!(mesi.grant(ReqKind::ReadShared, false), MesiState::Exclusive);
        assert_eq!(mesi.grant(ReqKind::ReadShared, true), MesiState::Shared);
    }

    #[test]
    fn unique_requests_invalidate_every_other_holder() {
        let mesi = Mesi;
        let snoops = mesi.snoops_for(ReqKind::ReadUnique, C0, holders(&[C0, C1, C2], None));
        assert_eq!(snoops, vec![(C1, SnoopKind::Unique), (C2, SnoopKind::Unique)]);
        let snoops = mesi.snoops_for(ReqKind::CleanUnique, C1, holders(&[C0, C1], None));
        assert_eq!(snoops, vec![(C0, SnoopKind::Unique)]);
        assert_eq!(mesi.grant(ReqKind::ReadUnique, false), MesiState::Modified);
        assert_eq!(mesi.grant(ReqKind::CleanUnique, false), MesiState::Modified);
    }

    #[test]
    fn core_sets_are_bitmaps() {
        let mut set = CoreSet::single(C1);
        set.insert(C2);
        assert_eq!(set.len(), 2);
        assert!(set.contains(C2) && !set.contains(C0));
        assert_eq!(set.without(C1).iter().collect::<Vec<_>>(), vec![C2]);
        set.remove(C1);
        set.remove(C2);
        assert!(set.is_empty());
    }
}
