//! Invariants of every cache, checked over the whole system.
//!
//! Each cache's own bookkeeping, inclusion as each level's policy sets it,
//! every copy above a level recorded by it, and, with several cores, the
//! coherence invariants. Lines with a fetch, writeback or transaction in flight are
//! left out of the cross-level checks, which hold only between those.

use crate::common::LineAddr;
use crate::config::InclusionPolicy;
use crate::sim::packet::Packet;
use crate::soc::cache::Cache;
use crate::soc::coherence::fabric::CoherenceFabric;
use crate::system::coherence_audit::{self, Violation};
use crate::system::state::SystemState;

/// Every broken invariant (empty when all hold).
#[must_use]
pub(crate) fn audit(state: &SystemState) -> Vec<Violation> {
    let mut violations = coherence_audit::audit(state);
    let llc = &state.uncore.l3_cache;
    let mut in_flight =
        state.uncore.coherence.as_ref().map(CoherenceFabric::lines_in_flight).unwrap_or_default();
    in_flight.extend(llc.lines_in_flight());
    for core in state.cores.iter().map(|core| &core.units) {
        for cache in [&core.l1_i_cache, &core.l1_d_cache, &core.l2_cache] {
            in_flight.extend(cache.lines_in_flight());
        }
    }
    in_flight.extend(lines_on_their_way(state));
    let busy = |line: &LineAddr| in_flight.contains(line);

    violations.extend(bookkeeping(llc));
    for core in state.cores.iter().map(|core| &core.units) {
        for cache in [&core.l1_i_cache, &core.l1_d_cache, &core.l2_cache] {
            violations.extend(bookkeeping(cache));
        }
        let uppers = [&core.l1_i_cache, &core.l1_d_cache];
        violations.extend(level_faults(&uppers, &core.l2_cache, &busy));
        if state.uncore.coherence.is_none() {
            violations.extend(level_faults(&[&core.l2_cache], llc, &busy));
        }
    }
    violations
}

/// The lines named by a message not yet delivered: a request, a fill, a
/// probe, a back-invalidation or a coherence message. A level and the ones
/// above it disagree about such a line only until it arrives.
fn lines_on_their_way(state: &SystemState) -> Vec<LineAddr> {
    let line_bytes =
        state.cores.first().map_or(64, |core| core.units.l1_d_cache.line_bytes()) as u64;
    state
        .uncore
        .event_queue
        .pending()
        .filter_map(|event| match &event.packet {
            Packet::MemReq { paddr, .. } => Some(LineAddr::from_phys(*paddr, line_bytes)),
            Packet::MemResp { line_addr, .. }
            | Packet::Probe { line_addr, .. }
            | Packet::CacheInval { line_addr } => Some(*line_addr),
            Packet::Coh(msg) => Some(msg.line()),
            _ => None,
        })
        .collect()
}

/// `cache`'s own bookkeeping faults.
fn bookkeeping(cache: &Cache) -> impl Iterator<Item = Violation> + '_ {
    cache
        .bookkeeping_faults()
        .into_iter()
        .map(|(line, what)| Violation { line, what: format!("cache {:?}: {what}", cache.id) })
}

/// What `lower` and the `uppers` directly above it break of the lower
/// level's inclusion policy and of its record of the copies above.
fn level_faults(
    uppers: &[&Cache],
    lower: &Cache,
    busy: &impl Fn(&LineAddr) -> bool,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    if !lower.is_enabled() {
        return violations;
    }
    let policy = lower.inclusion_of_upper_levels();
    for upper in uppers.iter().filter(|upper| upper.is_enabled()) {
        for (line, _) in upper.held_lines() {
            if busy(&line) {
                continue;
            }
            let below = lower.contains(line.val());
            let what = match (policy, below) {
                (InclusionPolicy::Inclusive, false) => {
                    Some("held above a level inclusive of it that does not hold it")
                }
                (InclusionPolicy::Exclusive, true) => {
                    Some("held both above and in a level exclusive of it")
                }
                (_, true) if !lower.recorded_holders(line).contains(&upper.component()) => {
                    Some("held above a level that does not record the copy")
                }
                _ => None,
            };
            if let Some(what) = what {
                violations.push(Violation {
                    line,
                    what: format!("caches {:?} and {:?}: {what}", upper.id, lower.id),
                });
            }
        }
    }
    violations
}
