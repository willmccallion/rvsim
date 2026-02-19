//! Fetch2 Stage: I-cache access and compressed instruction expansion.
//!
//! This stage reads the instruction bytes from the I-cache (or memory),
//! expands compressed (16-bit) instructions to 32-bit, and produces
//! IfIdEntry results for the decode stage.

use crate::common::constants::{
    COMPRESSED_INSTRUCTION_MASK, COMPRESSED_INSTRUCTION_VALUE, INSTRUCTION_SIZE_16,
    INSTRUCTION_SIZE_32,
};
use crate::common::{AccessType, ExceptionStage, Trap, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::{Fetch1Fetch2Entry, IfIdEntry};
use crate::isa::rvc::expand::expand;

/// Executes the Fetch2 stage: I-cache access + RVC expansion.
///
/// Consumes Fetch1->Fetch2 entries and produces Fetch2->Decode entries.
pub fn fetch2_stage(
    cpu: &mut Cpu,
    input: &mut Vec<Fetch1Fetch2Entry>,
    output: &mut Vec<IfIdEntry>,
    stall_out: &mut u64,
) {
    let entries = std::mem::take(input);
    output.clear();

    for f1 in entries {
        // Propagate traps from Fetch1
        if let Some(ref trap) = f1.trap {
            if cpu.trace {
                eprintln!("F2  pc={:#x} # TRAP: {:?}", f1.pc, trap);
            }
            output.push(IfIdEntry {
                pc: f1.pc,
                inst: 0,
                inst_size: 4,
                pred_taken: f1.pred_taken,
                pred_target: f1.pred_target,
                trap: f1.trap,
                exception_stage: f1.exception_stage,
            });
            break;
        }

        let phys_addr = f1.paddr;

        // Read the first half-word
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

        let (inst, step, inst_trap) = if is_compressed {
            let expanded = expand(half_word);
            if expanded == 0 {
                if output.is_empty() {
                    (
                        0,
                        INSTRUCTION_SIZE_16,
                        Some(Trap::IllegalInstruction(half_word as u32)),
                    )
                } else {
                    break;
                }
            } else {
                (expanded, INSTRUCTION_SIZE_16, None)
            }
        } else {
            let upper_va = f1.pc.wrapping_add(2);
            let crosses_page = (f1.pc >> 12) != (upper_va >> 12);

            let (upper_phys, upper_fault) = if crosses_page {
                let result = cpu.translate(VirtAddr::new(upper_va), AccessType::Fetch);
                *stall_out += result.cycles;
                (result.paddr.val(), result.trap)
            } else {
                (phys_addr + 2, None)
            };

            if let Some(t) = upper_fault {
                (0, INSTRUCTION_SIZE_32, Some(t))
            } else {
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
                (full_inst, INSTRUCTION_SIZE_32, None)
            }
        };

        if let Some(t) = inst_trap {
            if cpu.trace {
                eprintln!("F2  pc={:#x} # TRAP: {:?}", f1.pc, t);
            }
            output.push(IfIdEntry {
                pc: f1.pc,
                inst: 0,
                inst_size: step,
                pred_taken: f1.pred_taken,
                pred_target: f1.pred_target,
                trap: Some(t),
                exception_stage: Some(ExceptionStage::Fetch),
            });
            break;
        }

        // I-cache access latency (only when I-cache is enabled; otherwise the
        // instruction bytes were already read directly from the bus above).
        if cpu.l1_i_cache.enabled {
            *stall_out += cpu
                .simulate_memory_access(crate::common::PhysAddr::new(phys_addr), AccessType::Fetch);
        }

        if cpu.trace {
            eprintln!("F2  pc={:#x} inst={:#010x} (sz={})", f1.pc, inst, step);
        }

        output.push(IfIdEntry {
            pc: f1.pc,
            inst,
            inst_size: step,
            pred_taken: f1.pred_taken,
            pred_target: f1.pred_target,
            trap: None,
            exception_stage: None,
        });
    }
}
