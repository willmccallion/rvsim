//! Writeback stage: applies deferred ROB metadata and completes ROB entries.
//!
//! In the packet-based pipeline this stage runs at its usual place — after
//! Memory2 — and consumes the Memory2→Writeback latch. Each entry carries the
//! final scalar value (load data, ALU result, or `pc + inst_size` for jumps)
//! and any deferred state that must land on the ROB before commit can apply
//! it (FP flags, PTE A/D updates, SFENCE.VMA operands, LR/SC reservation
//! records).

use crate::common::ExceptionStage;
use crate::sim::SimState;
use crate::core::pipeline::latches::Mem2WbEntry;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::signals::ControlFlow;
use crate::trace_trap;
use crate::trace_writeback;

/// Executes the Writeback stage: marks ROB entries Completed (or Faulted).
pub fn writeback_stage(cpu: &mut SimState, input: &mut Vec<Mem2WbEntry>, rob: &mut Rob) {
    let entries = std::mem::take(input);

    for wb in entries {
        if let Some(ref trap) = wb.trap {
            if wb.fp_flags != 0 {
                rob.set_fp_flags(wb.rob_tag, wb.fp_flags);
            }
            rob.fault(
                wb.rob_tag,
                trap.clone(),
                wb.exception_stage.unwrap_or(ExceptionStage::Memory),
            );
            trace_trap!(cpu.config.general.trace_instructions;
                event   = "writeback-fault",
                pc      = %crate::trace::Hex(wb.pc),
                rob_tag = wb.rob_tag.0,
                trap    = ?trap,
                stage   = ?wb.exception_stage,
                "WB: entry marked faulted in ROB"
            );
            continue;
        }

        let val = if wb.ctrl.mem_read || wb.ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None {
            wb.load_data
        } else if wb.ctrl.control_flow == ControlFlow::Jump {
            wb.pc.wrapping_add(wb.inst_size.as_u64())
        } else {
            wb.alu
        };

        if wb.fp_flags != 0 {
            rob.set_fp_flags(wb.rob_tag, wb.fp_flags);
        }
        if let Some(pte_upd) = wb.pte_update {
            rob.set_pte_update(wb.rob_tag, pte_upd);
        }
        if let Some(sfence_info) = wb.sfence_vma {
            rob.set_sfence_vma(wb.rob_tag, sfence_info);
        }
        if let Some(lr_sc_rec) = wb.lr_sc {
            rob.set_lr_sc(wb.rob_tag, lr_sc_rec);
        }
        rob.complete(wb.rob_tag, val);

        trace_writeback!(cpu.config.general.trace_instructions;
            rob_tag  = wb.rob_tag.0,
            pc       = %crate::trace::Hex(wb.pc),
            result   = %crate::trace::Hex(val),
            from     = if wb.ctrl.mem_read { "load" } else if wb.ctrl.control_flow == ControlFlow::Jump { "jump_link" } else { "alu" },
            rd       = wb.rd.as_usize(),
            rd_phys  = wb.rd_phys.0,
            fp_flags = wb.fp_flags,
            "WB: ROB entry marked complete"
        );
    }
}
