//! Rename Stage: hands each decoded instruction to the engine, which
//! allocates its backend slots (ROB, store buffer, physical registers or
//! scoreboard tags) and returns the entry the issue stage works on.

use crate::core::pipeline::engine::{ExecutionEngine, Renamed};
use crate::core::pipeline::latches::{IdExEntry, RenameIssueEntry};
use crate::sim::StageCtx;

/// Executes the rename stage on the bundle in `input`. Up to
/// `engine.can_accept()` instructions are renamed in order; the first one
/// the engine cannot take, and everything behind it, waits in `input`.
pub fn rename_stage<E: ExecutionEngine>(
    state: &mut StageCtx<'_>,
    input: &mut Vec<IdExEntry>,
    engine: &mut E,
    rename_output: &mut Vec<RenameIssueEntry>,
) {
    let mut entries = std::mem::take(input).into_iter();
    let mut budget = engine.can_accept();
    let mut stalled = None;

    for id in entries.by_ref() {
        if budget == 0 {
            state.counter(state.core().stat_paths.pipeline.stalls_dispatch).inc();
            stalled = Some(id);
            break;
        }
        match engine.rename(state, id) {
            Renamed::Accepted(entry) => {
                rename_output.push(*entry);
                budget -= 1;
            }
            Renamed::Stalled(id) => {
                stalled = Some(*id);
                break;
            }
        }
    }
    input.extend(stalled);
    input.extend(entries);
}
