//! Writeback stage: applies deferred ROB metadata and completes ROB entries.
//!
//! In the packet-based pipeline this stage runs at its usual place — after
//! Memory2 — and consumes the Memory2→Writeback latch. Each entry carries the
//! final scalar value (load data, ALU result, or `pc + inst_size` for jumps)
//! and any deferred state that must land on the ROB before commit can apply
//! it (FP flags, PTE A/D updates, SFENCE.VMA operands, LR/SC reservation
//! records).

use crate::common::trace::Hex;
use crate::exec::signals::ControlFlow;
use crate::trace_trap;
use crate::trace_writeback;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::latches::Mem2WbEntry;
use crate::uarch::pipeline::rob::Rob;
use crate::uarch::pipeline::trace::Stage;

/// Executes the Writeback stage: marks ROB entries Completed (or Faulted).
pub fn writeback_stage(state: &mut StageCtx<'_>, input: &mut Vec<Mem2WbEntry>, rob: &mut Rob) {
    let entries = std::mem::take(input);

    for wb in entries {
        if let Some(seq) = rob.find_entry(wb.op.rob_tag).map(|entry| entry.seq) {
            state.trace_stage(seq, Stage::Writeback);
        }
        if let Some(ref trap) = wb.op.trap {
            if !wb.op.fp_flags.is_empty() {
                rob.set_fp_flags(wb.op.rob_tag, wb.op.fp_flags);
            }
            rob.fault(wb.op.rob_tag, trap.clone());
            trace_trap!(state.trace_trap_enabled(trap);
                event   = "writeback-fault",
                pc      = %Hex(wb.op.pc.val()),
                rob_tag = %wb.op.rob_tag,
                trap    = ?trap,
                "WB: entry marked faulted in ROB"
            );
            continue;
        }

        let val = wb.written_value();

        if !wb.op.fp_flags.is_empty() {
            rob.set_fp_flags(wb.op.rob_tag, wb.op.fp_flags);
        }
        if !wb.dirty_updates.is_empty() {
            rob.set_dirty_updates(wb.op.rob_tag, wb.dirty_updates);
        }
        if let Some(sfence_info) = wb.op.sfence_vma {
            rob.set_sfence_vma(wb.op.rob_tag, sfence_info);
        }
        if let Some(lr_sc_rec) = wb.lr_sc {
            rob.set_lr_sc(wb.op.rob_tag, lr_sc_rec);
        }
        if let Some(seq) = wb.observed {
            rob.set_observed(wb.op.rob_tag, seq);
        }
        if let Some(access) = wb.mem_access {
            rob.set_mem_access(wb.op.rob_tag, access);
        }
        #[cfg(feature = "commit-log")]
        if let Some(effect) = wb.mem_effect {
            rob.set_mem_effect(wb.op.rob_tag, effect);
        }
        rob.complete(wb.op.rob_tag, val);

        trace_writeback!(state.config.general.trace_instructions;
            rob_tag  = %wb.op.rob_tag,
            pc       = %Hex(wb.op.pc.val()),
            result   = %Hex(val),
            from     = if wb.op.ctrl.mem_read { "load" } else if matches!(wb.op.ctrl.control_flow, ControlFlow::Jump(_)) { "jump_link" } else { "alu" },
            rd       = wb.op.rd.as_usize(),
            rd_phys  = wb.op.rd_phys.index(),
            fp_flags = wb.op.fp_flags.bits(),
            "WB: ROB entry marked complete"
        );
    }
}
