//! Home-agent tracking: who holds a line, and therefore who must be
//! snooped.
//!
//! `Broadcast` tracks nothing and snoops every other core. `SnoopFilter`
//! keeps an exact sharer bitmap and owner per tracked line in a
//! set-associative array; when a set is full the least recently used
//! line is recalled (every holder invalidated) before a new line can be
//! tracked, as Arm's snoop filter and AMD's probe filter do.

use super::protocol::{CoreSet, Holders};
use super::stats::HomeStatPaths;
use crate::common::{CoreId, LineAddr};
use crate::sim::packet::MesiState;
use crate::sim::stats::Stats;

/// Whether a line can be tracked right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    /// The line is tracked already or its set has a free way.
    Available,
    /// This line must be recalled first.
    Recall(LineAddr),
    /// Every way of the set is tied up in a live transaction.
    AllBusy,
}

/// The policy that decides who may hold a line.
pub trait HomeAgent: Send + Sync + std::fmt::Debug {
    /// Name for stats and traces.
    fn name(&self) -> &'static str;

    /// Who holds `line`; `None` when the agent does not know (everyone
    /// must be snooped).
    fn holders(
        &mut self,
        line: LineAddr,
        stats: &mut Stats,
        paths: &HomeStatPaths,
    ) -> Option<Holders>;

    /// Whether `line` can be tracked, and what must be recalled first when
    /// its set is full. `in_flight` lines have a live transaction: they
    /// cannot be recalled, and the untracked ones among them already claim
    /// a way each.
    fn room_for(&self, line: LineAddr, in_flight: &[LineAddr]) -> Room;

    /// `core` now holds `line` in `state`.
    fn on_grant(&mut self, line: LineAddr, core: CoreId, state: MesiState, now: u64);

    /// `core` dropped `line`.
    fn on_release(&mut self, line: LineAddr, core: CoreId);

    /// `core` went from owner to sharer of `line`.
    fn on_downgrade(&mut self, line: LineAddr, core: CoreId);

    /// Exact holders of `line` for audits, when the agent tracks exactly.
    fn exact_holders(&self, line: LineAddr) -> Option<Holders>;

    /// Every tracked line for audits, when the agent tracks exactly.
    fn tracked_lines(&self) -> Option<Vec<LineAddr>>;

    /// Every core dropped every line, as when all caches are emptied.
    fn forget_all(&mut self);
}

/// Snoop everyone; track nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Broadcast;

impl HomeAgent for Broadcast {
    fn name(&self) -> &'static str {
        "broadcast"
    }

    fn holders(
        &mut self,
        _line: LineAddr,
        _stats: &mut Stats,
        _paths: &HomeStatPaths,
    ) -> Option<Holders> {
        None
    }

    fn room_for(&self, _line: LineAddr, _in_flight: &[LineAddr]) -> Room {
        Room::Available
    }

    fn on_grant(&mut self, _line: LineAddr, _core: CoreId, _state: MesiState, _now: u64) {}

    fn on_release(&mut self, _line: LineAddr, _core: CoreId) {}

    fn on_downgrade(&mut self, _line: LineAddr, _core: CoreId) {}

    fn exact_holders(&self, _line: LineAddr) -> Option<Holders> {
        None
    }

    fn tracked_lines(&self) -> Option<Vec<LineAddr>> {
        None
    }

    fn forget_all(&mut self) {}
}

#[derive(Clone, Copy, Debug)]
struct FilterEntry {
    line: LineAddr,
    holders: Holders,
    last_use: u64,
}

/// Exact sharer tracking in a set-associative array.
#[derive(Debug)]
pub struct SnoopFilter {
    sets: Vec<Vec<Option<FilterEntry>>>,
    line_bytes: u64,
}

impl SnoopFilter {
    /// A filter tracking `entries` lines, `ways` per set.
    #[must_use]
    pub fn new(entries: usize, ways: usize, line_bytes: u64) -> Self {
        let ways = ways.max(1);
        let num_sets = (entries / ways).max(1);
        Self { sets: (0..num_sets).map(|_| vec![None; ways]).collect(), line_bytes }
    }

    /// Lines the filter can track.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.sets.len() * self.sets[0].len()
    }

    /// Lines currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.iter().flatten().filter(|e| e.is_some()).count()
    }

    /// True when nothing is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    const fn set_index(&self, line: LineAddr) -> usize {
        ((line.val() / self.line_bytes) as usize) % self.sets.len()
    }

    fn find(&self, line: LineAddr) -> Option<(usize, usize)> {
        let set = self.set_index(line);
        self.sets[set].iter().position(|e| e.is_some_and(|e| e.line == line)).map(|way| (set, way))
    }

    fn entry_mut(&mut self, line: LineAddr) -> Option<&mut FilterEntry> {
        let (set, way) = self.find(line)?;
        self.sets[set][way].as_mut()
    }
}

impl HomeAgent for SnoopFilter {
    fn name(&self) -> &'static str {
        "snoop-filter"
    }

    fn holders(
        &mut self,
        line: LineAddr,
        stats: &mut Stats,
        paths: &HomeStatPaths,
    ) -> Option<Holders> {
        let Some((set, way)) = self.find(line) else {
            stats.counter(paths.filter_misses).inc();
            return Some(Holders::default());
        };
        stats.counter(paths.filter_hits).inc();
        self.sets[set][way].map(|e| e.holders)
    }

    fn room_for(&self, line: LineAddr, in_flight: &[LineAddr]) -> Room {
        if self.find(line).is_some() {
            return Room::Available;
        }
        let set_index = self.set_index(line);
        let set = &self.sets[set_index];
        let free_ways = set.iter().filter(|e| e.is_none()).count();
        let mut claimed: Vec<LineAddr> = in_flight
            .iter()
            .copied()
            .filter(|l| *l != line && self.set_index(*l) == set_index && self.find(*l).is_none())
            .collect();
        claimed.sort_unstable_by_key(|l| l.val());
        claimed.dedup();
        if free_ways > claimed.len() {
            return Room::Available;
        }
        set.iter()
            .flatten()
            .filter(|e| !in_flight.contains(&e.line))
            .min_by_key(|e| (e.last_use, e.line.val()))
            .map_or(Room::AllBusy, |e| Room::Recall(e.line))
    }

    fn on_grant(&mut self, line: LineAddr, core: CoreId, state: MesiState, now: u64) {
        let owner = matches!(state, MesiState::Modified | MesiState::Exclusive | MesiState::Owned);
        if let Some(entry) = self.entry_mut(line) {
            entry.holders.sharers.insert(core);
            if owner {
                entry.holders.owner = Some(core);
            } else if entry.holders.owner == Some(core) {
                entry.holders.owner = None;
            }
            entry.last_use = now;
            return;
        }
        if matches!(state, MesiState::Invalid) {
            return;
        }
        let set = self.set_index(line);
        let Some(way) = self.sets[set].iter().position(Option::is_none) else {
            debug_assert!(false, "snoop filter set full: caller must recall first");
            return;
        };
        self.sets[set][way] = Some(FilterEntry {
            line,
            holders: Holders { sharers: CoreSet::single(core), owner: owner.then_some(core) },
            last_use: now,
        });
    }

    fn on_release(&mut self, line: LineAddr, core: CoreId) {
        let Some((set, way)) = self.find(line) else { return };
        let Some(entry) = self.sets[set][way].as_mut() else { return };
        entry.holders.sharers.remove(core);
        if entry.holders.owner == Some(core) {
            entry.holders.owner = None;
        }
        if entry.holders.sharers.is_empty() {
            self.sets[set][way] = None;
        }
    }

    fn on_downgrade(&mut self, line: LineAddr, core: CoreId) {
        if let Some(entry) = self.entry_mut(line)
            && entry.holders.owner == Some(core)
        {
            entry.holders.owner = None;
        }
    }

    fn exact_holders(&self, line: LineAddr) -> Option<Holders> {
        Some(
            self.find(line)
                .and_then(|(set, way)| self.sets[set][way])
                .map_or_else(Holders::default, |e| e.holders),
        )
    }

    fn tracked_lines(&self) -> Option<Vec<LineAddr>> {
        Some(self.sets.iter().flatten().flatten().map(|e| e.line).collect())
    }

    fn forget_all(&mut self) {
        for entry in self.sets.iter_mut().flatten() {
            *entry = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::PhysAddr;

    const C0: CoreId = CoreId::new(0);
    const C1: CoreId = CoreId::new(1);

    fn line(addr: u64) -> LineAddr {
        LineAddr::from_phys(PhysAddr::new(addr), 64)
    }

    fn paths() -> HomeStatPaths {
        HomeStatPaths::new("test.ha")
    }

    #[test]
    fn the_filter_tracks_sharers_and_the_owner_exactly() {
        let mut filter = SnoopFilter::new(8, 2, 64);
        let mut stats = Stats::new();
        assert_eq!(filter.holders(line(0x1000), &mut stats, &paths()), Some(Holders::default()));

        filter.on_grant(line(0x1000), C0, MesiState::Exclusive, 1);
        let h = filter.exact_holders(line(0x1000)).unwrap();
        assert_eq!(h.owner, Some(C0));
        assert!(h.sharers.contains(C0));

        filter.on_downgrade(line(0x1000), C0);
        filter.on_grant(line(0x1000), C1, MesiState::Shared, 2);
        let h = filter.exact_holders(line(0x1000)).unwrap();
        assert_eq!(h.owner, None);
        assert_eq!(h.sharers.len(), 2);

        filter.on_release(line(0x1000), C0);
        filter.on_release(line(0x1000), C1);
        assert!(filter.is_empty(), "the last release frees the entry");
    }

    #[test]
    fn a_full_set_names_its_lru_line_for_recall_unless_busy() {
        let mut filter = SnoopFilter::new(2, 2, 64);
        // Two sets? entries/ways = 1 set of 2 ways.
        filter.on_grant(line(0x0000), C0, MesiState::Shared, 5);
        filter.on_grant(line(0x0040), C1, MesiState::Shared, 9);
        assert_eq!(filter.room_for(line(0x0080), &[]), Room::Recall(line(0x0000)));
        assert_eq!(filter.room_for(line(0x0080), &[line(0x0000)]), Room::Recall(line(0x0040)));
        assert_eq!(filter.room_for(line(0x0080), &[line(0x0000), line(0x0040)]), Room::AllBusy);
        assert_eq!(filter.room_for(line(0x0000), &[line(0x0000), line(0x0040)]), Room::Available);
    }

    #[test]
    fn an_untracked_line_in_flight_claims_a_way() {
        let mut filter = SnoopFilter::new(2, 2, 64);
        filter.on_grant(line(0x0000), C0, MesiState::Shared, 5);
        assert_eq!(filter.room_for(line(0x0080), &[]), Room::Available);
        assert_eq!(filter.room_for(line(0x0080), &[line(0x0040)]), Room::Recall(line(0x0000)));
        assert_eq!(filter.room_for(line(0x0080), &[line(0x0040), line(0x0000)]), Room::AllBusy);
    }

    #[test]
    fn broadcast_knows_nothing() {
        let mut b = Broadcast;
        let mut stats = Stats::new();
        assert_eq!(b.holders(line(0), &mut stats, &paths()), None);
        assert_eq!(b.exact_holders(line(0)), None);
        assert_eq!(b.room_for(line(0), &[]), Room::Available);
    }
}
