//! Fetch1 Stage: PC generation, branch prediction, I-TLB lookup.
//!
//! This is the first stage of the frontend. It generates PCs for fetch,
//! performs branch prediction to determine the next PC, and initiates
//! I-TLB lookups for address translation.

// RISC-V instructions may be misaligned (compressed 16-bit instructions); read_unaligned is intentional.
#![allow(clippy::cast_ptr_alignment)]

use crate::common::InstSize;
use crate::common::constants::{
    COMPRESSED_INSTRUCTION_MASK, COMPRESSED_INSTRUCTION_VALUE, OPCODE_MASK, RD_MASK, RD_SHIFT,
    RS1_MASK, RS1_SHIFT,
};
use crate::common::{
    AccessType, ExceptionStage, PhysAddr, RegIdx, TranslationResult, Trap, VirtAddr,
};
use crate::core::Cpu;
use crate::core::arch::csr;
use crate::core::pipeline::latches::Fetch1Fetch2Entry;
use crate::core::units::bru::{BranchPredictor, Ghr};
use crate::isa::abi;
use crate::isa::rv64i::opcodes;
use crate::trace_branch;
use crate::trace_fetch;

/// Executes the Fetch1 stage: PC generation + I-TLB + branch prediction.
///
/// Produces entries in the Fetch1->Fetch2 latch with physical addresses
/// and prediction information.
pub fn fetch1_stage(cpu: &mut Cpu, output: &mut Vec<Fetch1Fetch2Entry>, stall_out: &mut u64) {
    output.clear();

    let mut current_pc = cpu.pc;
    // When MISA[C]=0, compressed instructions are disabled; require 4-byte alignment.
    let c_enabled = (cpu.csrs.misa & csr::MISA_EXT_C) != 0;
    let align_mask: u64 = if c_enabled { 1 } else { 3 };

    // Cache-line-aligned fetch: a real frontend fetches one cache line per cycle.
    // If the PC is near the end of a line, only the remaining bytes are available.
    // We track a byte budget and stop when the next instruction wouldn't fit.
    let line_bytes = cpu.i_cache_line_bytes as u64;
    let line_end = (current_pc | (line_bytes - 1)) + 1; // end of current cache line

    for _ in 0..cpu.pipeline_width {
        // Stop if fewer than 2 bytes remain in this cache line (minimum instruction size).
        if current_pc + 2 > line_end {
            break;
        }
        // Check alignment
        let mut fetch_trap = None;
        if (current_pc & align_mask) != 0 {
            if output.is_empty() {
                fetch_trap = Some(Trap::InstructionAddressMisaligned(current_pc));
            } else {
                break;
            }
        }

        // I-TLB lookup
        let TranslationResult { paddr, cycles, trap, .. } = if fetch_trap.is_none() {
            cpu.translate(VirtAddr::new(current_pc), AccessType::Fetch, 4)
        } else {
            TranslationResult::success(crate::common::PhysAddr::new(0), 0)
        };
        *stall_out += cycles;

        let trap_cause = fetch_trap.or(trap);
        if let Some(ref trap_cause) = trap_cause {
            trace_fetch!(cpu.trace;
                pc          = %crate::trace::Hex(current_pc),
                tlb_cycles  = cycles,
                trap        = ?trap_cause,
                "F1: fetch trap"
            );
            output.push(Fetch1Fetch2Entry {
                pc: current_pc,
                paddr: PhysAddr::new(0),
                pred_taken: false,
                pred_target: 0,
                trap: Some(trap_cause.clone()),
                exception_stage: Some(ExceptionStage::Fetch),
                ghr_snapshot: Ghr::default(),
                ras_snapshot: 0,
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
            cpu.bus.bus.read_u16(paddr)
        };

        let is_compressed =
            (half_word & COMPRESSED_INSTRUCTION_MASK) != COMPRESSED_INSTRUCTION_VALUE;

        let step = if is_compressed { InstSize::Compressed } else { InstSize::Standard };

        // Branch prediction (peek at opcode from half_word for 32-bit instructions)
        let mut next_pc_calc = current_pc.wrapping_add(step.as_u64());
        let mut pred_taken = false;
        let mut pred_target = 0;
        let mut stop_fetch = false;
        let ghr_snapshot = cpu.branch_predictor.snapshot_history();
        let ras_snapshot = cpu.branch_predictor.snapshot_ras();

        if is_compressed {
            // Compressed branch prediction: detect C.BEQZ / C.BNEZ
            // Quadrant 1 (bits 1:0 = 01), funct3 = 110 or 111
            let quadrant = half_word & 0x3;
            let funct3_c = (half_word >> 13) & 0x7;
            if quadrant == 0x01 && (funct3_c == 0b110 || funct3_c == 0b111) {
                let (taken, target) = cpu.branch_predictor.predict_branch(current_pc);
                cpu.branch_predictor.speculate(current_pc, taken);
                if taken && let Some(tgt) = target {
                    next_pc_calc = tgt;
                    pred_taken = true;
                    pred_target = tgt;
                    stop_fetch = true;
                }
                trace_branch!(cpu.trace;
                    event        = "predict",
                    pc           = %crate::trace::Hex(current_pc),
                    paddr        = %crate::trace::Hex(phys_addr),
                    bp_type      = "compressed-branch",
                    pred_taken   = taken,
                    pred_target  = %crate::trace::Hex(target.unwrap_or(0)),
                    "F1: compressed branch prediction"
                );
            }
        } else {
            // For 32-bit instructions, read full instruction for opcode extraction
            let upper_va = current_pc.wrapping_add(2);
            let crosses_page = (current_pc >> 12) != (upper_va >> 12);
            let upper_phys = if crosses_page {
                let result = cpu.translate(VirtAddr::new(upper_va), AccessType::Fetch, 2);
                *stall_out += result.cycles;
                if result.trap.is_some() {
                    // Page crossing fault; let fetch2 handle it
                    trace_fetch!(cpu.trace;
                        pc           = %crate::trace::Hex(current_pc),
                        paddr        = %crate::trace::Hex(phys_addr),
                        crosses_page = true,
                        "F1: page-crossing fault deferred to F2"
                    );
                    output.push(Fetch1Fetch2Entry {
                        pc: current_pc,
                        paddr: PhysAddr::new(phys_addr),
                        pred_taken: false,
                        pred_target: 0,
                        trap: None,
                        exception_stage: None,
                        ghr_snapshot: Ghr::default(),
                        ras_snapshot,
                    });
                    cpu.pc = next_pc_calc;
                    break;
                }
                result.paddr
            } else {
                PhysAddr::new(phys_addr + 2)
            };

            let upper_raw = upper_phys.val();
            let upper_half = if upper_raw >= cpu.ram_start && upper_raw < cpu.ram_end {
                let offset = (upper_raw - cpu.ram_start) as usize;
                unsafe {
                    let ptr = cpu.ram_ptr.add(offset) as *const u16;
                    ptr.read_unaligned()
                }
            } else {
                cpu.bus.bus.read_u16(upper_phys)
            };

            let full_inst = (upper_half as u32) << 16 | (half_word as u32);
            let opcode = full_inst & OPCODE_MASK;
            let rd = RegIdx::new(((full_inst >> RD_SHIFT) & RD_MASK) as u8);
            let rs1 = RegIdx::new(((full_inst >> RS1_SHIFT) & RS1_MASK) as u8);

            if opcode == opcodes::OP_BRANCH {
                let (taken, target) = cpu.branch_predictor.predict_branch(current_pc);
                cpu.branch_predictor.speculate(current_pc, taken);
                if taken && let Some(tgt) = target {
                    next_pc_calc = tgt;
                    pred_taken = true;
                    pred_target = tgt;
                    stop_fetch = true;
                }
                trace_branch!(cpu.trace;
                    event        = "predict",
                    pc           = %crate::trace::Hex(current_pc),
                    paddr        = %crate::trace::Hex(phys_addr),
                    inst         = %crate::trace::Hex32(full_inst),
                    bp_type      = "branch",
                    pred_taken   = taken,
                    pred_target  = %crate::trace::Hex(target.unwrap_or(0)),
                    "F1: branch prediction"
                );
            } else if opcode == opcodes::OP_JAL {
                if let Some(tgt) = cpu.branch_predictor.predict_btb(current_pc) {
                    next_pc_calc = tgt;
                    pred_taken = true;
                    pred_target = tgt;
                    stop_fetch = true;
                }
                trace_branch!(cpu.trace;
                    event       = "predict",
                    pc          = %crate::trace::Hex(current_pc),
                    paddr       = %crate::trace::Hex(phys_addr),
                    inst        = %crate::trace::Hex32(full_inst),
                    bp_type     = "JAL/BTB",
                    pred_taken  = pred_taken,
                    pred_target = %crate::trace::Hex(pred_target),
                    "F1: JAL prediction"
                );
            } else if opcode == opcodes::OP_JALR {
                let is_ret = rd == abi::REG_ZERO && rs1 == abi::REG_RA;
                if is_ret {
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
                trace_branch!(cpu.trace;
                    event       = "predict",
                    pc          = %crate::trace::Hex(current_pc),
                    paddr       = %crate::trace::Hex(phys_addr),
                    inst        = %crate::trace::Hex32(full_inst),
                    bp_type     = if is_ret { "JALR/RAS" } else { "JALR/BTB" },
                    pred_taken  = pred_taken,
                    pred_target = %crate::trace::Hex(pred_target),
                    "F1: JALR prediction"
                );
            }
        }

        trace_fetch!(cpu.trace;
            pc          = %crate::trace::Hex(current_pc),
            paddr       = %crate::trace::Hex(phys_addr),
            compressed  = is_compressed,
            tlb_cycles  = cycles,
            pred_taken,
            pred_target = %crate::trace::Hex(pred_target),
            "F1: fetch entry queued"
        );

        output.push(Fetch1Fetch2Entry {
            pc: current_pc,
            paddr: PhysAddr::new(phys_addr),
            pred_taken,
            pred_target,
            trap: None,
            exception_stage: None,
            ghr_snapshot,
            ras_snapshot,
        });

        current_pc = next_pc_calc;
        if stop_fetch {
            break;
        }
    }

    cpu.pc = current_pc;
}
