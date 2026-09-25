//! Invariants of the coherence protocol, checked over the whole system.
//!
//! For every line not in the middle of a transaction: at most one private
//! L2 holds it Modified or Exclusive, and then no other L2 holds it at all;
//! an L1 never holds a line its L2 does not, nor holds it dirtier than the
//! L2's rights allow; and a precise home agent's tracking matches what the
//! L2s hold. Duplicate tags anywhere are a violation too.

use std::collections::BTreeMap;

use super::protocol::CoreSet;
use crate::common::{CoreId, LineAddr};
use crate::sim::packet::MesiState;
use crate::sim::state::SimState;

/// One broken invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// Line concerned.
    pub line: LineAddr,
    /// What was wrong.
    pub what: String,
}

const fn is_owner(state: MesiState) -> bool {
    matches!(state, MesiState::Modified | MesiState::Exclusive | MesiState::Owned)
}

/// Checks every invariant and returns what is broken (empty when all hold).
#[must_use]
pub fn audit(state: &SimState) -> Vec<Violation> {
    let mut violations = Vec::new();
    let Some(fabric) = state.shared.coherence.as_ref() else { return violations };

    let mut in_flight: Vec<LineAddr> = fabric.lines_in_flight();
    for core in &state.cores {
        in_flight.extend(core.l2_cache.lines_in_flight());
        in_flight.extend(core.l1_d_cache.lines_in_flight());
        in_flight.extend(core.l1_i_cache.lines_in_flight());
    }
    let busy = |line: LineAddr| in_flight.contains(&line);

    // Who holds what, from the L2s (the requesting agents).
    let mut holders: BTreeMap<LineAddr, Vec<(CoreId, MesiState)>> = BTreeMap::new();
    for core in &state.cores {
        for cache in [&core.l1_i_cache, &core.l1_d_cache, &core.l2_cache] {
            for dup in cache.duplicate_lines() {
                violations.push(Violation { line: dup, what: format!("duplicate tag in cache {:?}", cache.id) });
            }
        }
        let l2: BTreeMap<LineAddr, MesiState> = core.l2_cache.held_lines().into_iter().collect();
        if core.l2_cache.is_enabled() {
            for (line, l2_state) in &l2 {
                holders.entry(*line).or_default().push((core.core_id, *l2_state));
            }
        } else {
            // A disabled L2 still requests for its L1s: they are the holders.
            let mut held: BTreeMap<LineAddr, MesiState> = BTreeMap::new();
            for (line, state) in core.l1_i_cache.held_lines().into_iter().chain(core.l1_d_cache.held_lines()) {
                let entry = held.entry(line).or_insert(state);
                if is_owner(state) {
                    *entry = state;
                }
            }
            for (line, state) in held {
                holders.entry(line).or_default().push((core.core_id, state));
            }
        }
        for (line, l1_state) in core.l1_d_cache.held_lines() {
            if busy(line) || !core.l2_cache.is_enabled() {
                continue;
            }
            match l2.get(&line) {
                None => violations.push(Violation { line, what: format!("core {} L1D holds a line its L2 does not", core.core_id.val()) }),
                Some(l2_state) if is_owner(l1_state) && !is_owner(*l2_state) => violations.push(Violation {
                    line,
                    what: format!("core {} L1D holds {l1_state:?} while its L2 holds {l2_state:?}", core.core_id.val()),
                }),
                Some(_) => {}
            }
        }
    }

    for (line, held) in &holders {
        if busy(*line) {
            continue;
        }
        let owners: Vec<CoreId> = held.iter().filter(|(_, s)| is_owner(*s)).map(|(c, _)| *c).collect();
        if owners.len() > 1 {
            violations.push(Violation { line: *line, what: format!("owners {owners:?}") });
        }
        if owners.len() == 1 && held.len() > 1 {
            violations.push(Violation { line: *line, what: format!("core {} owns the line while {} other cores hold it", owners[0].val(), held.len() - 1) });
        }
        if let Some(tracked) = fabric.tracked_holders(*line) {
            let mut actual = CoreSet::EMPTY;
            for (core, _) in held {
                actual.insert(*core);
            }
            if tracked.sharers != actual {
                violations.push(Violation { line: *line, what: format!("home tracks sharers {:?}, L2s hold {:?}", tracked.sharers.iter().collect::<Vec<_>>(), actual.iter().collect::<Vec<_>>()) });
            }
            if tracked.owner != owners.first().copied() {
                violations.push(Violation { line: *line, what: format!("home tracks owner {:?}, L2s say {:?}", tracked.owner, owners.first()) });
            }
        }
    }
    for line in fabric.tracked_lines().unwrap_or_default() {
        if !busy(line) && !holders.contains_key(&line) {
            violations.push(Violation { line, what: "home tracks a line no L2 holds".to_string() });
        }
    }
    violations
}
