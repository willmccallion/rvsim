//! Writeback (WB) Stage.
//!
//! This module implements the final stage of the instruction pipeline.
//! It commits results to the register file (integer or floating-point),
//! handles traps and interrupts (including delegation), and updates
//! performance statistics. It also manages pipeline flushing upon exceptions.

use crate::common::constants::{
    DELEG_MEIP_BIT, DELEG_MSIP_BIT, DELEG_MTIP_BIT, DELEG_SEIP_BIT, DELEG_SSIP_BIT, DELEG_STIP_BIT,
};
use crate::core::Cpu;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::arch::trap::TrapHandler;
use crate::core::cpu::PC_TRACE_MAX;
use crate::core::pipeline::signals::AluOp;

/// Executes the writeback stage of the pipeline.
///
/// Writes instruction results back to registers, handles trap and interrupt
/// detection, updates performance statistics, and manages pipeline flushing
/// on exceptions. This is the final stage of the pipeline.
///
/// # Arguments
///
/// * `cpu` - Mutable reference to the CPU state
///
/// # Behavior
///
/// - Writes ALU results, load data, or jump targets to destination registers
/// - Detects pending interrupts and exceptions
/// - Updates instruction retirement statistics
/// - Handles trap processing and privilege mode transitions
/// - Flushes pipeline on trap events
pub fn wb_stage(cpu: &mut Cpu) {
    let mut trap_event: Option<(crate::common::error::Trap, u64)> = None;

    if !cpu.mem_wb.entries.is_empty() || cpu.wfi_waiting {
        if cpu.interrupt_inhibit_one_cycle {
            cpu.interrupt_inhibit_one_cycle = false;
        } else {
            let interrupt_pc = if !cpu.mem_wb.entries.is_empty() {
                cpu.mem_wb.entries[0].pc
            } else {
                0
            };

            let mip = cpu.csrs.mip;
            let mie = cpu.csrs.mie;
            let mstatus = cpu.csrs.mstatus;

            let m_global_ie = (mstatus & csr::MSTATUS_MIE) != 0;
            let s_global_ie = (mstatus & csr::MSTATUS_SIE) != 0;

            let check =
                |bit: u64, enable_bit: u64, deleg_bit: u64| -> Option<crate::common::error::Trap> {
                    let pending = (mip & bit) != 0;
                    let enabled = (mie & enable_bit) != 0;
                    if !pending || !enabled {
                        return None;
                    }

                    let delegated = (cpu.csrs.mideleg & deleg_bit) != 0;
                    let target_priv = if delegated {
                        PrivilegeMode::Supervisor
                    } else {
                        PrivilegeMode::Machine
                    };

                    if cpu.privilege.to_u8() < target_priv.to_u8() {
                        return Some(TrapHandler::irq_to_trap(bit));
                    }
                    if cpu.privilege == target_priv {
                        if target_priv == PrivilegeMode::Machine && m_global_ie {
                            return Some(TrapHandler::irq_to_trap(bit));
                        }
                        if target_priv == PrivilegeMode::Supervisor && s_global_ie {
                            return Some(TrapHandler::irq_to_trap(bit));
                        }
                    }
                    None
                };

            let interrupt = check(csr::MIP_MEIP, csr::MIE_MEIP, 1 << DELEG_MEIP_BIT)
                .or_else(|| check(csr::MIP_MSIP, csr::MIE_MSIP, 1 << DELEG_MSIP_BIT))
                .or_else(|| check(csr::MIP_MTIP, csr::MIE_MTIE, 1 << DELEG_MTIP_BIT))
                .or_else(|| check(csr::MIP_SEIP, csr::MIE_SEIP, 1 << DELEG_SEIP_BIT))
                .or_else(|| check(csr::MIP_SSIP, csr::MIE_SSIP, 1 << DELEG_SSIP_BIT))
                .or_else(|| check(csr::MIP_STIP, csr::MIE_STIE, 1 << DELEG_STIP_BIT));

            if let Some(interrupt_trap) = interrupt {
                let epc = if cpu.wfi_waiting {
                    cpu.wfi_pc
                } else {
                    interrupt_pc
                };
                cpu.wfi_waiting = false;
                if cpu.trace {
                    eprintln!(
                        "WB  pc={:#x} * INTERRUPT DETECTED: {:?}",
                        epc, interrupt_trap
                    );
                }
                trap_event = Some((interrupt_trap, epc));
            } else if cpu.wfi_waiting {
                // WFI Wakeup Logic (without trap)
                // If global interrupts are disabled, WFI can still wake up if a locally enabled
                // interrupt becomes pending. Execution resumes at the next instruction.
                let pending = cpu.csrs.mip;
                let enabled = cpu.csrs.mie;
                if (pending & enabled) != 0 {
                    cpu.wfi_waiting = false;
                    cpu.pc = cpu.wfi_pc;
                }
            }
        }
    }

    let mut processed_count = 0;
    for (idx, wb) in cpu.mem_wb.entries.iter().enumerate() {
        if trap_event.is_some() {
            break;
        }

        if let Some(trap) = &wb.trap {
            if cpu.trace {
                eprintln!("WB  pc={:#x} * TRAP DETECTED: {:?}", wb.pc, trap);
            }
            trap_event = Some((trap.clone(), wb.pc));

            cpu.mem_wb.entries.truncate(idx);
            break;
        }

        processed_count = idx + 1;

        if cpu.trace {
            eprintln!("WB  pc={:#x}", wb.pc);
        }

        cpu.pc_trace.push((wb.pc, wb.inst));
        if cpu.pc_trace.len() > PC_TRACE_MAX {
            cpu.pc_trace.remove(0);
        }

        if wb.inst != 0 && wb.inst != 0x13 {
            cpu.stats.instructions_retired += 1;
            if wb.ctrl.mem_read {
                if wb.ctrl.fp_reg_write {
                    cpu.stats.inst_fp_load += 1;
                } else {
                    cpu.stats.inst_load += 1;
                }
            } else if wb.ctrl.mem_write {
                if wb.ctrl.rs2_fp {
                    cpu.stats.inst_fp_store += 1;
                } else {
                    cpu.stats.inst_store += 1;
                }
            } else if wb.ctrl.branch || wb.ctrl.jump {
                cpu.stats.inst_branch += 1;
            } else if wb.ctrl.is_system {
                cpu.stats.inst_system += 1;
            } else {
                match wb.ctrl.alu {
                    AluOp::FAdd
                    | AluOp::FSub
                    | AluOp::FMul
                    | AluOp::FMin
                    | AluOp::FMax
                    | AluOp::FSgnJ
                    | AluOp::FSgnJN
                    | AluOp::FSgnJX
                    | AluOp::FEq
                    | AluOp::FLt
                    | AluOp::FLe
                    | AluOp::FClass
                    | AluOp::FCvtWS
                    | AluOp::FCvtLS
                    | AluOp::FCvtSW
                    | AluOp::FCvtSL
                    | AluOp::FCvtSD
                    | AluOp::FCvtDS
                    | AluOp::FMvToX
                    | AluOp::FMvToF => cpu.stats.inst_fp_arith += 1,
                    AluOp::FDiv | AluOp::FSqrt => cpu.stats.inst_fp_div_sqrt += 1,
                    AluOp::FMAdd | AluOp::FMSub | AluOp::FNMAdd | AluOp::FNMSub => {
                        cpu.stats.inst_fp_fma += 1
                    }
                    _ => cpu.stats.inst_alu += 1,
                }
            }
        }

        let val = if wb.ctrl.mem_read {
            wb.load_data
        } else if wb.ctrl.jump {
            wb.pc.wrapping_add(wb.inst_size)
        } else {
            wb.alu
        };

        if cpu.trace {
            if wb.ctrl.reg_write {
                eprintln!("WB  pc={:#x} x{} <= {:#x}", wb.pc, wb.rd, val);
            } else if wb.ctrl.fp_reg_write {
                eprintln!("WB  pc={:#x} f{} <= {:#x}", wb.pc, wb.rd, val);
            }
        }

        if wb.ctrl.fp_reg_write {
            cpu.regs.write_f(wb.rd, val);
        } else if wb.ctrl.reg_write && wb.rd != 0 {
            cpu.regs.write(wb.rd, val);
        }
    }

    if processed_count < cpu.mem_wb.entries.len() {
        cpu.mem_wb.entries.truncate(processed_count);
    }

    if let Some((trap, pc)) = trap_event {
        if cpu.trace {
            eprintln!("WB  * HANDLING TRAP: {:?} at PC {:#x}", trap, pc);
        }
        cpu.if_id = Default::default();
        cpu.id_ex = Default::default();
        cpu.ex_mem = Default::default();
        cpu.wb_latch = Default::default();

        cpu.mem_wb = Default::default();

        let exit_code_before = cpu.exit_code.is_some();
        cpu.trap(trap, pc);

        if cpu.trace && !cpu.exit_code.is_some() {
            eprintln!("WB  * TRAP HANDLED, new PC={:#x}", cpu.pc);
        } else if cpu.trace && cpu.exit_code.is_some() && !exit_code_before {
            eprintln!("WB  * TRAP CAUSED EXIT (direct mode)");
        }
    }
}
