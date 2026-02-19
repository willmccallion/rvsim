//! Writeback Stage: mark ROB entries as Completed with result values.
//!
//! This stage takes results from Memory2 and marks the corresponding ROB
//! entries as Completed. It does NOT write to the register file — that
//! happens at commit.

use crate::common::ExceptionStage;
use crate::core::Cpu;
use crate::core::pipeline::latches::Mem2WbEntry;
use crate::core::pipeline::rob::Rob;

/// Executes the Writeback stage: mark ROB entries Completed.
pub fn writeback_stage(cpu: &mut Cpu, input: &mut Vec<Mem2WbEntry>, rob: &mut Rob) {
    let entries = std::mem::take(input);

    for wb in entries {
        if let Some(ref trap) = wb.trap {
            // Mark as faulted
            rob.fault(
                wb.rob_tag,
                trap.clone(),
                wb.exception_stage.unwrap_or(ExceptionStage::Memory),
            );
            if cpu.trace {
                eprintln!(
                    "WB  pc={:#x} rob_tag={} FAULTED: {:?}",
                    wb.pc, wb.rob_tag.0, trap
                );
            }
            continue;
        }

        // Compute the result value
        let val = if wb.ctrl.mem_read {
            wb.load_data
        } else if wb.ctrl.jump {
            wb.pc.wrapping_add(wb.inst_size)
        } else {
            wb.alu
        };

        rob.complete(wb.rob_tag, val);

        if cpu.trace {
            eprintln!(
                "WB  pc={:#x} rob_tag={} result={:#x}",
                wb.pc, wb.rob_tag.0, val
            );
        }
    }
}
