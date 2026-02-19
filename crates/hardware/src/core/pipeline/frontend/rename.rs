//! Rename Stage: ROB allocation, store buffer allocation, scoreboard marking.
//!
//! This stage takes decoded instructions (IdExEntry) and converts them into
//! RenameIssueEntry with ROB tags. It allocates ROB entries and store buffer
//! entries, and marks the scoreboard with the ROB tag for any destination
//! register. Operand values are NOT read here — that happens at issue.
//! Source register tags are captured BEFORE the scoreboard is updated for rd,
//! so that instructions reading their own destination (e.g. ADDI x5, x5, 16)
//! get the previous producer's tag, not their own.

use crate::core::Cpu;
use crate::core::pipeline::engine::ExecutionEngine;
use crate::core::pipeline::latches::{IdExEntry, RenameIssueEntry};

/// Executes the rename stage: allocate ROB/SB entries, capture source tags, mark scoreboard.
pub fn rename_stage<E: ExecutionEngine>(
    cpu: &mut Cpu,
    input: &mut Vec<IdExEntry>,
    engine: &mut E,
    rename_output: &mut Vec<RenameIssueEntry>,
) {
    let entries = std::mem::take(input);

    for id in entries {
        // Check if engine can accept more instructions
        if engine.can_accept() == 0 {
            // Put unconsumed entries back
            input.push(id);
            continue;
        }

        // Allocate ROB entry
        let rob_tag = match engine.rob_mut().allocate(
            id.pc,
            id.inst,
            id.inst_size,
            id.rd,
            id.ctrl.fp_reg_write,
            id.ctrl,
        ) {
            Some(tag) => tag,
            None => {
                // ROB full, stall
                input.push(id);
                break;
            }
        };

        // Capture source register tags BEFORE updating scoreboard for rd.
        // This ensures that if rs == rd, we get the PREVIOUS producer tag,
        // not the one we're about to set for ourselves.
        let rs1_tag = engine.scoreboard().get_producer(id.rs1, id.ctrl.rs1_fp);
        let rs2_tag = engine.scoreboard().get_producer(id.rs2, id.ctrl.rs2_fp);
        let rs3_tag = if id.ctrl.rs3_fp {
            engine.scoreboard().get_producer(id.rs3, true)
        } else {
            None
        };

        // Mark scoreboard: this instruction will write rd
        if id.ctrl.reg_write || id.ctrl.fp_reg_write {
            engine
                .scoreboard_mut()
                .set_producer(id.rd, id.ctrl.fp_reg_write, rob_tag);
        }

        // Allocate store buffer entry if this is a store
        if id.ctrl.mem_write && id.ctrl.atomic_op == crate::core::pipeline::signals::AtomicOp::None
        {
            let width = id.ctrl.width;
            if !engine.store_buffer_mut().allocate(rob_tag, width) {
                input.push(id);
                break;
            }
        }

        // Create RenameIssueEntry — operand values are 0, read at issue stage
        let entry = RenameIssueEntry {
            rob_tag,
            pc: id.pc,
            inst: id.inst,
            inst_size: id.inst_size,
            rs1: id.rs1,
            rs2: id.rs2,
            rs3: id.rs3,
            rd: id.rd,
            imm: id.imm,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag,
            rs2_tag,
            rs3_tag,
            ctrl: id.ctrl,
            trap: id.trap,
            exception_stage: id.exception_stage,
            pred_taken: id.pred_taken,
            pred_target: id.pred_target,
        };

        if cpu.trace {
            eprintln!("RN  pc={:#x} rob_tag={}", entry.pc, entry.rob_tag.0);
        }

        rename_output.push(entry);
    }
}
