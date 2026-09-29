//! Retiring the ROB head: its architectural effects, the system effects
//! that end a retire group, and the memory and rename resources it
//! releases.

use crate::arch::reservation::LrScRecord;
use crate::exec::compute::vector::mem::{is_vec_load, is_vec_store};
use crate::exec::retire;
use crate::exec::signals::ControlFlow;
use crate::isa::csr;
use crate::isa::op::{MemWidth, SystemOp};
use crate::isa::reg::RegIdx;
use crate::isa::rvv::VRegIdx;
use crate::soc::uncore::debug::PC_TRACE_MAX;
use crate::trace_branch;
use crate::trace_commit;
use crate::trace_csr;
use crate::trace_trap;
use crate::uarch::ctx::CoreCtx;
use crate::uarch::pipeline::engine::BackendCommon;
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;
use crate::uarch::pipeline::outstanding::StoreOwner;
use crate::uarch::pipeline::rob::RobEntry;

use super::gate::updated_pte;
use super::stats::update_instruction_stats;
use super::writes::write_store_to_memory;
use super::{CommitEvent, CommitFlow, CommitRegisters};

/// The ROB and store buffers retirement updates.
pub(super) struct RetireTargets<'a, 'r> {
    pub common: &'a mut BackendCommon,
    pub store_buffer: &'a mut StoreBuffer,
    pub vec_store_buffer: &'a mut VecStoreBuffer,
    pub registers: &'a mut CommitRegisters<'r>,
}

/// Retires `entry`, which has left the ROB head.
pub(super) fn retire_entry(
    state: &mut CoreCtx<'_>,
    targets: &mut RetireTargets<'_, '_>,
    entry: &RobEntry,
) -> CommitFlow {
    advance_pc(state, entry);
    count_retired(state, entry);
    write_destinations(state, targets.registers, entry);
    apply_deferred_updates(state, targets.common, entry);

    if let CommitFlow::Stop(event) = retire_system(state, entry) {
        return CommitFlow::Stop(event);
    }
    release_memory_resources(state, targets, entry);
    if let Some(event) = retire_fence(state, entry) {
        return CommitFlow::Stop(Some(event));
    }

    state.hart.regs.write(RegIdx::new(0), 0);
    CommitFlow::Continue
}

/// Advances the architectural PC to the retired instruction's successor:
/// a taken branch's target, so an interrupt's EPC is right.
fn advance_pc(state: &mut CoreCtx<'_>, entry: &RobEntry) {
    let fall_through = entry.pc.wrapping_add(entry.inst_size.as_u64());
    state.hart.pc = match entry.ctrl.control_flow {
        ControlFlow::Jump => entry.bp_target.unwrap_or(fall_through),
        ControlFlow::Branch if entry.bp_outcome.taken => entry.bp_target.unwrap_or(fall_through),
        _ => fall_through,
    };
}

/// Records the retirement: the trace, the debug PC ring, the retired
/// counters and per-instruction statistics, and branch outcomes.
fn count_retired(state: &mut CoreCtx<'_>, entry: &RobEntry) {
    trace_commit!(state.config.general.trace_instructions;
        rob_tag    = entry.tag.0,
        pc         = %crate::common::trace::Hex(entry.pc),
        rd         = entry.rd.as_usize(),
        rd_phys    = entry.phys_dst.0,
        old_phys   = entry.old_phys_dst.0,
        result     = %crate::common::trace::Hex(entry.result.unwrap_or(0)),
        is_fp      = entry.ctrl.fp_reg_write,
        reg_write  = entry.ctrl.reg_write,
        is_store   = entry.ctrl.mem_write,
        is_load    = entry.ctrl.mem_read,
        fp_flags   = entry.fp_flags,
        "CM: instruction retired"
    );

    let hart_idx = state.hart.hart_id.as_index();
    let pc_trace = &mut state.per_hart_debug[hart_idx].pc_trace;
    pc_trace.push((entry.pc, entry.inst));
    if pc_trace.len() > PC_TRACE_MAX {
        let _ = pc_trace.remove(0);
    }

    state.hart.csrs.count_retired();
    state.hart.instructions_retired += 1;
    let hart_paths = state.hart_paths();
    state.stats.counter(hart_paths.retired_insts).inc();
    update_instruction_stats(state, entry);

    if entry.control_resolved {
        trace_branch!(state.config.general.trace_instructions;
            event         = "retire",
            pc            = %crate::common::trace::Hex(entry.pc),
            rob_tag       = entry.tag.0,
            actual_taken  = entry.bp_outcome.taken,
            actual_target = %crate::common::trace::Hex(entry.bp_target.unwrap_or(0)),
            mispredicted  = entry.bp_outcome.mispredicted,
            "CM: branch retired"
        );
        if entry.bp_outcome.mispredicted {
            state.uncore.stats.counter(state.core.stat_paths.bp.committed_mispredicts).inc();
        } else {
            state.uncore.stats.counter(state.core.stat_paths.bp.committed_hits).inc();
        }
    }
}

/// Writes the instruction's scalar and vector results to the architectural
/// registers and releases the physical registers they replace.
fn write_destinations(
    state: &mut CoreCtx<'_>,
    registers: &mut CommitRegisters<'_>,
    entry: &RobEntry,
) {
    let val = entry.result.unwrap_or(0);
    // Taken before the register write so the logged rd value is the result.
    #[cfg(feature = "commit-log")]
    let commit_log_entry = state.commit_log.is_some().then(|| {
        let has_rd = (entry.ctrl.reg_write && !entry.rd.is_zero()) || entry.ctrl.fp_reg_write;
        (entry.pc, entry.inst, has_rd, entry.rd.as_usize(), val)
    });

    debug_assert!(
        entry.result.is_some() || (!entry.ctrl.reg_write && !entry.ctrl.fp_reg_write),
        "CM: committing instruction with reg_write but no result: rob_tag={} pc={:#x}",
        entry.tag.0,
        entry.pc,
    );
    if entry.ctrl.fp_reg_write {
        retire::write_fp(state.hart, entry.rd, val);
        registers.retire_scalar(entry, true);
        trace_commit!(state.config.general.trace_instructions;
            pc       = %crate::common::trace::Hex(entry.pc),
            rob_tag  = entry.tag.0,
            reg      = entry.rd.as_usize(),
            rd_phys  = entry.phys_dst.0,
            old_phys = entry.old_phys_dst.0,
            value    = %crate::common::trace::Hex(val),
            is_fp    = true,
            "CM: FP register write"
        );
    } else if entry.ctrl.reg_write && !entry.rd.is_zero() {
        retire::write_int(state.hart, entry.rd, val);
        registers.retire_scalar(entry, false);
        trace_commit!(state.config.general.trace_instructions;
            pc       = %crate::common::trace::Hex(entry.pc),
            rob_tag  = entry.tag.0,
            reg      = entry.rd.as_usize(),
            rd_phys  = entry.phys_dst.0,
            old_phys = entry.old_phys_dst.0,
            value    = %crate::common::trace::Hex(val),
            is_fp    = false,
            "CM: integer register write"
        );
    }

    if let Some(writes) = &entry.vec_writes {
        retire::apply_vector_writes(state.hart, writes);
    }
    if entry.vec_dst_count > 0 {
        let vd_base = entry.ctrl.vd.as_u8();
        for i in 0..entry.vec_dst_count as usize {
            let vreg = VRegIdx::new(vd_base + i as u8);
            registers.retire_vec(state.hart.regs.vpr_mut(), entry, i, vreg);
        }
        retire::mark_vector_retired(state.hart);
    }

    #[cfg(feature = "commit-log")]
    if let Some((pc, inst, has_rd, rd, val)) = commit_log_entry
        && let Some(ref mut log) = state.commit_log
    {
        use std::io::Write;
        if has_rd {
            let _ = writeln!(log, "core   0: 0x{pc:016x} (0x{inst:08x}) x{rd} 0x{val:016x}");
        } else {
            let _ = writeln!(log, "core   0: 0x{pc:016x} (0x{inst:08x})");
        }
    }
}

/// Applies what execute deferred to retirement: hardware A/D bit updates,
/// accrued FP flags, vxsat, a fault-only-first vl trim and a vector
/// configuration.
fn apply_deferred_updates(state: &mut CoreCtx<'_>, common: &mut BackendCommon, entry: &RobEntry) {
    for update in entry.dirty_updates.iter() {
        if let Some(pte) = updated_pte(state, update) {
            let _ = write_store_to_memory(
                state,
                common,
                update.pte_addr,
                pte,
                MemWidth::Double,
                StoreOwner::Untracked,
            );
        }
    }

    // Apply fp_flags before CSR writes to keep execute-time CSR reads of fflags consistent.
    retire::accrue_fp_flags(state.hart, entry.fp_flags);
    if entry.vxsat {
        state.hart.csrs.vxsat = 1;
    }
    if let Some(vl) = entry.vl_trim {
        state.hart.csrs.vl = vl;
    }
    if let Some(vector) = entry.vec_csr_update {
        retire::apply_vector_config(state.hart, vector);
    }
}

/// Applies a CSR write, xRET or WFI, each of which ends the retire group.
fn retire_system(state: &mut CoreCtx<'_>, entry: &RobEntry) -> CommitFlow {
    if let Some(csr_update) = &entry.csr_update {
        // O3 applies fflags/fcsr eagerly at complete time; don't re-apply.
        if !csr_update.applied {
            let pc_before = state.hart.pc;
            state.csr_write(csr_update.addr, csr_update.new_val);
            // A write that trapped (the simulator panic CSR) moved the PC to a handler.
            if state.hart.pc != pc_before {
                return CommitFlow::Stop(Some(CommitEvent::SquashAfter(state.hart.pc)));
            }
        }
        trace_csr!(state.config.general.trace_instructions;
            op       = if csr_update.applied { "write-eager" } else { "write-deferred" },
            pc       = %crate::common::trace::Hex(entry.pc),
            rob_tag  = entry.tag.0,
            csr_addr = %crate::common::trace::Hex32(csr_update.addr.as_u32()),
            old_val  = %crate::common::trace::Hex(csr_update.old_val),
            new_val  = %crate::common::trace::Hex(csr_update.new_val),
            deferred = !csr_update.applied,
            "CM: CSR write applied at commit"
        );
        // SATP redirect: post-execute fetches used old tables; refetch from the next instruction.
        let satp_redirect = (csr_update.addr == csr::SATP)
            .then(|| CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64())));
        return CommitFlow::Stop(satp_redirect);
    }

    match entry.ctrl.system_op {
        SystemOp::Mret => {
            state.do_mret();
            trace_trap!(state.config.general.trace_instructions;
                event      = "return",
                insn       = "MRET",
                pc         = %crate::common::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::common::trace::Hex(state.hart.pc),
                mstatus    = %crate::common::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: MRET committed — privilege restored"
            );
            CommitFlow::Stop(Some(CommitEvent::SquashAfter(state.hart.pc)))
        }
        SystemOp::Sret => {
            state.do_sret();
            trace_trap!(state.config.general.trace_instructions;
                event      = "return",
                insn       = "SRET",
                pc         = %crate::common::trace::Hex(entry.pc),
                rob_tag    = entry.tag.0,
                return_pc  = %crate::common::trace::Hex(state.hart.pc),
                mstatus    = %crate::common::trace::Hex(state.hart.csrs.mstatus),
                priv_mode  = ?state.hart.privilege,
                "CM: SRET committed — privilege restored"
            );
            CommitFlow::Stop(Some(CommitEvent::SquashAfter(state.hart.pc)))
        }
        SystemOp::Wfi => CommitFlow::Stop(
            (!retire::wfi(state.hart))
                .then(|| CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64()))),
        ),
        _ => CommitFlow::Continue,
    }
}

/// Releases what the instruction held in the memory system and rename
/// state: sets an LR's reservation, commits its store-buffer entries,
/// frees its load-queue slot and branch checkpoint.
fn release_memory_resources(
    state: &mut CoreCtx<'_>,
    targets: &mut RetireTargets<'_, '_>,
    entry: &RobEntry,
) {
    // A reservation is set as its LR retires, so a squashed LR leaves none.
    if let Some(LrScRecord::Lr { paddr }) = entry.lr_sc {
        state.set_reservation(paddr);
    }

    if entry.ctrl.uses_store_buffer() {
        // A hart's own store (or CBO) to its reservation set fails its
        // SC, which the spec allows; other harts' reservations break
        // when the store is performed. An SC or AMO already took effect
        // in the cache and has left the store buffer.
        if let Some(paddr) = targets.store_buffer.find_paddr(entry.tag)
            && state.check_reservation(paddr)
        {
            state.clear_reservation();
        }
        targets.store_buffer.mark_committed(entry.tag);
    } else if is_vec_store(entry.ctrl.vec_op) {
        // Vector store data lives in the dedicated VecStoreBuffer.
        targets.store_buffer.mark_committed(entry.tag);
        targets.vec_store_buffer.mark_committed(entry.tag);
    }

    if entry.ctrl.mem_read || is_vec_load(entry.ctrl.vec_op) {
        targets.registers.release_load(entry.tag);
    }
    if let Some(ckpt_id) = entry.checkpoint_id {
        targets.registers.free_checkpoint(ckpt_id);
    }
}

/// Applies a FENCE.I or SFENCE.VMA, whose older stores have all been
/// written (commit waited for them), and returns the squash of the
/// younger instructions fetched or translated before it.
fn retire_fence(state: &mut CoreCtx<'_>, entry: &RobEntry) -> Option<CommitEvent> {
    let squash_younger = CommitEvent::SquashAfter(entry.pc.wrapping_add(entry.inst_size.as_u64()));
    if entry.ctrl.system_op == SystemOp::FenceI {
        state.core.l1_i_cache.invalidate_all();
        return Some(squash_younger);
    }
    if let Some(info) = entry.sfence_vma {
        state.core.mmu.sfence_vma(&info);
        state.clear_reservation();
        return Some(squash_younger);
    }
    None
}
