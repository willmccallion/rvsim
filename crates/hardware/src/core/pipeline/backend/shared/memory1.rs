//! Memory1 Stage: MMU/TLB address translation.
//!
//! Translates virtual addresses to physical addresses for loads and stores.
//! This stage is the same for both in-order and O3 backends.

use crate::common::{AccessType, ExceptionStage, TranslationResult, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::units::lsu::unaligned;

/// Executes the Memory1 stage: address translation.
///
/// `current_cycle` is the current simulation cycle. Each output entry's
/// `complete_cycle` is set to `current_cycle + per_entry_latency`, allowing
/// the O3 backend to track per-operation latency instead of a single global
/// stall counter.
pub fn memory1_stage(
    cpu: &mut Cpu,
    input: &mut Vec<ExMem1Entry>,
    output: &mut Vec<Mem1Mem2Entry>,
    current_cycle: u64,
    mut load_queue: Option<&mut LoadQueue>,
) {
    let entries = std::mem::take(input);
    // Do NOT clear output — memory2 may have pushed stalled entries back
    // into this latch. We append new entries after any stalled ones.

    let mut iter = entries.into_iter();

    while let Some(ex) = iter.next() {
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
                rd_phys: ex.rd_phys,
                alu: ex.alu,
                vaddr: ex.alu,
                paddr: 0,
                store_data: ex.store_data,
                ctrl: ex.ctrl,
                trap: ex.trap,
                exception_stage: ex.exception_stage,
                fp_flags: ex.fp_flags,
                complete_cycle: current_cycle,
            });
            // Remaining entries go back to input — they'll be flushed when
            // the trap reaches commit, but must not be silently dropped.
            input.extend(iter);
            return;
        }

        let needs_translation = ex.ctrl.mem_read || ex.ctrl.mem_write;

        if needs_translation {
            let mut per_entry_latency: u64 = 0;

            // Check alignment
            let size = unaligned::width_to_bytes(ex.ctrl.width);
            if !unaligned::is_aligned(ex.alu, size) {
                let latency_penalty = unaligned::calculate_unaligned_latency(ex.alu, size, 64);
                per_entry_latency += latency_penalty;
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
            per_entry_latency += cycles;

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
                    rd_phys: ex.rd_phys,
                    alu: ex.alu,
                    vaddr: ex.alu,
                    paddr: 0,
                    store_data: ex.store_data,
                    ctrl: ex.ctrl,
                    trap: Some(t),
                    exception_stage: Some(ExceptionStage::Memory),
                    fp_flags: ex.fp_flags,
                    complete_cycle: current_cycle + per_entry_latency,
                });
                // Remaining entries go back to input.
                input.extend(iter);
                return;
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

            // Fill load queue with translated address
            if ex.ctrl.mem_read {
                if let Some(ref mut lq) = load_queue {
                    lq.fill_address(ex.rob_tag, ex.alu, paddr.val());
                }
            }

            // D-cache/bus latency: only cacheable addresses (RAM) go through
            // the cache hierarchy. MMIO addresses (below cache_base) bypass
            // caches entirely — they are uncacheable by nature.
            if paddr.val() >= cpu.cache_base {
                let lat = cpu.simulate_memory_access(paddr, access_type);
                per_entry_latency += lat;
            }

            output.push(Mem1Mem2Entry {
                rob_tag: ex.rob_tag,
                pc: ex.pc,
                inst: ex.inst,
                inst_size: ex.inst_size,
                rd: ex.rd,
                rd_phys: ex.rd_phys,
                alu: ex.alu,
                vaddr: ex.alu,
                paddr: paddr.val(),
                store_data: ex.store_data,
                ctrl: ex.ctrl,
                trap: None,
                exception_stage: None,
                fp_flags: ex.fp_flags,
                complete_cycle: current_cycle + per_entry_latency,
            });
        } else {
            // Non-memory instruction: pass through (ready immediately)
            if cpu.trace {
                eprintln!("M1  pc={:#x} (pass-through)", ex.pc);
            }
            output.push(Mem1Mem2Entry {
                rob_tag: ex.rob_tag,
                pc: ex.pc,
                inst: ex.inst,
                inst_size: ex.inst_size,
                rd: ex.rd,
                rd_phys: ex.rd_phys,
                alu: ex.alu,
                vaddr: 0,
                paddr: 0,
                store_data: ex.store_data,
                ctrl: ex.ctrl,
                trap: None,
                exception_stage: None,
                fp_flags: ex.fp_flags,
                complete_cycle: current_cycle,
            });
        }
    }
}
