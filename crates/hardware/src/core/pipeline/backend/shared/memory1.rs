//! Memory1 Stage: MMU/TLB address translation.
//!
//! Translates virtual addresses to physical addresses for loads and stores.
//! This stage is the same for both in-order and O3 backends.

use crate::common::{AccessType, ExceptionStage, TranslationResult, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry};
use crate::core::units::lsu::unaligned;

/// Executes the Memory1 stage: address translation.
pub fn memory1_stage(
    cpu: &mut Cpu,
    input: &mut Vec<ExMem1Entry>,
    output: &mut Vec<Mem1Mem2Entry>,
    stall_out: &mut u64,
) {
    let entries = std::mem::take(input);
    // Do NOT clear output — memory2 may have pushed stalled entries back
    // into this latch. We append new entries after any stalled ones.

    let mut flush_remaining = false;

    for ex in entries {
        if flush_remaining {
            break;
        }

        // Propagate traps
        if let Some(ref trap) = ex.trap {
            if cpu.trace {
                eprintln!("M1  pc={:#x} # TRAP: {:?}", ex.pc, trap);
            }
            output.push(Mem1Mem2Entry {
                rob_tag: ex.rob_tag,
                pc: ex.pc,
                inst: ex.inst,
                inst_size: ex.inst_size,
                rd: ex.rd,
                alu: ex.alu,
                vaddr: ex.alu,
                paddr: 0,
                store_data: ex.store_data,
                ctrl: ex.ctrl,
                trap: ex.trap,
                exception_stage: ex.exception_stage,
            });
            flush_remaining = true;
            continue;
        }

        let needs_translation = ex.ctrl.mem_read || ex.ctrl.mem_write;

        if needs_translation {
            // Check alignment
            let size = unaligned::width_to_bytes(ex.ctrl.width);
            if !unaligned::is_aligned(ex.alu, size) {
                let latency_penalty = unaligned::calculate_unaligned_latency(ex.alu, size, 64);
                *stall_out += latency_penalty;
            }

            let access_type = if ex.ctrl.mem_write {
                AccessType::Write
            } else {
                AccessType::Read
            };

            let TranslationResult {
                paddr,
                cycles,
                trap: fault,
            } = cpu.translate(VirtAddr::new(ex.alu), access_type);
            *stall_out += cycles;

            if let Some(t) = fault {
                if cpu.trace {
                    eprintln!("M1  pc={:#x} # TRAP: {:?} (addr={:#x})", ex.pc, t, ex.alu);
                }
                output.push(Mem1Mem2Entry {
                    rob_tag: ex.rob_tag,
                    pc: ex.pc,
                    inst: ex.inst,
                    inst_size: ex.inst_size,
                    rd: ex.rd,
                    alu: ex.alu,
                    vaddr: ex.alu,
                    paddr: 0,
                    store_data: ex.store_data,
                    ctrl: ex.ctrl,
                    trap: Some(t),
                    exception_stage: Some(ExceptionStage::Memory),
                });
                flush_remaining = true;
                continue;
            }

            if cpu.trace {
                if ex.ctrl.mem_read {
                    eprintln!(
                        "M1  pc={:#x} LOAD vaddr={:#x} paddr={:#x}",
                        ex.pc,
                        ex.alu,
                        paddr.val()
                    );
                } else if ex.ctrl.mem_write {
                    eprintln!(
                        "M1  pc={:#x} STORE vaddr={:#x} paddr={:#x}",
                        ex.pc,
                        ex.alu,
                        paddr.val()
                    );
                }
            }

            // D-cache/bus latency for RAM and MMIO
            if paddr.val() >= cpu.mmio_base {
                let lat = cpu.simulate_memory_access(paddr, access_type);
                *stall_out += lat;
            } else if ex.ctrl.mem_write {
                let addr = paddr.val();
                if (0x10001000..0x10002000).contains(&addr) {
                    cpu.l1_d_cache.flush();
                    cpu.l2_cache.flush();
                    cpu.l3_cache.flush();
                }
            }

            output.push(Mem1Mem2Entry {
                rob_tag: ex.rob_tag,
                pc: ex.pc,
                inst: ex.inst,
                inst_size: ex.inst_size,
                rd: ex.rd,
                alu: ex.alu,
                vaddr: ex.alu,
                paddr: paddr.val(),
                store_data: ex.store_data,
                ctrl: ex.ctrl,
                trap: None,
                exception_stage: None,
            });
        } else {
            // Non-memory instruction: pass through
            if cpu.trace {
                eprintln!("M1  pc={:#x} (pass-through)", ex.pc);
            }
            output.push(Mem1Mem2Entry {
                rob_tag: ex.rob_tag,
                pc: ex.pc,
                inst: ex.inst,
                inst_size: ex.inst_size,
                rd: ex.rd,
                alu: ex.alu,
                vaddr: 0,
                paddr: 0,
                store_data: ex.store_data,
                ctrl: ex.ctrl,
                trap: None,
                exception_stage: None,
            });
        }
    }
}
