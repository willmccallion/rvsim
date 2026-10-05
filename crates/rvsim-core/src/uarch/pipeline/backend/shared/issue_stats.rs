//! The issue-stall counts, shared by both backends.

use crate::uarch::ctx::CoreCtx;

/// Why the oldest instruction left in the issue queue did not issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueHold {
    /// Its operands were not ready.
    Operands,
    /// Every unit of its class was busy.
    Unit,
    /// Program order held it: it executes only as the oldest, waits on a
    /// fence or an older store's address, or a pending squash removes it.
    Ordering,
}

/// Counts the cycle's issue stalls: `stalls.fu_structural` when a ready
/// instruction found every unit of its class busy, and, when nothing
/// issued, `stalls.data` or `stalls.ordering` by what held the oldest.
pub fn count_issue_stalls(
    state: &mut CoreCtx<'_>,
    unit_stalled: bool,
    issued_any: bool,
    oldest: Option<IssueHold>,
) {
    let paths = &state.core.stat_paths.pipeline;
    let idle = match oldest.filter(|_| !issued_any) {
        Some(IssueHold::Operands) => Some(paths.stalls_data),
        Some(IssueHold::Ordering) => Some(paths.stalls_ordering),
        Some(IssueHold::Unit) | None => None,
    };
    let structural = unit_stalled.then_some(paths.stalls_fu_structural);
    for stat in [structural, idle].into_iter().flatten() {
        state.uncore.stats.counter(stat).inc();
    }
}
