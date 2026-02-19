//! Fetch1 Stage: PC generation, branch prediction, I-TLB lookup.
//!
//! This is the first stage of the frontend. It generates PCs for fetch,
//! performs branch prediction to determine the next PC, and initiates
//! I-TLB lookups for address translation.

use crate::common::constants::{
    COMPRESSED_INSTRUCTION_MASK, COMPRESSED_INSTRUCTION_VALUE, INSTRUCTION_SIZE_16,
    INSTRUCTION_SIZE_32, OPCODE_MASK, RD_MASK, RD_SHIFT, RS1_MASK, RS1_SHIFT,
};
use crate::common::{AccessType, ExceptionStage, TranslationResult, Trap, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::Fetch1Fetch2Entry;
use crate::core::units::bru::BranchPredictor;
use crate::isa::abi;
use crate::isa::rv64i::opcodes;

/// Executes the Fetch1 stage: PC generation + I-TLB + branch prediction.
///
/// Produces entries in the Fetch1->Fetch2 latch with physical addresses
/// and prediction information.
pub fn fetch1_stage(cpu: &mut Cpu, output: &mut Vec<Fetch1Fetch2Entry>, stall_out: &mut u64) {
    output.clear();

    let mut current_pc = cpu.pc;

    for _ in 0..cpu.pipeline_width {
        // Check alignment
        let mut fetch_trap = None;
        if (current_pc & 1) != 0 {
            if output.is_empty() {
                fetch_trap = Some(Trap::InstructionAddressMisaligned(current_pc));
            } else {
                break;
            }
        }

        // I-TLB lookup
        let TranslationResult {
            paddr,
            cycles,
            trap,
        } = if fetch_trap.is_none() {
            cpu.translate(VirtAddr::new(current_pc), AccessType::Fetch)
        } else {
            TranslationResult {
                paddr: crate::common::PhysAddr::new(0),
                cycles: 0,
                trap: None,
            }
        };
        *stall_out += cycles;

        let trap_cause = fetch_trap.or(trap);
        if let Some(ref trap_cause) = trap_cause {
            if cpu.trace {
                eprintln!("F1  pc={:#x} # TRAP: {:?}", current_pc, trap_cause);
            }
            output.push(Fetch1Fetch2Entry {
                pc: current_pc,
                paddr: 0,
                pred_taken: false,
                pred_target: 0,
                trap: Some(trap_cause.clone()),
                exception_stage: Some(ExceptionStage::Fetch),
            });
            break;
        }

        let phys_addr = paddr.val();

        // Read the first half-word to determine instruction type for prediction
        let half_word = if phys_addr >= cpu.ram_start && phys_addr < cpu.ram_end {
            let offset = (phys_addr - cpu.ram_start) as usize;
            unsafe {
                let ptr = cpu.ram_ptr.add(offset) as *const u16;
                ptr.read_unaligned()
            }
        } else {
            cpu.bus.bus.read_u16(phys_addr)
        };

        let is_compressed =
            (half_word & COMPRESSED_INSTRUCTION_MASK) != COMPRESSED_INSTRUCTION_VALUE;

        let step = if is_compressed {
            INSTRUCTION_SIZE_16
        } else {
            INSTRUCTION_SIZE_32
        };

        // Branch prediction (peek at opcode from half_word for 32-bit instructions)
        let mut next_pc_calc = current_pc.wrapping_add(step);
        let mut pred_taken = false;
        let mut pred_target = 0;
        let mut stop_fetch = false;

        if !is_compressed {
            // For 32-bit instructions, read full instruction for opcode extraction
            let upper_va = current_pc.wrapping_add(2);
            let crosses_page = (current_pc >> 12) != (upper_va >> 12);
            let upper_phys = if crosses_page {
                let result = cpu.translate(VirtAddr::new(upper_va), AccessType::Fetch);
                *stall_out += result.cycles;
                if result.trap.is_some() {
                    // Page crossing fault; let fetch2 handle it
                    output.push(Fetch1Fetch2Entry {
                        pc: current_pc,
                        paddr: phys_addr,
                        pred_taken: false,
                        pred_target: 0,
                        trap: None,
                        exception_stage: None,
                    });
                    cpu.pc = next_pc_calc;
                    break;
                }
                result.paddr.val()
            } else {
                phys_addr + 2
            };

            let upper_half = if upper_phys >= cpu.ram_start && upper_phys < cpu.ram_end {
                let offset = (upper_phys - cpu.ram_start) as usize;
                unsafe {
                    let ptr = cpu.ram_ptr.add(offset) as *const u16;
                    ptr.read_unaligned()
                }
            } else {
                cpu.bus.bus.read_u16(upper_phys)
            };

            let full_inst = (upper_half as u32) << 16 | (half_word as u32);
            let opcode = full_inst & OPCODE_MASK;
            let rd = ((full_inst >> RD_SHIFT) & RD_MASK) as usize;
            let rs1 = ((full_inst >> RS1_SHIFT) & RS1_MASK) as usize;

            if opcode == opcodes::OP_BRANCH {
                let (taken, target) = cpu.branch_predictor.predict_branch(current_pc);
                if taken && let Some(tgt) = target {
                    next_pc_calc = tgt;
                    pred_taken = true;
                    pred_target = tgt;
                    stop_fetch = true;
                }
            } else if opcode == opcodes::OP_JAL {
                if let Some(tgt) = cpu.branch_predictor.predict_btb(current_pc) {
                    next_pc_calc = tgt;
                    pred_taken = true;
                    pred_target = tgt;
                    stop_fetch = true;
                }
            } else if opcode == opcodes::OP_JALR {
                if rd == abi::REG_ZERO && rs1 == abi::REG_RA {
                    if let Some(tgt) = cpu.branch_predictor.predict_return() {
                        next_pc_calc = tgt;
                        pred_taken = true;
                        pred_target = tgt;
                    }
                } else if let Some(tgt) = cpu.branch_predictor.predict_btb(current_pc) {
                    next_pc_calc = tgt;
                    pred_taken = true;
                    pred_target = tgt;
                }
                stop_fetch = true;
            }
        }

        output.push(Fetch1Fetch2Entry {
            pc: current_pc,
            paddr: phys_addr,
            pred_taken,
            pred_target,
            trap: None,
            exception_stage: None,
        });

        current_pc = next_pc_calc;
        if stop_fetch {
            break;
        }
    }

    cpu.pc = current_pc;
}
