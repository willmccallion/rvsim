//! Invariants of the coherence protocol, checked over the whole system.
//!
//! For every line not in the middle of a transaction: at most one private
//! L2 holds it Modified or Exclusive, and then no other L2 holds it at all;
//! an L1 never holds a line its L2 does not, nor holds it dirtier than the
//! L2's rights allow; and a precise home agent's tracking matches what the
//! L2s hold. Duplicate tags anywhere are a violation too.

use std::collections::BTreeMap;
use std::fmt;

use crate::common::{CoreId, LineAddr};
use crate::sim::components::CacheId;
use crate::sim::packet::MesiState;
use crate::soc::cache::BookkeepingFault;
use crate::soc::cache::Cache;
use crate::soc::coherence::fabric::CoherenceFabric;
use crate::soc::coherence::protocol::CoreSet;
use crate::system::state::SystemState;
use crate::uarch::CoreUnits;

/// One broken invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// Line concerned.
    pub line: LineAddr,
    /// Which invariant broke.
    pub kind: ViolationKind,
}

/// Which invariant a [`Violation`] broke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViolationKind {
    /// A core's L1D holds a line its L2 does not.
    L1NotInL2 {
        /// The core whose caches disagree.
        core: CoreId,
    },
    /// A core's L1D holds the line with rights its L2 lacks.
    L1DirtierThanL2 {
        /// The core whose caches disagree.
        core: CoreId,
        /// The L1D's state for the line.
        l1: MesiState,
        /// The L2's state for the line.
        l2: MesiState,
    },
    /// More than one core holds the line Modified or Exclusive.
    MultipleOwners(Vec<CoreId>),
    /// One core owns the line while others hold copies.
    OwnerWithSharers {
        /// The core holding it Modified or Exclusive.
        owner: CoreId,
        /// How many other cores hold a copy.
        others: usize,
    },
    /// The home agent's sharer set differs from the L2s'.
    HomeSharersMismatch {
        /// The sharers the home agent records.
        tracked: CoreSet,
        /// The cores whose L2s hold the line.
        held: CoreSet,
    },
    /// The home agent's owner differs from the L2s'.
    HomeOwnerMismatch {
        /// The owner the home agent records.
        tracked: Option<CoreId>,
        /// The core whose L2 owns the line.
        held: Option<CoreId>,
    },
    /// The home agent tracks a line no L2 holds.
    HomeTracksUnheldLine,
    /// A cache's own bookkeeping is inconsistent.
    Bookkeeping {
        /// The inconsistent cache.
        cache: CacheId,
        /// What is inconsistent.
        fault: BookkeepingFault,
    },
    /// A cache and the level below it break the lower level's inclusion
    /// policy or its record of the copies above.
    Inclusion {
        /// The cache holding the line above.
        upper: CacheId,
        /// The level below it.
        lower: CacheId,
        /// How they disagree.
        fault: InclusionFault,
    },
}

/// How a cache and the level below it disagree about a line above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InclusionFault {
    /// The lower level is inclusive of the upper but lacks the line.
    MissingFromInclusiveLevel,
    /// The lower level is exclusive of the upper but holds the line too.
    InExclusiveLevel,
    /// The lower level holds the line but does not record the upper copy.
    UnrecordedCopy,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {:#x}: {}", self.line.val(), self.kind)
    }
}

impl fmt::Display for ViolationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::L1NotInL2 { core } => {
                write!(f, "core {} L1D holds a line its L2 does not", core.val())
            }
            Self::L1DirtierThanL2 { core, l1, l2 } => {
                write!(f, "core {} L1D holds {l1:?} while its L2 holds {l2:?}", core.val())
            }
            Self::MultipleOwners(owners) => write!(f, "owners {owners:?}"),
            Self::OwnerWithSharers { owner, others } => {
                write!(f, "core {} owns the line while {others} other cores hold it", owner.val())
            }
            Self::HomeSharersMismatch { tracked, held } => write!(
                f,
                "home tracks sharers {:?}, L2s hold {:?}",
                tracked.iter().collect::<Vec<_>>(),
                held.iter().collect::<Vec<_>>()
            ),
            Self::HomeOwnerMismatch { tracked, held } => {
                write!(f, "home tracks owner {tracked:?}, L2s say {held:?}")
            }
            Self::HomeTracksUnheldLine => write!(f, "home tracks a line no L2 holds"),
            Self::Bookkeeping { cache, fault } => write!(f, "cache {cache:?}: {fault}"),
            Self::Inclusion { upper, lower, fault } => {
                write!(f, "caches {upper:?} and {lower:?}: {fault}")
            }
        }
    }
}

impl fmt::Display for InclusionFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingFromInclusiveLevel => {
                "held above a level inclusive of it that does not hold it"
            }
            Self::InExclusiveLevel => "held both above and in a level exclusive of it",
            Self::UnrecordedCopy => "held above a level that does not record the copy",
        })
    }
}

const fn is_owner(state: MesiState) -> bool {
    matches!(state, MesiState::Modified | MesiState::Exclusive)
}

/// Lines the coherence fabric or a core's private cache has a fetch,
/// writeback or transaction open on.
pub fn lines_in_flight(state: &SystemState) -> Vec<LineAddr> {
    let fabric = state.uncore.coherence.iter().flat_map(CoherenceFabric::lines_in_flight);
    let private = state.cores.iter().flat_map(|core| {
        let units = &core.units;
        [&units.l1_i_cache, &units.l1_d_cache, &units.l2_cache]
    });
    fabric.chain(private.flat_map(Cache::lines_in_flight)).collect()
}

/// Checks every invariant and returns what is broken (empty when all hold).
#[must_use]
pub fn audit(state: &SystemState) -> Vec<Violation> {
    let Some(fabric) = state.uncore.coherence.as_ref() else { return Vec::new() };
    let in_flight = lines_in_flight(state);
    let busy = |line: &LineAddr| in_flight.contains(line);
    let held_by = holders_by_line(state);

    let mut violations: Vec<Violation> =
        state.cores.iter().flat_map(|core| l1_inclusion_faults(&core.units, &busy)).collect();
    for (line, held) in held_by.iter().filter(|(line, _)| !busy(line)) {
        let owners: Vec<CoreId> =
            held.iter().filter(|(_, state)| is_owner(*state)).map(|(core, _)| *core).collect();
        violations.extend(ownership_faults(*line, held, &owners));
        violations.extend(home_tracking_faults(*line, held, &owners, fabric));
    }
    let untracked = fabric.tracked_lines().unwrap_or_default().into_iter();
    violations.extend(
        untracked
            .filter(|line| !busy(line) && !held_by.contains_key(line))
            .map(|line| Violation { line, kind: ViolationKind::HomeTracksUnheldLine }),
    );
    violations
}

/// The cores holding each line, and in what state.
type HeldBy = BTreeMap<LineAddr, Vec<(CoreId, MesiState)>>;

/// Who holds what, from the requesting agents: each core's L2, or its L1s
/// when the L2 is disabled.
fn holders_by_line(state: &SystemState) -> HeldBy {
    let mut held_by = HeldBy::new();
    for core in state.cores.iter().map(|core| &core.units) {
        for (line, state) in core_holdings(core) {
            held_by.entry(line).or_default().push((core.core_id, state));
        }
    }
    held_by
}

/// The lines `core` holds as a requesting agent. A disabled L2 still
/// requests for its L1s, so they hold the lines, the owning copy counting.
fn core_holdings(core: &CoreUnits) -> BTreeMap<LineAddr, MesiState> {
    if core.l2_cache.is_enabled() {
        return core.l2_cache.held_lines().into_iter().collect();
    }
    let mut held = BTreeMap::new();
    for (line, state) in
        core.l1_i_cache.held_lines().into_iter().chain(core.l1_d_cache.held_lines())
    {
        let entry = held.entry(line).or_insert(state);
        if is_owner(state) {
            *entry = state;
        }
    }
    held
}

/// Lines `core`'s L1D holds that its L2 lacks, or holds with fewer rights.
fn l1_inclusion_faults<'a>(
    core: &'a CoreUnits,
    busy: &'a impl Fn(&LineAddr) -> bool,
) -> impl Iterator<Item = Violation> + 'a {
    let l2: BTreeMap<LineAddr, MesiState> = core.l2_cache.held_lines().into_iter().collect();
    // Without an L2 the L1s are the requesting agents; there is no inclusion.
    let checked = core.l2_cache.is_enabled();
    core.l1_d_cache
        .held_lines()
        .into_iter()
        .filter(move |(line, _)| checked && !busy(line))
        .filter_map(move |(line, l1_state)| {
            let kind = match l2.get(&line) {
                None => ViolationKind::L1NotInL2 { core: core.core_id },
                Some(&l2_state) if is_owner(l1_state) && !is_owner(l2_state) => {
                    ViolationKind::L1DirtierThanL2 {
                        core: core.core_id,
                        l1: l1_state,
                        l2: l2_state,
                    }
                }
                Some(_) => return None,
            };
            Some(Violation { line, kind })
        })
}

/// A line with more than one owner, or an owner and other holders.
fn ownership_faults(
    line: LineAddr,
    held: &[(CoreId, MesiState)],
    owners: &[CoreId],
) -> Vec<Violation> {
    let mut violations = Vec::new();
    if owners.len() > 1 {
        violations.push(Violation { line, kind: ViolationKind::MultipleOwners(owners.to_vec()) });
    }
    if owners.len() == 1 && held.len() > 1 {
        let kind = ViolationKind::OwnerWithSharers { owner: owners[0], others: held.len() - 1 };
        violations.push(Violation { line, kind });
    }
    violations
}

/// Where the home agent's record of a line's sharers or owner differs
/// from who holds it.
fn home_tracking_faults(
    line: LineAddr,
    held: &[(CoreId, MesiState)],
    owners: &[CoreId],
    fabric: &CoherenceFabric,
) -> Vec<Violation> {
    let Some(tracked) = fabric.tracked_holders(line) else { return Vec::new() };
    let mut violations = Vec::new();
    let mut actual = CoreSet::EMPTY;
    for (core, _) in held {
        actual.insert(*core);
    }
    if tracked.sharers != actual {
        let kind = ViolationKind::HomeSharersMismatch { tracked: tracked.sharers, held: actual };
        violations.push(Violation { line, kind });
    }
    if tracked.owner != owners.first().copied() {
        let kind = ViolationKind::HomeOwnerMismatch {
            tracked: tracked.owner,
            held: owners.first().copied(),
        };
        violations.push(Violation { line, kind });
    }
    violations
}
