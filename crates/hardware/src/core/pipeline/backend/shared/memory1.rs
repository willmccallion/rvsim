//! Memory1 Stage: MMU/TLB address translation.
//!
//! Translates virtual addresses to physical addresses for loads and stores.
//! This stage is the same for both in-order and O3 backends.
//!
//! When the CPU has MSHRs configured (`l1d_mshrs.capacity() > 0`), L1D misses
//! are handled non-blocking: loads are parked in an MSHR and the pipeline
//! continues. When MSHRs are not configured, the original blocking behavior
//! is preserved (full miss penalty added to `complete_cycle`).

use crate::common::{AccessType, ExceptionStage, PhysAddr, TranslationResult, VirtAddr};
use crate::core::Cpu;
use crate::core::pipeline::latches::{ExMem1Entry, Mem1Mem2Entry};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::prf::PhysReg;
use crate::core::pipeline::signals::AtomicOp;
use crate::core::units::cache::mshr::{CacheResponse, MshrWaiter};
use crate::core::units::lsu::unaligned;
use crate::trace_mem;
use crate::trace_trap;

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
            trace_trap!(cpu.trace;
                event   = "propagate",
                stage   = "M1",
                pc      = %crate::trace::Hex(ex.pc),
                rob_tag = ex.rob_tag.0,
                trap    = ?trap,
                "M1: trap propagated through memory1"
            );
            output.push(Mem1Mem2Entry {
                rob_tag: ex.rob_tag,
                pc: ex.pc,
                inst: ex.inst,
                inst_size: ex.inst_size,
                rd: ex.rd,
                rd_phys: ex.rd_phys,
                alu: ex.alu,
                vaddr: VirtAddr::new(ex.alu),
                paddr: PhysAddr::new(0),
                store_data: ex.store_data,
                ctrl: ex.ctrl,
                trap: ex.trap,
                exception_stage: ex.exception_stage,
                fp_flags: ex.fp_flags,
                complete_cycle: current_cycle,
                pte_update: None,
                sfence_vma: ex.sfence_vma,
                vec_mem: ex.vec_mem,
            });
            // Remaining entries go back to input — they'll be flushed when
            // the trap reaches commit, but must not be silently dropped.
            input.extend(iter);
            return cancelled_wakeups;
        }

        let needs_translation = ex.ctrl.mem_read || ex.ctrl.mem_write;

        if needs_translation {
            let mut per_entry_latency: u64 = 0;

            // Check alignment.
            //
            // RISC-V spec Section 8.4: atomic operations (LR/SC/AMO)
            // ALWAYS require natural alignment, even when the
            // implementation supports misaligned regular accesses.
            let size = unaligned::width_to_bytes(ex.ctrl.width);
            let is_atomic = ex.ctrl.atomic_op != AtomicOp::None;
            if !unaligned::is_aligned(ex.alu, size) {
                if cpu.misaligned_access_trap || is_atomic {
                    let trap = if ex.ctrl.mem_write {
                        unaligned::store_misaligned_trap(ex.alu)
                    } else {
                        unaligned::load_misaligned_trap(ex.alu)
                    };
                    trace_trap!(cpu.trace;
                        event     = "misaligned-access",
                        stage     = "M1",
                        pc        = %crate::trace::Hex(ex.pc),
                        rob_tag   = ex.rob_tag.0,
                        vaddr     = %crate::trace::Hex(ex.alu),
                        size      = size,
                        is_write  = ex.ctrl.mem_write,
                        trap      = ?trap,
                        "M1: misaligned memory access trap"
                    );
                    output.push(Mem1Mem2Entry {
                        rob_tag: ex.rob_tag,
                        pc: ex.pc,
                        inst: ex.inst,
                        inst_size: ex.inst_size,
                        rd: ex.rd,
                        rd_phys: ex.rd_phys,
                        alu: ex.alu,
                        vaddr: VirtAddr::new(ex.alu),
                        paddr: PhysAddr::new(0),
                        store_data: ex.store_data,
                        ctrl: ex.ctrl,
                        trap: Some(trap),
                        exception_stage: Some(ExceptionStage::Memory),
                        fp_flags: ex.fp_flags,
                        complete_cycle: current_cycle,
                        pte_update: None,
                        sfence_vma: ex.sfence_vma,
                        vec_mem: ex.vec_mem,
                    });
                    input.extend(iter);
                    return cancelled_wakeups;
                }
                let latency_penalty = unaligned::calculate_unaligned_latency(ex.alu, size, 64);
                per_entry_latency += latency_penalty;
            }

            let access_type = if ex.ctrl.mem_write { AccessType::Write } else { AccessType::Read };

            let TranslationResult { paddr, cycles, trap: fault, pte_update } =
                cpu.translate(VirtAddr::new(ex.alu), access_type, size);
            per_entry_latency += cycles;

            if let Some(t) = fault {
                trace_trap!(cpu.trace;
                    event      = "translation-fault",
                    stage      = "M1",
                    pc         = %crate::trace::Hex(ex.pc),
                    rob_tag    = ex.rob_tag.0,
                    vaddr      = %crate::trace::Hex(ex.alu),
                    trap       = ?t,
                    tlb_cycles = cycles,
                    "M1: address translation fault"
                );
                output.push(Mem1Mem2Entry {
                    rob_tag: ex.rob_tag,
                    pc: ex.pc,
                    inst: ex.inst,
                    inst_size: ex.inst_size,
                    rd: ex.rd,
                    rd_phys: ex.rd_phys,
                    alu: ex.alu,
                    vaddr: VirtAddr::new(ex.alu),
                    paddr: PhysAddr::new(0),
                    store_data: ex.store_data,
                    ctrl: ex.ctrl,
                    trap: Some(t),
                    exception_stage: Some(ExceptionStage::Memory),
                    fp_flags: ex.fp_flags,
                    complete_cycle: current_cycle + per_entry_latency,
                    pte_update: None,
                    sfence_vma: ex.sfence_vma,
                    vec_mem: ex.vec_mem,
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
                && !cpu.bus.bus.is_valid_address(paddr)
            {
                let fault = if ex.ctrl.mem_write {
                    crate::common::Trap::StoreAccessFault(ex.alu)
                } else {
                    crate::common::Trap::LoadAccessFault(ex.alu)
                };
                trace_trap!(cpu.trace;
                    event     = "access-fault",
                    stage     = "M1",
                    pc        = %crate::trace::Hex(ex.pc),
                    rob_tag   = ex.rob_tag.0,
                    vaddr     = %crate::trace::Hex(ex.alu),
                    paddr     = %crate::trace::Hex(paddr.val()),
                    is_write  = ex.ctrl.mem_write,
                    priv_mode = ?cpu.privilege,
                    "M1: unmapped physical address access fault"
                );
                output.push(Mem1Mem2Entry {
                    rob_tag: ex.rob_tag,
                    pc: ex.pc,
                    inst: ex.inst,
                    inst_size: ex.inst_size,
                    rd: ex.rd,
                    rd_phys: ex.rd_phys,
                    alu: ex.alu,
                    vaddr: VirtAddr::new(ex.alu),
                    paddr: PhysAddr::new(0),
                    store_data: ex.store_data,
                    ctrl: ex.ctrl,
                    trap: Some(fault),
                    exception_stage: Some(ExceptionStage::Memory),
                    fp_flags: ex.fp_flags,
                    complete_cycle: current_cycle + per_entry_latency,
                    pte_update: None,
                    sfence_vma: ex.sfence_vma,
                    vec_mem: ex.vec_mem,
                });
                input.extend(iter);
                return cancelled_wakeups;
            }

            trace_mem!(cpu.trace;
                stage            = "M1",
                rob_tag          = ex.rob_tag.0,
                pc               = %crate::trace::Hex(ex.pc),
                op               = if ex.ctrl.mem_write { "store" } else { "load" },
                width            = ?ex.ctrl.width,
                signed           = ex.ctrl.signed_load,
                vaddr            = %crate::trace::Hex(ex.alu),
                paddr            = %crate::trace::Hex(paddr.val()),
                tlb_cycles       = cycles,
                unaligned_penalty = per_entry_latency.saturating_sub(cycles),
                is_mmio          = paddr.val() < cpu.cache_base,
                "M1: address translated"
            );

            // Fill load queue with translated address
            if ex.ctrl.mem_read
                && let Some(ref mut lq) = load_queue
            {
                lq.fill_address(ex.rob_tag, VirtAddr::new(ex.alu), paddr);
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
                    trace_mem!(cpu.trace;
                        stage      = "M1",
                        rob_tag    = ex.rob_tag.0,
                        pc         = %crate::trace::Hex(ex.pc),
                        paddr      = %crate::trace::Hex(paddr.val()),
                        cache_hit  = true,
                        latency    = cpu.l1_d_cache.latency,
                        "M1: L1D cache HIT"
                    );
                    output.push(Mem1Mem2Entry {
                        rob_tag: ex.rob_tag,
                        pc: ex.pc,
                        inst: ex.inst,
                        inst_size: ex.inst_size,
                        rd: ex.rd,
                        rd_phys: ex.rd_phys,
                        alu: ex.alu,
                        vaddr: VirtAddr::new(ex.alu),
                        paddr,
                        store_data: ex.store_data,
                        ctrl: ex.ctrl,
                        trap: None,
                        exception_stage: None,
                        fp_flags: ex.fp_flags,
                        complete_cycle: current_cycle + per_entry_latency,
                        pte_update,
                        sfence_vma: ex.sfence_vma,
                        vec_mem: ex.vec_mem,
                    });
                } else {
                    // L1D miss — compute miss latency from L2/L3/DRAM
                    cpu.stats.dcache_misses += 1;
                    let miss_latency =
                        cpu.l1_d_cache.latency + cpu.simulate_l1d_miss_latency(paddr, access_type);
                    trace_mem!(cpu.trace;
                        stage       = "M1",
                        rob_tag     = ex.rob_tag.0,
                        pc          = %crate::trace::Hex(ex.pc),
                        paddr       = %crate::trace::Hex(paddr.val()),
                        cache_hit   = false,
                        miss_latency,
                        "M1: L1D cache MISS"
                    );

                    let is_atomic = ex.ctrl.atomic_op != AtomicOp::None;
                    let is_store_only = ex.ctrl.mem_write && !ex.ctrl.mem_read && !is_atomic;

                    if is_store_only {
                        // Stores: allocate MSHR for write-allocate but proceed
                        // immediately. The store buffer handles the actual write.
                        let waiter = MshrWaiter { rob_tag: ex.rob_tag, parked_entry: None };
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
                            vaddr: VirtAddr::new(ex.alu),
                            paddr,
                            store_data: ex.store_data,
                            ctrl: ex.ctrl,
                            trap: None,
                            exception_stage: None,
                            fp_flags: ex.fp_flags,
                            complete_cycle: current_cycle + per_entry_latency,
                            pte_update,
                            sfence_vma: ex.sfence_vma,
                            vec_mem: ex.vec_mem,
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
                            vaddr: VirtAddr::new(ex.alu),
                            paddr,
                            store_data: ex.store_data,
                            ctrl: ex.ctrl,
                            trap: None,
                            exception_stage: None,
                            fp_flags: ex.fp_flags,
                            complete_cycle: 0, // set by MSHR completion
                            pte_update,
                            sfence_vma: ex.sfence_vma,
                            vec_mem: ex.vec_mem.clone(),
                        };
                        let waiter = MshrWaiter { rob_tag: ex.rob_tag, parked_entry: Some(parked) };
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
                                trace_mem!(cpu.trace;
                                    stage         = "M1",
                                    rob_tag       = ex.rob_tag.0,
                                    pc            = %crate::trace::Hex(ex.pc),
                                    paddr         = %crate::trace::Hex(paddr.val()),
                                    mshr_action   = "allocated",
                                    rd_phys       = ex.rd_phys.0,
                                    miss_latency,
                                    "M1: MSHR allocated — load parked, speculative wakeup cancelled"
                                );
                                // Load is parked — do not push to output
                            }
                            CacheResponse::MshrCoalesced { .. } => {
                                cpu.stats.mshr_coalesces += 1;
                                // Cancel speculative wakeup — load won't complete at L1D latency
                                cancelled_wakeups.push(ex.rd_phys);
                                cpu.stats.load_replays += 1;
                                trace_mem!(cpu.trace;
                                    stage       = "M1",
                                    rob_tag     = ex.rob_tag.0,
                                    pc          = %crate::trace::Hex(ex.pc),
                                    paddr       = %crate::trace::Hex(paddr.val()),
                                    mshr_action = "coalesced",
                                    rd_phys     = ex.rd_phys.0,
                                    "M1: MSHR coalesced — load parked, speculative wakeup cancelled"
                                );
                                // Load is parked — do not push to output
                            }
                            CacheResponse::MshrFull => {
                                cpu.stats.stalls_mshr_full += 1;
                                trace_mem!(cpu.trace;
                                    stage       = "M1",
                                    rob_tag     = ex.rob_tag.0,
                                    pc          = %crate::trace::Hex(ex.pc),
                                    paddr       = %crate::trace::Hex(paddr.val()),
                                    mshr_action = "full-stall",
                                    "M1: MSHR full — load pushed back, retry next cycle"
                                );
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
                    vaddr: VirtAddr::new(ex.alu),
                    paddr,
                    store_data: ex.store_data,
                    ctrl: ex.ctrl,
                    trap: None,
                    exception_stage: None,
                    fp_flags: ex.fp_flags,
                    complete_cycle: current_cycle + per_entry_latency,
                    pte_update,
                    sfence_vma: ex.sfence_vma,
                    vec_mem: ex.vec_mem,
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
                    vaddr: VirtAddr::new(ex.alu),
                    paddr,
                    store_data: ex.store_data,
                    ctrl: ex.ctrl,
                    trap: None,
                    exception_stage: None,
                    fp_flags: ex.fp_flags,
                    complete_cycle: current_cycle + per_entry_latency,
                    pte_update,
                    sfence_vma: ex.sfence_vma,
                    vec_mem: ex.vec_mem,
                });
            }
        } else {
            // Non-memory instruction: pass through (ready immediately)
            trace_mem!(cpu.trace;
                stage   = "M1",
                rob_tag = ex.rob_tag.0,
                pc      = %crate::trace::Hex(ex.pc),
                op      = "passthrough",
                "M1: non-memory instruction pass-through"
            );
            output.push(Mem1Mem2Entry {
                rob_tag: ex.rob_tag,
                pc: ex.pc,
                inst: ex.inst,
                inst_size: ex.inst_size,
                rd: ex.rd,
                rd_phys: ex.rd_phys,
                alu: ex.alu,
                vaddr: VirtAddr::new(0),
                paddr: PhysAddr::new(0),
                store_data: ex.store_data,
                ctrl: ex.ctrl,
                trap: None,
                exception_stage: None,
                fp_flags: ex.fp_flags,
                complete_cycle: current_cycle,
                pte_update: None,
                sfence_vma: ex.sfence_vma,
                vec_mem: ex.vec_mem,
            });
        }
    }
    cancelled_wakeups
}

#[cfg(test)]
#[allow(unused_results)]
mod tests {
    use super::*;
    use crate::common::{InstSize, RegIdx};
    use crate::config::Config;
    use crate::core::pipeline::signals::ControlSignals;
    use crate::soc::builder::System;

    #[test]
    fn test_memory1_pass_through() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        let mut input = vec![ExMem1Entry {
            rob_tag: crate::core::pipeline::rob::RobTag(1),
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            alu: 42,
            store_data: 0,
            ctrl: ControlSignals::default(), // No mem_read/mem_write
            trap: None,
            exception_stage: None,
            rd_phys: PhysReg(0),
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        }];
        let mut output = Vec::new();

        let cancelled = memory1_stage(&mut cpu, &mut input, &mut output, 10, None);

        assert!(cancelled.is_empty());
        assert_eq!(input.len(), 0);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].paddr, PhysAddr::new(0)); // Pass-through
        assert_eq!(output[0].complete_cycle, 10);
    }

    #[test]
    fn test_memory1_trap_propagation() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        let mut input = vec![ExMem1Entry {
            rob_tag: crate::core::pipeline::rob::RobTag(2),
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            alu: 0,
            store_data: 0,
            ctrl: ControlSignals::default(),
            trap: Some(crate::common::Trap::IllegalInstruction(0)),
            exception_stage: Some(ExceptionStage::Execute),
            rd_phys: PhysReg(0),
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        }];
        let mut output = Vec::new();

        let cancelled = memory1_stage(&mut cpu, &mut input, &mut output, 10, None);

        assert!(cancelled.is_empty());
        assert_eq!(input.len(), 0);
        assert_eq!(output.len(), 1);
        assert!(output[0].trap.is_some());
    }

    #[test]
    fn test_memory1_translation_unmapped_access_fault() {
        let config = Config::default(); // RAM usually starts at 0x8000_0000.
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        cpu.privilege = crate::core::arch::mode::PrivilegeMode::Supervisor; // S-mode traps on unmapped

        // Ensure translation succeeds (direct mode by default), but paddr is invalid
        let ctrl = ControlSignals { mem_read: true, ..Default::default() };

        let mut input = vec![ExMem1Entry {
            rob_tag: crate::core::pipeline::rob::RobTag(3),
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            alu: 0x1000, // Invalid physical address
            store_data: 0,
            ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: PhysReg(0),
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        }];
        let mut output = Vec::new();

        let cancelled = memory1_stage(&mut cpu, &mut input, &mut output, 10, None);

        assert!(cancelled.is_empty());
        assert_eq!(output.len(), 1);
        assert!(matches!(output[0].trap, Some(crate::common::Trap::LoadAccessFault(_))));
    }

    #[test]
    fn test_memory1_cache_hit_and_miss_with_mshrs() {
        let mut config = Config::default();
        config.cache.l1_d.enabled = true;
        config.cache.l1_d.size_bytes = 4096;
        config.cache.l1_d.mshr_count = 4;
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        let ctrl = ControlSignals { mem_read: true, ..Default::default() };

        // RAM starts at 0x8000_0000
        let mut input = vec![ExMem1Entry {
            rob_tag: crate::core::pipeline::rob::RobTag(4),
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            alu: 0x8000_0000,
            store_data: 0,
            ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: PhysReg(1),
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        }];
        let mut output = Vec::new();

        // 1st load: miss -> allocated in MSHR
        let cancelled = memory1_stage(&mut cpu, &mut input, &mut output, 10, None);
        assert_eq!(cancelled.len(), 1); // Speculative wakeup cancelled
        assert_eq!(output.len(), 0); // Parked in MSHR

        // 2nd load: hit (we must inject it directly into the cache first to simulate hit)
        cpu.l1_d_cache.install_or_replace(0x8000_0000, false, 0);
        let mut input2 = vec![ExMem1Entry {
            rob_tag: crate::core::pipeline::rob::RobTag(5),
            pc: 0x1004,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(2),
            alu: 0x8000_0000,
            store_data: 0,
            ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: PhysReg(2),
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        }];
        let cancelled2 = memory1_stage(&mut cpu, &mut input2, &mut output, 10, None);
        assert_eq!(cancelled2.len(), 0); // Hit, no cancel
        assert_eq!(output.len(), 1); // Proceeds to memory2
        assert_eq!(output[0].paddr, PhysAddr::new(0x8000_0000));
    }

    #[test]
    fn test_memory1_mshr_full_stall() {
        let mut config = Config::default();
        config.cache.l1_d.enabled = true;
        config.cache.l1_d.mshr_count = 1; // Only 1 MSHR
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        let ctrl = ControlSignals { mem_read: true, ..Default::default() };

        let entry1 = ExMem1Entry {
            rob_tag: crate::core::pipeline::rob::RobTag(1),
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            alu: 0x8000_0000,
            store_data: 0,
            ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: PhysReg(1),
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        };
        let entry2 = ExMem1Entry {
            rob_tag: crate::core::pipeline::rob::RobTag(2),
            pc: 0x1004,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(2),
            alu: 0x8000_1000, // Different cache line
            store_data: 0,
            ctrl,
            trap: None,
            exception_stage: None,
            rd_phys: PhysReg(2),
            fp_flags: 0,
            sfence_vma: None,
            vec_mem: None,
        };

        let mut input = vec![entry1, entry2];
        let mut output = Vec::new();

        let cancelled = memory1_stage(&mut cpu, &mut input, &mut output, 10, None);

        // 1st load misses and allocates the only MSHR.
        // 2nd load misses, sees MSHR full, and gets pushed back to input.
        assert_eq!(cancelled.len(), 1);
        assert_eq!(input.len(), 1); // entry2 pushed back
        assert_eq!(input[0].rob_tag.0, 2);
        assert_eq!(output.len(), 0);
    }
}
