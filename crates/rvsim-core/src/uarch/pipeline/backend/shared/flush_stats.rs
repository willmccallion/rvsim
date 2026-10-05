//! The `pipeline.flushes.*` counts, shared by both backends.

use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::backend::shared::commit::{CommitEvent, ReExecuteCause};
use crate::uarch::pipeline::squash::SquashCause;

/// Why the pipeline dropped in-flight instructions: a squash an executed
/// instruction asked for, or one commit took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushCause {
    /// A branch or jump resolved against its prediction.
    Branch,
    /// A system instruction, CSR access or vector op refetched what
    /// followed it, at execute or at commit, or a store re-executed for a
    /// PTE that changed under it.
    System,
    /// A load read stale data past an older store.
    MemoryOrder,
    /// A load or an LR/AMO read a value another hart has since overwritten.
    Coherence,
    /// An exception or interrupt taken at commit.
    Trap,
}

impl From<SquashCause> for FlushCause {
    fn from(cause: SquashCause) -> Self {
        match cause {
            SquashCause::Branch => Self::Branch,
            SquashCause::System => Self::System,
            SquashCause::MemoryOrder => Self::MemoryOrder,
            SquashCause::Coherence => Self::Coherence,
        }
    }
}

impl From<&CommitEvent> for FlushCause {
    fn from(event: &CommitEvent) -> Self {
        match event {
            CommitEvent::Trap(..) => Self::Trap,
            CommitEvent::ReExecute(_, ReExecuteCause::StaleLine) => Self::Coherence,
            CommitEvent::ReExecute(_, ReExecuteCause::ChangedPte) | CommitEvent::SquashAfter(_) => {
                Self::System
            }
        }
    }
}

/// Counts a flush for `cause` that dropped `squashed` ROB entries.
pub fn count_flush(state: &mut CoreCtx<'_>, cause: FlushCause, squashed: usize) {
    let paths = &state.core.stat_paths.pipeline;
    let by_cause = match cause {
        FlushCause::Branch => paths.flushes_branch,
        FlushCause::System => paths.flushes_system,
        FlushCause::MemoryOrder => paths.flushes_mem_violations,
        FlushCause::Coherence => paths.flushes_coherence,
        FlushCause::Trap => paths.flushes_trap,
    };
    let (total, squashed_insns) = (paths.flushes_total, paths.flushes_squashed_insns);
    let stats = &mut state.uncore.stats;
    stats.counter(total).inc();
    stats.counter(by_cause).inc();
    stats.counter(squashed_insns).add(squashed as u64);
}
