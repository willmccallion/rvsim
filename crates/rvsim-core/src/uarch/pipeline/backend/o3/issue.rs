//! Dispatch into the issue queue, and select and issue out of it.

use super::O3Entry;
use crate::exec::compute::vector::agnostic::AgnosticRecorder;
use crate::exec::compute::vector::execute::execute_vec_op_on;
use crate::exec::compute::vector::mem::{check_vec_mem_emul, element_accesses, is_vec_store};
use crate::isa::extension::Extension;
use crate::isa::rvv::{VRegIdx, parse_vtype};
use crate::soc::uncore::commit_log_open;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::mdp::MemDepAccess;
use crate::uarch::pipeline::backend::shared::execute::record_vector_flags;
use crate::uarch::pipeline::backend::shared::issue_stats::count_issue_stalls;
use crate::uarch::pipeline::backend::shared::vec_mem::VecMemInflight;
use crate::uarch::pipeline::latches::ExMem1Entry;
use crate::uarch::pipeline::rename::vec_prf::{VecPhysReg, VecPrfView, VecRegGroup};
use crate::uarch::pipeline::squash::PendingSquash;
use crate::uarch::pipeline::trace::Stage;
use crate::uarch::vector::chaining::VecPendingResult;
use crate::uarch::vector::lane_model;

use super::execute;
use super::fu_pool::FuType;
use super::issue_queue::{IssueBudget, SelectedEntry};
use super::{O3Engine, PendingResult};

/// A vector instruction's destination register group.
#[derive(Clone, Copy, Debug)]
struct VecDest {
    /// Its renamed physical registers.
    group: VecRegGroup,
    /// The first architectural register of the group.
    reg: VRegIdx,
}

/// A vector instruction that has issued, on its way into the vector unit.
struct IssuedVector<'a> {
    entry: &'a O3Entry,
    result: ExMem1Entry,
    fu_type: FuType,
    dest: Option<VecDest>,
    complete_cycle: u64,
}

/// Points `count` architectural registers from `base` at `regs`.
fn overlay(mapping: &mut [VecPhysReg; 32], base: VRegIdx, group: &VecRegGroup) {
    let base = base.as_u8() as usize;
    for (i, &reg) in group.as_slice().iter().enumerate() {
        if base + i < 32 {
            mapping[base + i] = reg;
        }
    }
}

impl O3Engine {
    /// Selects ready instructions within the issue width, ports and free
    /// units, and issues them.
    pub(super) fn issue(&mut self, state: &mut CoreCtx<'_>, now: u64, memory_blocked: bool) {
        let budget = IssueBudget {
            width: self.issue_width,
            load_ports: self.load_ports,
            store_ports: self.store_ports,
            units: &self.fu_pool,
            now,
            memory_blocked,
            head: self.common.commit.head_at_cycle_start,
        };
        let selection = self.issue_queue.select(&budget, &self.store_buffer, &self.rob);
        let issued_any = !selection.entries.is_empty() || !selection.store_data.is_empty();
        count_issue_stalls(state, selection.unit_stalls > 0, issued_any, selection.oldest);
        for selected in selection.entries {
            self.issue_one(state, selected, now);
        }
        for data in selection.store_data {
            self.store_buffer.resolve_data(data.rob_tag, data.value);
            state.uncore.kernel.stats.counter(state.core.stat_paths.lsq.split_stores).inc();
        }
    }

    /// Issues one selected instruction: reserves its unit, executes it, and
    /// tracks its result until the unit is done with it.
    fn issue_one(&mut self, state: &mut CoreCtx<'_>, selected: SelectedEntry, now: u64) {
        let SelectedEntry { entry, fu_type, unit, data_follows } = selected;
        state.trace_stage(entry.seq, Stage::Issue);
        if entry.inst.ctrl.mem_read || entry.inst.ctrl.uses_store_buffer() {
            self.mdp.issued(entry.rob_tag);
        }

        // vsetvl* run synchronously in execute_one; exclude from deferred VecPrfView.
        let is_vec_config = entry.inst.ctrl.is_vector_config();
        let is_vec_arith = fu_type.is_vector() && fu_type != FuType::VecMem && !is_vec_config;
        let is_vec_mem = fu_type == FuType::VecMem;
        let dest = vector_dest(&entry);

        let complete_cycle = if is_vec_arith {
            let latency = self.fu_pool.vector_op_latency(
                fu_type,
                &entry.inst.ctrl,
                entry.vector_config.vl as usize,
                self.num_vec_lanes.as_usize(),
            );
            self.fu_pool.acquire_with_latency(unit, now, latency, entry.rob_tag)
        } else {
            // A vector memory op's unit is the address generator;
            // its elements pay their latency in memory1 and memory2.
            self.fu_pool.acquire(unit, now, entry.rob_tag)
        };
        let busy = self.fu_pool.busy_cycles(unit, now);
        state.uncore.kernel.stats.counter(state.core.stat_paths.fu.all[fu_type as usize]).add(busy);

        let (mut result, redirect) =
            execute::execute_one(&mut state.stage(), &entry, &mut self.rob);
        result.store_data_follows = data_follows;
        if let Some(redirect) = redirect {
            self.common.request_squash(PendingSquash {
                keep_tag: Some(result.op.rob_tag),
                redirect,
                apply_at: complete_cycle + self.redirect_latency,
            });
        }
        if is_vec_config {
            self.common.vector_config_unresolved = false;
        }

        if (is_vec_arith || is_vec_mem) && result.op.trap.is_none() {
            let issued = IssuedVector { entry: &entry, result, fu_type, dest, complete_cycle };
            if is_vec_arith {
                self.execute_vector_arith(state, issued, now);
            } else {
                self.start_vector_memory(state, issued, now);
            }
            return;
        }

        let pending = PendingResult { entry: result, complete_cycle };
        if pending.entry.op.ctrl.uses_memory_pipeline() {
            self.pending_addresses.push(pending);
        } else {
            self.pending_results.push(pending);
        }
    }

    /// The speculative rename map's vector mappings, one per architectural
    /// register.
    fn vec_rename_view(&self) -> [VecPhysReg; 32] {
        std::array::from_fn(|i| self.rename_map.get_vec(VRegIdx::new(i as u8)))
    }

    /// Runs a vector arithmetic instruction on the registers it renamed,
    /// with the vtype and vl it was dispatched under, and tracks its result
    /// until its lanes finish.
    fn execute_vector_arith(&mut self, state: &CoreCtx<'_>, issued: IssuedVector<'_>, now: u64) {
        let IssuedVector { entry, result, fu_type, dest, complete_cycle } = issued;
        let ctrl = &entry.inst.ctrl;

        // Map through the rename-time registers so later renames don't alias.
        let mut mapping = self.vec_rename_view();
        overlay(&mut mapping, ctrl.vs2, &entry.regs.vs2);
        overlay(&mut mapping, ctrl.vs1, &entry.regs.vs1);
        // Before the destination: a masked compare writing v0 writes the
        // new register, reading each mask bit before it overwrites it.
        if let Some(mask) = entry.regs.mask {
            mapping[0] = mask;
        }
        if let Some(dest) = dest {
            let base = dest.reg.as_u8() as usize;
            for (i, &phys) in dest.group.as_slice().iter().enumerate() {
                if base + i < 32 {
                    // Pre-copy old vd so tail/mask-undisturbed reads see correct baseline.
                    if i < usize::from(entry.regs.vs3.len()) {
                        self.vec_prf.copy_reg(phys, entry.regs.vs3.get(i));
                    }
                    mapping[base + i] = phys;
                }
            }
        }

        let mut view = VecPrfView::new(&mut self.vec_prf, mapping);
        let mut recorder = AgnosticRecorder::new(&mut view, commit_log_open(state.uncore));
        let executed = execute_vec_op_on(
            &mut recorder,
            entry.vector_config,
            entry.vector_rounding,
            state.uncore.kernel.config.isa.vector.elen,
            state.core.isa.extensions.contains(Extension::Zvfh),
            &entry.inst,
        );
        #[cfg(feature = "commit-log")]
        let fills = recorder.into_fills();
        let vec_result = match executed {
            Ok(r) => r,
            Err(trap) => {
                self.rob.fault(result.op.rob_tag, trap);
                return;
            }
        };
        #[cfg(feature = "commit-log")]
        if let Some(fills) = fills {
            self.rob.set_vec_agnostic(result.op.rob_tag, fills);
        }
        record_vector_flags(&mut self.rob, result.op.rob_tag, &vec_result);

        let first_ready = lane_model::first_group_ready(now, self.fu_pool.startup_latency(fu_type));
        if result.op.ctrl.vec_reg_write {
            self.vec_pending.push(VecPendingResult {
                rob_tag: result.op.rob_tag,
                vd: dest.map_or(VecRegGroup::EMPTY, |d| d.group),
                first_group_ready: first_ready,
                full_complete: complete_cycle,
                wakeup_fired: false,
            });
        } else {
            // Scalar-result ops (vmv.x.s, vcpop.m, vfirst.m) take the scalar path.
            let mut scalar = result;
            scalar.op.alu = vec_result.scalar_result.unwrap_or(0);
            self.pending_results.push(PendingResult { entry: scalar, complete_cycle });
        }
    }

    /// Generates a vector memory instruction's element accesses and plans
    /// them into the micro-ops the memory stages will carry, released in
    /// waves as the load queue has room.
    fn start_vector_memory(&mut self, state: &CoreCtx<'_>, issued: IssuedVector<'_>, now: u64) {
        let IssuedVector { entry, result, fu_type, dest, complete_cycle } = issued;
        let ctrl = &entry.inst.ctrl;
        let vec_op = ctrl.vec_op;
        let is_store = is_vec_store(vec_op);
        let vd = dest.map_or(VecRegGroup::EMPTY, |d| d.group);

        // Reject illegal EMUL (>8) before element_accesses would panic.
        let vtype = parse_vtype(entry.vector_config.vtype);
        if let Err(trap) = check_vec_mem_emul(result.op.bits, ctrl, vtype) {
            self.rob.fault(result.op.rob_tag, trap);
            return;
        }

        let mut mapping = self.vec_rename_view();
        overlay(&mut mapping, ctrl.vs2, &entry.regs.vs2);
        overlay(&mut mapping, ctrl.vd, &entry.regs.vs3);
        if let Some(mask) = entry.regs.mask {
            mapping[0] = mask;
        }
        let micro_ops = element_accesses(
            &VecPrfView::new(&mut self.vec_prf, mapping),
            result.op.alu,
            result.store_data.cast_signed(),
            ctrl,
            entry.vector_config,
        );

        // Pre-copy old vd so tail / mask-undisturbed elements observe prior values.
        if !is_store && let Some(dest) = dest {
            let old = entry.regs.vs3.as_slice();
            for (&dst, &src) in dest.group.as_slice().iter().zip(old) {
                self.vec_prf.copy_reg(dst, src);
            }
        }

        if is_store {
            self.vec_store_buffer.set_expected_elements(result.op.rob_tag, micro_ops.len());
        }
        if micro_ops.is_empty() {
            // VL=0 / vill=1: route through vec_pending so destination physregs surface ready.
            let first_ready =
                lane_model::first_group_ready(now, self.fu_pool.startup_latency(fu_type));
            self.vec_pending.push(VecPendingResult {
                rob_tag: result.op.rob_tag,
                vd,
                first_group_ready: first_ready,
                full_complete: complete_cycle,
                wakeup_fired: false,
            });
            return;
        }
        let width = state.uncore.kernel.config.pipeline.vector_mem_width_bytes();
        self.vec_mem_inflight.push(VecMemInflight::plan(&result, micro_ops, width, vd));
    }

    /// Dispatches renamed instructions into the issue queue, each with the
    /// memory dependence the MDP predicts for it.
    pub(super) fn dispatch(&mut self, state: &mut CoreCtx<'_>, rename_output: &mut Vec<O3Entry>) {
        for entry in std::mem::take(rename_output) {
            state.trace_stage(entry.seq, Stage::Dispatch);
            let access = MemDepAccess::of(&entry.inst.ctrl);
            let mem_dep = self.mdp.dispatch(entry.inst.pc, entry.rob_tag, access);
            let ok = self.issue_queue.dispatch(entry, &self.prf, &self.vec_prf, mem_dep);
            debug_assert!(ok, "IQ dispatch failed — rename budget should prevent this");
        }
    }

    /// Adds what the memory dependence unit counted this cycle to the stats.
    pub(super) fn publish_mdp_stats(&mut self, state: &mut CoreCtx<'_>) {
        let counted = self.mdp.take_stats();
        let paths = &state.core.stat_paths.mdp;
        for (stat, value) in [
            (paths.predictions_bypass, counted.predictions_bypass),
            (paths.predictions_wait_all, counted.predictions_wait_all),
            (paths.predictions_wait_for, counted.predictions_wait_for),
            (paths.violations, counted.violations),
        ] {
            state.uncore.kernel.stats.counter(stat).add(value);
        }
    }
}

/// The destination register group of a vector instruction that writes
/// vector registers, as rename renamed it.
fn vector_dest(entry: &O3Entry) -> Option<VecDest> {
    let group = entry.regs.vd;
    (!group.is_empty()).then_some(VecDest { group, reg: entry.inst.ctrl.vd })
}
