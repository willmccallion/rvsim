//! Memory1 Stage: MMU/TLB address translation.
//!
//! Translates virtual addresses to physical addresses for loads and stores.
//! This stage is the same for both in-order and O3 backends.
//!
//! When the CPU has MSHRs configured (`l1d_mshrs.capacity() > 0`), L1D misses
//! are handled non-blocking: loads are parked in an MSHR and the pipeline
//! continues. When MSHRs are not configured, the original blocking behavior
//! is preserved (full miss penalty added to `complete_cycle`).

use crate::common::{AccessType, ExceptionStage, TranslationResult, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::signals::AtomicOp;
use crate::core::units::cache::mshr::{CacheResponse, MshrWaiter};
use crate::core::units::lsu::unaligned;

/// Executes the Memory1 stage: address translation + cache probe.
///
/// `current_cycle` is the current simulation cycle. Each output entry's
/// `complete_cycle` is set to `current_cycle + per_entry_latency`, allowing
/// the O3 backend to track per-operation latency instead of a single global
/// stall counter.
///
/// When MSHRs are available, L1D misses for loads/atomics are parked in MSHRs
/// (their `Mem1Mem2Entry` is stored inside the MSHR waiter). Stores that miss
/// L1D allocate an MSHR for write-allocate but proceed immediately. Entries
/// that cannot be processed (MSHR full) are pushed back into `input` for retry.
/// Returns a list of physical registers whose speculative wakeup should be
/// cancelled (loads that missed L1D and were parked in MSHRs).
pub fn memory1_stage(
    cpu: &mut Cpu,
    input: &mut Vec<ExMem1Entry>,
    output: &mut Vec<Mem1Mem2Entry>,
    current_cycle: u64,
    mut load_queue: Option<&mut LoadQueue>,
) -> Vec<PhysReg> {
    let entries = std::mem::take(input);
    let has_mshrs = cpu.l1d_mshrs.capacity() > 0;
    let mut cancelled_wakeups: Vec<PhysReg> = Vec::new();
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
            return cancelled_wakeups;
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
                return cancelled_wakeups;
            }

            // Check that the physical address is backed by a device.
            // Unmapped regions generate access faults for S/U-mode (Linux
            // device probing depends on this). M-mode firmware (OpenSBI)
            // probes addresses expecting bus default (0), not faults.
            if cpu.privilege != crate::core::arch::mode::PrivilegeMode::Machine
                && !cpu.bus.bus.is_valid_address(paddr.val())
            {
                let fault = if ex.ctrl.mem_write {
                    crate::common::Trap::StoreAccessFault(ex.alu)
                } else {
                    crate::common::Trap::LoadAccessFault(ex.alu)
                };
                if cpu.trace {
                    eprintln!(
                        "M1  pc={:#x} # ACCESS FAULT (unmapped): paddr={:#x}",
                        ex.pc,
                        paddr.val()
                    );
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
                    trap: Some(fault),
                    exception_stage: Some(ExceptionStage::Memory),
                    fp_flags: ex.fp_flags,
                    complete_cycle: current_cycle + per_entry_latency,
                });
                input.extend(iter);
                return cancelled_wakeups;
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
            if ex.ctrl.mem_read
                && let Some(ref mut lq) = load_queue
            {
                lq.fill_address(ex.rob_tag, ex.alu, paddr.val());
            }

            // D-cache/bus latency: only cacheable addresses (RAM) go through
            // the cache hierarchy. MMIO addresses (below cache_base) bypass
            // caches entirely — they are uncacheable by nature.
            if paddr.val() >= cpu.cache_base && has_mshrs {
                // ── Non-blocking path (MSHRs available) ──
                let is_write = ex.ctrl.mem_write;
                let l1d_hit = cpu.l1_d_cache.access_check(paddr.val(), is_write);

                if l1d_hit {
                    cpu.stats.dcache_hits += 1;
                    per_entry_latency += cpu.l1_d_cache.latency;
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
                    // L1D miss — compute miss latency from L2/L3/DRAM
                    cpu.stats.dcache_misses += 1;
                    let miss_latency =
                        cpu.l1_d_cache.latency + cpu.simulate_l1d_miss_latency(paddr, access_type);

                    let is_atomic = ex.ctrl.atomic_op != AtomicOp::None;
                    let is_store_only = ex.ctrl.mem_write && !ex.ctrl.mem_read && !is_atomic;

                    if is_store_only {
                        // Stores: allocate MSHR for write-allocate but proceed
                        // immediately. The store buffer handles the actual write.
                        let waiter = MshrWaiter {
                            rob_tag: ex.rob_tag,
                            parked_entry: None,
                        };
                        let resp = cpu.l1d_mshrs.request(
                            paddr.val(),
                            true,
                            miss_latency,
                            current_cycle,
                            waiter,
                        );
                        match resp {
                            CacheResponse::MshrAllocated { .. } => {
                                cpu.stats.mshr_allocations += 1;
                            }
                            CacheResponse::MshrCoalesced { .. } => {
                                cpu.stats.mshr_coalesces += 1;
                            }
                            CacheResponse::MshrFull | CacheResponse::Hit => {
                                // Store can proceed anyway — worst case the
                                // write-allocate just doesn't happen.
                            }
                        }
                        // Store proceeds with just L1D tag-check latency
                        per_entry_latency += cpu.l1_d_cache.latency;
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
                        // Loads and atomics: park in MSHR
                        let parked = Mem1Mem2Entry {
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
                            complete_cycle: 0, // set by MSHR completion
                        };
                        let waiter = MshrWaiter {
                            rob_tag: ex.rob_tag,
                            parked_entry: Some(parked),
                        };
                        let resp = cpu.l1d_mshrs.request(
                            paddr.val(),
                            is_write,
                            miss_latency,
                            current_cycle,
                            waiter,
                        );
                        match resp {
                            CacheResponse::MshrAllocated { .. } => {
                                cpu.stats.mshr_allocations += 1;
                                // Cancel speculative wakeup — load won't complete at L1D latency
                                cancelled_wakeups.push(ex.rd_phys);
                                cpu.stats.load_replays += 1;
                                if cpu.trace {
                                    eprintln!(
                                        "M1  pc={:#x} MSHR allocated for paddr={:#x}",
                                        ex.pc,
                                        paddr.val()
                                    );
                                }
                                // Load is parked — do not push to output
                            }
                            CacheResponse::MshrCoalesced { .. } => {
                                cpu.stats.mshr_coalesces += 1;
                                // Cancel speculative wakeup — load won't complete at L1D latency
                                cancelled_wakeups.push(ex.rd_phys);
                                cpu.stats.load_replays += 1;
                                if cpu.trace {
                                    eprintln!(
                                        "M1  pc={:#x} MSHR coalesced for paddr={:#x}",
                                        ex.pc,
                                        paddr.val()
                                    );
                                }
                                // Load is parked — do not push to output
                            }
                            CacheResponse::MshrFull => {
                                cpu.stats.stalls_mshr_full += 1;
                                // Push back to input for retry next cycle
                                input.push(ex);
                                input.extend(iter);
                                return cancelled_wakeups;
                            }
                            CacheResponse::Hit => unreachable!(),
                        }
                    }
                }
            } else if paddr.val() >= cpu.cache_base {
                // ── Blocking path (no MSHRs) ──
                let lat = cpu.simulate_memory_access(paddr, access_type);
                per_entry_latency += lat;
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
                // MMIO: bypass caches entirely
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
            }
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
    cancelled_wakeups
}
