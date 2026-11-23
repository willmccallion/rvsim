//! Squashes: flushing after a trap or re-execution at commit, taking a
//! pending squash, and squashing a load that read memory too early.

use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::backend::shared::commit::{
    self, CommitEvent, CommitRegisters, CommitResources,
};
use crate::uarch::pipeline::backend::shared::flush_stats::count_flush;
use crate::uarch::pipeline::engine::ExecutionEngine;
use crate::uarch::pipeline::lsq::load_queue::OrderViolation;
use crate::uarch::pipeline::rob::RobTag;
use crate::uarch::pipeline::squash::SquashScope;
use crate::uarch::pipeline::squash::{FetchRedirect, PendingSquash, Redirect, SquashCause};
use crate::uarch::pipeline::trace::Departure;

use super::O3Engine;

/// A mispredicted branch whose correct path has yet to issue.
#[derive(Clone, Copy, Debug)]
pub(super) struct MispredictRecovery {
    branch: RobTag,
    resolved_at: u64,
}

impl O3Engine {
    /// Counts a cycle rename is blocked while a squash walks the ROB.
    pub(super) fn count_squash_stall(&mut self, state: &mut CoreCtx<'_>) {
        if self.squash_stall_remaining > 0 {
            self.squash_stall_remaining -= 1;
            state.uncore.kernel.stats.counter(state.core.stat_paths.pipeline.stalls_squash).inc();
        }
    }

    /// Cycles rename waits while a squash no checkpoint covers walks the ROB
    /// back from its tail, undoing `squash_width` squashed entries' mappings
    /// per cycle, as a core without rename checkpoints recovers.
    const fn rob_walk_cycles(&self, squashed: usize) -> u64 {
        squashed.div_ceil(self.squash_width) as u64
    }

    /// Records the cycles from a mispredicted branch resolving to the first
    /// instruction after it issuing, once that instruction is `issued`.
    pub(super) fn note_correct_path_issue(&mut self, state: &mut CoreCtx<'_>, issued: RobTag) {
        let Some(recovery) = self.mispredict_recovery else { return };
        if !issued.is_newer_than(recovery.branch) {
            return;
        }
        let penalty = state.uncore.kernel.cycle - recovery.resolved_at;
        state
            .uncore
            .kernel
            .stats
            .histogram(state.core.stat_paths.bp.mispredict_penalty)
            .record(penalty);
        self.mispredict_recovery = None;
    }

    /// Retires what the ROB head allows. A trap or a re-execution at commit
    /// flushes the whole window and redirects fetch; returns whether it did.
    pub(super) fn retire(
        &mut self,
        state: &mut CoreCtx<'_>,
        redirect: &mut Option<FetchRedirect>,
    ) -> bool {
        let commit_event = commit::commit_stage(
            state,
            CommitResources {
                common: &mut self.common,
                rob: &mut self.rob,
                store_buffer: &mut self.store_buffer,
                vec_store_buffer: &mut self.vec_store_buffer,
                width: self.commit_width,
                registers: CommitRegisters::Renamed {
                    prf: &mut self.prf,
                    rename_map: &mut self.committed_rename_map,
                    free_list: &mut self.free_list,
                    load_queue: &mut self.load_queue,
                    checkpoints: &mut self.checkpoints,
                    vec_prf: &mut self.vec_prf,
                    vec_free_list: &mut self.vec_free_list,
                },
            },
        );

        if let Some(event) = &commit_event {
            count_flush(state, event.into(), self.rob.len());
        }
        match commit_event {
            Some(CommitEvent::Trap(trap, pc)) => {
                self.flush(state);
                state.trap(&trap, pc);
                *redirect = Some(FetchRedirect::Commit(state.hart.pc));
                true
            }
            Some(CommitEvent::ReExecute(pc, _) | CommitEvent::SquashAfter(pc)) => {
                self.flush(state);
                state.hart.pc = pc;
                *redirect = Some(FetchRedirect::Commit(pc));
                true
            }
            None => false,
        }
    }

    /// Squashes from the load of a memory-order violation, or from the
    /// oldest load a coherence invalidation caught, whichever is older;
    /// the load re-executes, so it does not survive either.
    pub(super) fn squash_on_violation(
        &mut self,
        state: &mut CoreCtx<'_>,
        now: u64,
        order_violation: Option<OrderViolation>,
    ) {
        let squash = match (order_violation, self.common.commit.coherence_violation.take()) {
            (Some(order), Some(coherence_tag)) if coherence_tag.is_older_than(order.load) => {
                Some((coherence_tag, None))
            }
            (Some(order), _) => Some((order.load, Some(order.store_pc))),
            (None, Some(coherence_tag)) => Some((coherence_tag, None)),
            (None, None) => None,
        };
        let Some((violating_tag, store_pc)) = squash else { return };

        let violation_pc = self.rob.find_entry(violating_tag).map_or(state.hart.pc, |e| e.pc);
        let cause = if let Some(store_pc) = store_pc {
            self.mdp.violation(violation_pc, store_pc);
            SquashCause::MemoryOrder
        } else {
            state.uncore.kernel.stats.counter(state.core.stat_paths.lsq.coherence_violations).inc();
            SquashCause::Coherence
        };
        self.common.request_squash(PendingSquash {
            keep_tag: self.rob.prev_tag_of(violating_tag),
            redirect: Redirect::to(violation_pc, cause),
            resolved_at: now,
            apply_at: now + self.redirect_latency,
        });
    }

    /// Rebuild the speculative rename map after a partial flush by replaying surviving ROB entries.
    fn rebuild_rename_map(&mut self) {
        self.rename_map = self.committed_rename_map.clone();
        for entry in self.rob.iter() {
            if let Some(dest) = entry.ctrl.dest(entry.rd) {
                self.rename_map.set(dest, entry.dst.phys);
            }
            for (offset, &reg) in entry.vec_dst.as_slice().iter().enumerate() {
                self.rename_map.set_vec(entry.ctrl.vd.offset(offset), reg);
            }
        }
    }

    /// Removes from every structure what `scope` squashes, returning the
    /// squashed instructions' registers to the free lists.
    pub(super) fn squash_in_flight(&mut self, state: &mut CoreCtx<'_>, scope: SquashScope) {
        for seq in self.rob.seqs_after(scope.keep_tag()) {
            state.trace_left(seq, Departure::Squashed);
        }
        for entry in self.rob.iter_squashed(scope) {
            self.free_list.reclaim(entry.dst.phys);
            for &vec_dst in entry.vec_dst.as_slice() {
                self.vec_free_list.reclaim(vec_dst);
            }
        }
        // Older un-issued IQ entries survive a partial squash, or the
        // pipeline deadlocks.
        self.issue_queue.squash(scope);
        self.rob.squash(scope);
        self.store_buffer.squash(scope);
        self.load_queue.squash(scope);
        self.mdp.squash(scope, &self.rob);
        self.vec_store_buffer.squash(scope);
        self.common.squash(scope);
        self.fu_pool.release_squashed(state.uncore.kernel.cycle, |tag| !scope.keeps(tag));
        self.mem1_mem2.retain(|e| scope.keeps(e.op.rob_tag));
        self.mem2_wb.retain(|e| scope.keeps(e.op.rob_tag));
        self.pending_results.retain(|p| scope.keeps(p.entry.op.rob_tag));
        self.pending_addresses.retain(|p| scope.keeps(p.entry.op.rob_tag));
        self.vec_pending.retain(|v| scope.keeps(v.rob_tag));
        self.vec_mem_pending.retain(|m| scope.keeps(m.entry.op.rob_tag));
        self.vec_mem_inflight.retain(|m| scope.keeps(m.rob_tag));
        self.execute_mem1.retain(|e| scope.keeps(e.op.rob_tag));
    }

    /// Takes a squash: drops everything younger than its kept tag (or the
    /// whole window when that tag has already retired), reclaims their
    /// physical registers, restores the rename map, and redirects fetch.
    pub(super) fn apply_squash(
        &mut self,
        state: &mut CoreCtx<'_>,
        squash: PendingSquash,
        redirect: &mut Option<FetchRedirect>,
    ) {
        self.serialization.squash(|tag| squash.squashes(tag));
        let keep_tag = squash.keep_tag.filter(|tag| self.rob.find_entry(*tag).is_some());
        let scope = SquashScope::keeping(keep_tag);
        let keep_seq = keep_tag.and_then(|tag| self.rob.find_entry(tag)).map(|entry| entry.seq);
        let squashed = self.rob.iter_squashed(scope).count();
        count_flush(state, squash.redirect.cause.into(), squashed);
        self.squash_in_flight(state, scope);

        // The committed map is the state after a kept instruction that has
        // already retired; a kept branch's checkpoint is the state after it.
        // Either is copied back at once.
        let restored_map = match keep_tag {
            None => Some(self.committed_rename_map.clone()),
            Some(tag) => self.checkpoints.find_by_tag(tag).map(|ckpt| ckpt.rename_map.clone()),
        };
        if let Some(rename_map) = restored_map {
            self.rename_map = rename_map;
            self.squash_stall_remaining = 0;
        } else {
            self.rebuild_rename_map();
            self.squash_stall_remaining = self.rob_walk_cycles(squashed);
        }
        self.checkpoints.squash(scope);
        self.mispredict_recovery = match (squash.redirect.cause, squash.keep_tag) {
            (SquashCause::Branch, Some(branch)) => {
                Some(MispredictRecovery { branch, resolved_at: squash.resolved_at })
            }
            _ => None,
        };

        *redirect = Some(FetchRedirect::Execute(squash.redirect.target));
        let now = state.uncore.kernel.cycle;
        self.common.squash_predictions(&mut state.core.branch_predictor, &squash, keep_seq, now);
        self.debug_assert_registers_conserved();
    }
}
