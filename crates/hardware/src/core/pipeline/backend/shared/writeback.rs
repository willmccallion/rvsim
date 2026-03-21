//! Writeback Stage: mark ROB entries as Completed with result values.
//!
//! This stage takes results from Memory2 and marks the corresponding ROB
//! entries as Completed. It does NOT write to the register file — that
//! happens at commit.

use crate::common::ExceptionStage;
use crate::core::Cpu;
use crate::core::pipeline::latches::Mem2WbEntry;
use crate::core::pipeline::rob::Rob;
use crate::core::pipeline::signals::ControlFlow;
use crate::trace_trap;
use crate::trace_writeback;

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
            trace_trap!(cpu.trace;
                event   = "writeback-fault",
                pc      = %crate::trace::Hex(wb.pc),
                rob_tag = wb.rob_tag.0,
                trap    = ?trap,
                stage   = ?wb.exception_stage,
                "WB: entry marked faulted in ROB"
            );
            continue;
        }

        // Compute the result value
        let val = if wb.ctrl.mem_read {
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

        trace_writeback!(cpu.trace;
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

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::common::{InstSize, RegIdx, Trap};
    use crate::config::Config;
    use crate::core::pipeline::signals::ControlSignals;
    use crate::soc::builder::System;

    #[test]
    fn test_writeback_stage_normal() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        let mut rob = Rob::new(4);

        let rob_tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();

        let mut input = vec![Mem2WbEntry {
            rob_tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            alu: 42,
            load_data: 0,
            ctrl: ControlSignals::default(),
            trap: None,
            exception_stage: None,
            fp_flags: 0,
            pte_update: None,
            sfence_vma: None,
            lr_sc: None,
        }];

        writeback_stage(&mut cpu, &mut input, &mut rob);

        assert!(input.is_empty());
        let entry = rob.find_entry(rob_tag).unwrap();
        assert_eq!(entry.state, crate::core::pipeline::rob::RobState::Completed);
        assert_eq!(entry.result, Some(42));
    }

    #[test]
    fn test_writeback_stage_trap() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        let mut rob = Rob::new(4);

        let rob_tag = rob
            .allocate(
                0x1000,
                0,
                InstSize::Standard,
                RegIdx::new(1),
                false,
                ControlSignals::default(),
                crate::core::pipeline::prf::PhysReg(0),
                crate::core::pipeline::prf::PhysReg(0),
            )
            .unwrap();

        let mut input = vec![Mem2WbEntry {
            rob_tag,
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            alu: 0,
            load_data: 0,
            ctrl: ControlSignals::default(),
            trap: Some(Trap::IllegalInstruction(0)),
            exception_stage: Some(ExceptionStage::Execute),
            fp_flags: 0,
            pte_update: None,
            sfence_vma: None,
            lr_sc: None,
        }];

        writeback_stage(&mut cpu, &mut input, &mut rob);

        assert!(input.is_empty());
        let entry = rob.find_entry(rob_tag).unwrap();
        assert_eq!(entry.state, crate::core::pipeline::rob::RobState::Faulted);
        assert!(entry.trap.is_some());
    }
}
