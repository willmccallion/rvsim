//! Memory2 Stage: D-cache access for loads, store buffer resolution.
//!
//! For loads: read data from the cache/memory (with store-to-load forwarding).
//! For stores: resolve the store buffer entry with paddr + data (NO memory write).
//! This stage is the same for both in-order and O3 backends.

use crate::common::error::{ExceptionStage, LrScRecord, Trap};
use crate::core::Cpu;
use crate::core::pipeline::latches::{Mem1Mem2Entry, Mem2WbEntry};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::rob::{Rob, RobTag};
use crate::core::pipeline::signals::{AtomicOp, MemWidth};
use crate::core::pipeline::store_buffer::{ForwardResult, StoreBuffer};
use crate::core::units::lsu::Lsu;
use crate::trace_fwd;
use crate::trace_mem;
use crate::trace_trap;

/// Executes the Memory2 stage: D-cache access + store buffer forwarding.
///
/// Returns `Some(violating_rob_tag)` if a memory ordering violation is detected
/// (a store resolved its address and overlapped with a younger already-executed load).
/// The caller should flush from this tag onward.
pub fn memory2_stage(
    cpu: &mut Cpu,
    input: &mut Vec<Mem1Mem2Entry>,
    output: &mut Vec<Mem2WbEntry>,
    store_buffer: &mut StoreBuffer,
    _rob: &mut Rob,
    mut load_queue: Option<&mut LoadQueue>,
) -> Option<RobTag> {
    let mut violation: Option<RobTag> = None;
    let mut entries = std::mem::take(input);

    // Sort by rob_tag to ensure entries are processed in program order.
    // This prevents younger loads from stalling on unresolved older stores
    // that are behind them in the latch, which would deadlock the pipeline.
    entries.sort_by_key(|e| e.rob_tag.0);

    output.clear();

    let mut iter = entries.into_iter();

    while let Some(mem) = iter.next() {
        // Propagate traps
        if let Some(ref trap) = mem.trap {
            trace_trap!(cpu.trace;
                event   = "propagate",
                stage   = "M2",
                pc      = %crate::trace::Hex(mem.pc),
                rob_tag = mem.rob_tag.0,
                trap    = ?trap,
                "M2: trap propagated through memory2"
            );
            output.push(Mem2WbEntry {
                rob_tag: mem.rob_tag,
                pc: mem.pc,
                inst: mem.inst,
                inst_size: mem.inst_size,
                rd: mem.rd,
                alu: mem.alu,
                load_data: 0,
                ctrl: mem.ctrl,
                trap: mem.trap,
                exception_stage: mem.exception_stage,
                rd_phys: mem.rd_phys,
                fp_flags: mem.fp_flags,
                pte_update: mem.pte_update,
                sfence_vma: mem.sfence_vma,
                lr_sc: None,
            });
            // Trap: remaining entries stay in the input latch for next cycle.
            // They will be flushed by the commit-stage trap handler, but must
            // not be silently dropped here or their ROB entries become orphans.
            input.extend(iter);
            return None;
        }

        let raw_paddr = mem.paddr;
        let is_ram = raw_paddr.val() >= cpu.ram_start && raw_paddr.val() < cpu.ram_end;
        let ram_offset = if is_ram { (raw_paddr.val() - cpu.ram_start) as usize } else { 0 };

        let mut ld: u64 = 0;
        let trap: Option<Trap> = None;
        let exception_stage: Option<ExceptionStage> = None;
        let mut lr_sc: Option<LrScRecord> = None;

        if mem.ctrl.atomic_op != AtomicOp::None {
            // Atomic operations
            match mem.ctrl.atomic_op {
                AtomicOp::Lr => {
                    // LR must read the globally-visible value, not a
                    // locally-speculative one.  Stall until all older stores
                    // to this address have drained.
                    if store_buffer.has_older_store_to(raw_paddr, mem.ctrl.width, mem.rob_tag) {
                        input.push(mem);
                        input.extend(iter);
                        return None;
                    }
                    ld = match mem.ctrl.width {
                        MemWidth::Word => (cpu.bus.bus.read_u32(raw_paddr) as i32) as i64 as u64,
                        MemWidth::Double => cpu.bus.bus.read_u64(raw_paddr),
                        _ => 0,
                    };
                    // Defer reservation to commit — speculative LR must not
                    // modify architectural reservation state.
                    lr_sc = Some(LrScRecord::Lr { paddr: raw_paddr });
                }
                AtomicOp::Sc => {
                    // Optimistically assume SC succeeds: resolve the store
                    // buffer and return 0 (success).  The commit stage will
                    // verify the reservation and, if invalid, cancel the
                    // store and flush the pipeline.
                    store_buffer.resolve(mem.rob_tag, mem.vaddr, raw_paddr, mem.store_data);
                    ld = 0; // optimistic success
                    lr_sc = Some(LrScRecord::Sc { paddr: raw_paddr });
                }
                _ => {
                    // AMO: atomic read-modify-write must operate on the
                    // globally-visible value.  Stall until all older stores
                    // to this address have drained, then read from memory.
                    if store_buffer.has_older_store_to(raw_paddr, mem.ctrl.width, mem.rob_tag) {
                        input.push(mem);
                        input.extend(iter);
                        return None;
                    }
                    let old_val = match mem.ctrl.width {
                        MemWidth::Word => (cpu.bus.bus.read_u32(raw_paddr) as i32) as i64 as u64,
                        MemWidth::Double => cpu.bus.bus.read_u64(raw_paddr),
                        _ => 0,
                    };

                    let new_val = Lsu::atomic_alu(
                        mem.ctrl.atomic_op,
                        old_val,
                        mem.store_data,
                        mem.ctrl.width,
                    );

                    // Resolve store buffer with the computed new value
                    store_buffer.resolve(mem.rob_tag, mem.vaddr, raw_paddr, new_val);

                    ld = old_val;
                    // Note: AMOs may optionally clear the reservation per
                    // spec.  We skip this here because Memory2 is speculative
                    // — clearing on squash would corrupt LR/SC pairs.
                    // The reservation will be cleared by the next SC commit.
                }
            }
        } else if mem.ctrl.mem_read {
            // Check store buffer for forwarding first
            match store_buffer.forward_load(raw_paddr, mem.ctrl.width, mem.rob_tag) {
                ForwardResult::Hit(forwarded) => {
                    // Apply sign extension for signed loads (LB, LH, LW on RV64).
                    // The store buffer returns raw masked data without sign extension.
                    ld = if mem.ctrl.signed_load {
                        match mem.ctrl.width {
                            MemWidth::Byte => (forwarded as u8 as i8) as i64 as u64,
                            MemWidth::Half => (forwarded as u16 as i16) as i64 as u64,
                            MemWidth::Word => (forwarded as u32 as i32) as i64 as u64,
                            _ => forwarded,
                        }
                    } else {
                        forwarded
                    };
                    // NaN-boxing for FP loads forwarded from store buffer
                    if mem.ctrl.fp_reg_write && matches!(mem.ctrl.width, MemWidth::Word) {
                        ld |= 0xFFFF_FFFF_0000_0000;
                    }

                    trace_fwd!(cpu.trace;
                        event           = "forward",
                        load_pc         = %crate::trace::Hex(mem.pc),
                        load_tag        = mem.rob_tag.0,
                        paddr           = %crate::trace::Hex(raw_paddr.val()),
                        width           = ?mem.ctrl.width,
                        signed          = mem.ctrl.signed_load,
                        forwarded_val   = %crate::trace::Hex(ld),
                        "M2: store-to-load forwarding HIT"
                    );
                    trace_mem!(cpu.trace;
                        stage       = "M2",
                        rob_tag     = mem.rob_tag.0,
                        pc          = %crate::trace::Hex(mem.pc),
                        op          = "load",
                        paddr       = %crate::trace::Hex(raw_paddr.val()),
                        width       = ?mem.ctrl.width,
                        forwarded   = true,
                        load_data   = %crate::trace::Hex(ld),
                        "M2: load satisfied from store buffer"
                    );
                }
                ForwardResult::Stall => {
                    // Partial overlap — push back current + remaining entries
                    trace_fwd!(cpu.trace;
                        event           = "stall",
                        load_pc         = %crate::trace::Hex(mem.pc),
                        load_tag        = mem.rob_tag.0,
                        paddr           = %crate::trace::Hex(raw_paddr.val()),
                        width           = ?mem.ctrl.width,
                        partial_overlap = true,
                        "M2: store-to-load forwarding STALL (partial overlap)"
                    );
                    input.push(mem);
                    input.extend(iter);
                    return None;
                }
                ForwardResult::Miss => {
                    // Read from memory/cache
                    trace_fwd!(cpu.trace;
                        event   = "miss",
                        load_pc = %crate::trace::Hex(mem.pc),
                        load_tag = mem.rob_tag.0,
                        paddr   = %crate::trace::Hex(raw_paddr.val()),
                        width   = ?mem.ctrl.width,
                        is_ram,
                        "M2: store buffer miss — reading from memory"
                    );
                    ld = if is_ram {
                        unsafe {
                            match (mem.ctrl.width, mem.ctrl.signed_load) {
                                (MemWidth::Byte, true) => {
                                    (*cpu.ram_ptr.add(ram_offset) as i8) as i64 as u64
                                }
                                (MemWidth::Half, true) => {
                                    ((cpu.ram_ptr.add(ram_offset) as *const u16).read_unaligned()
                                        as i16) as i64 as u64
                                }
                                (MemWidth::Word, true) => {
                                    ((cpu.ram_ptr.add(ram_offset) as *const u32).read_unaligned()
                                        as i32) as i64 as u64
                                }
                                (MemWidth::Byte, false) => *cpu.ram_ptr.add(ram_offset) as u64,
                                (MemWidth::Half, false) => {
                                    (cpu.ram_ptr.add(ram_offset) as *const u16).read_unaligned()
                                        as u64
                                }
                                (MemWidth::Word, false) => {
                                    (cpu.ram_ptr.add(ram_offset) as *const u32).read_unaligned()
                                        as u64
                                }
                                (MemWidth::Double, _) => {
                                    (cpu.ram_ptr.add(ram_offset) as *const u64).read_unaligned()
                                }
                                _ => 0,
                            }
                        }
                    } else {
                        match (mem.ctrl.width, mem.ctrl.signed_load) {
                            (MemWidth::Byte, true) => {
                                (cpu.bus.bus.read_u8(raw_paddr) as i8) as i64 as u64
                            }
                            (MemWidth::Half, true) => {
                                (cpu.bus.bus.read_u16(raw_paddr) as i16) as i64 as u64
                            }
                            (MemWidth::Word, true) => {
                                (cpu.bus.bus.read_u32(raw_paddr) as i32) as i64 as u64
                            }
                            (MemWidth::Byte, false) => cpu.bus.bus.read_u8(raw_paddr) as u64,
                            (MemWidth::Half, false) => cpu.bus.bus.read_u16(raw_paddr) as u64,
                            (MemWidth::Word, false) => cpu.bus.bus.read_u32(raw_paddr) as u64,
                            (MemWidth::Double, _) => cpu.bus.bus.read_u64(raw_paddr),
                            _ => 0,
                        }
                    };

                    // NaN-boxing for FP loads
                    if mem.ctrl.fp_reg_write && matches!(mem.ctrl.width, MemWidth::Word) {
                        ld |= 0xFFFF_FFFF_0000_0000;
                    }
                }
            }

            // Fill load queue with completed data
            if let Some(ref mut lq) = load_queue {
                lq.fill_data(mem.rob_tag, ld);
            }

            trace_mem!(cpu.trace;
                stage     = "M2",
                rob_tag   = mem.rob_tag.0,
                pc        = %crate::trace::Hex(mem.pc),
                op        = "load",
                paddr     = %crate::trace::Hex(raw_paddr.val()),
                width     = ?mem.ctrl.width,
                forwarded = false,
                load_data = %crate::trace::Hex(ld),
                is_ram,
                "M2: load complete"
            );
        } else if mem.ctrl.mem_write {
            // Stores: resolve store buffer with paddr + data, NO memory write
            store_buffer.resolve(mem.rob_tag, mem.vaddr, raw_paddr, mem.store_data);

            // Check for memory ordering violation: did a younger load already
            // execute with stale data at this address?
            if let Some(ref lq) = load_queue
                && let Some(violating_tag) =
                    lq.check_ordering_violation(raw_paddr, mem.ctrl.width, mem.rob_tag)
            {
                trace_fwd!(cpu.trace;
                    event             = "violation",
                    store_pc          = %crate::trace::Hex(mem.pc),
                    store_tag         = mem.rob_tag.0,
                    paddr             = %crate::trace::Hex(raw_paddr.val()),
                    width             = ?mem.ctrl.width,
                    violation_flush   = violating_tag.0,
                    "M2: memory ordering VIOLATION — younger load executed with stale data"
                );
                // Record the oldest violation
                match violation {
                    None => violation = Some(violating_tag),
                    Some(prev) if violating_tag.is_older_than(prev) => {
                        violation = Some(violating_tag);
                    }
                    _ => {}
                }
            }

            // Note: stores to the reservation address may clear the
            // reservation, but this is deferred to commit to avoid
            // corrupting LR/SC state on speculative squash.

            trace_mem!(cpu.trace;
                stage      = "M2",
                rob_tag    = mem.rob_tag.0,
                pc         = %crate::trace::Hex(mem.pc),
                op         = "store-resolve",
                paddr      = %crate::trace::Hex(raw_paddr.val()),
                vaddr      = %crate::trace::Hex(mem.vaddr.val()),
                width      = ?mem.ctrl.width,
                store_data = %crate::trace::Hex(mem.store_data),
                is_ram,
                "M2: store resolved into store buffer (write deferred to commit)"
            );
        } else {
            trace_mem!(cpu.trace;
                stage   = "M2",
                rob_tag = mem.rob_tag.0,
                pc      = %crate::trace::Hex(mem.pc),
                op      = "passthrough",
                "M2: non-memory instruction pass-through"
            );
        }

        output.push(Mem2WbEntry {
            rob_tag: mem.rob_tag,
            pc: mem.pc,
            inst: mem.inst,
            inst_size: mem.inst_size,
            rd: mem.rd,
            rd_phys: mem.rd_phys,
            alu: mem.alu,
            load_data: ld,
            ctrl: mem.ctrl,
            trap: trap.clone(),
            exception_stage,
            fp_flags: mem.fp_flags,
            pte_update: mem.pte_update,
            sfence_vma: mem.sfence_vma,
            lr_sc,
        });

        if trap.is_some() {
            input.extend(iter);
            return None;
        }
    }

    violation
}

#[cfg(test)]
#[allow(unused_results)]
mod tests {
    use super::*;
    use crate::common::{InstSize, PhysAddr, RegIdx, VirtAddr};
    use crate::config::Config;
    use crate::core::pipeline::rob::Rob;
    use crate::core::pipeline::signals::ControlSignals;
    use crate::core::pipeline::store_buffer::StoreBuffer;
    use crate::soc::builder::System;

    #[test]
    fn test_memory2_pass_through() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        let mut store_buffer = StoreBuffer::new(4);
        let mut rob = Rob::new(4);

        let mut input = vec![Mem1Mem2Entry {
            rob_tag: RobTag(1),
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            alu: 42,
            vaddr: VirtAddr::new(0),
            paddr: PhysAddr::new(0),
            store_data: 0,
            ctrl: ControlSignals::default(),
            trap: None,
            exception_stage: None,
            fp_flags: 0,
            complete_cycle: 10,
            pte_update: None,
            sfence_vma: None,
        }];
        let mut output = Vec::new();

        let violation =
            memory2_stage(&mut cpu, &mut input, &mut output, &mut store_buffer, &mut rob, None);

        assert!(violation.is_none());
        assert_eq!(input.len(), 0);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].load_data, 0);
    }

    #[test]
    fn test_memory2_trap_propagation() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        let mut store_buffer = StoreBuffer::new(4);
        let mut rob = Rob::new(4);

        let mut input = vec![Mem1Mem2Entry {
            rob_tag: RobTag(1),
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            alu: 0,
            vaddr: VirtAddr::new(0),
            paddr: PhysAddr::new(0),
            store_data: 0,
            ctrl: ControlSignals::default(),
            trap: Some(crate::common::Trap::IllegalInstruction(0)),
            exception_stage: Some(ExceptionStage::Execute),
            fp_flags: 0,
            complete_cycle: 10,
            pte_update: None,
            sfence_vma: None,
        }];
        let mut output = Vec::new();

        let violation =
            memory2_stage(&mut cpu, &mut input, &mut output, &mut store_buffer, &mut rob, None);

        assert!(violation.is_none());
        assert_eq!(input.len(), 0); // Input is drained because trap is pushed
        assert_eq!(output.len(), 1);
        assert!(output[0].trap.is_some());
    }

    #[test]
    fn test_memory2_atomic_lr_sc_deferred() {
        use crate::common::error::LrScRecord;

        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        let mut store_buffer = StoreBuffer::new(4);
        let mut rob = Rob::new(4);

        let ctrl_lr = ControlSignals {
            atomic_op: crate::core::pipeline::signals::AtomicOp::Lr,
            width: crate::core::pipeline::signals::MemWidth::Word,
            ..Default::default()
        };

        let mut input_lr = vec![Mem1Mem2Entry {
            rob_tag: RobTag(1),
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(1),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            alu: 0,
            vaddr: VirtAddr::new(0x8000_0000),
            paddr: PhysAddr::new(0x8000_0000),
            store_data: 0,
            ctrl: ctrl_lr,
            trap: None,
            exception_stage: None,
            fp_flags: 0,
            complete_cycle: 10,
            pte_update: None,
            sfence_vma: None,
        }];
        let mut output = Vec::new();

        memory2_stage(&mut cpu, &mut input_lr, &mut output, &mut store_buffer, &mut rob, None);
        // LR does NOT set reservation at Memory2 — deferred to commit
        assert!(!cpu.check_reservation(PhysAddr::new(0x8000_0000)));
        // But the output carries the deferred LR record
        assert!(matches!(output[0].lr_sc, Some(LrScRecord::Lr { paddr: PhysAddr(0x8000_0000) })));

        let ctrl_sc = ControlSignals {
            atomic_op: crate::core::pipeline::signals::AtomicOp::Sc,
            width: crate::core::pipeline::signals::MemWidth::Word,
            ..Default::default()
        };
        store_buffer.allocate(RobTag(2), crate::core::pipeline::signals::MemWidth::Word);

        let mut input_sc = vec![Mem1Mem2Entry {
            rob_tag: RobTag(2),
            pc: 0x1004,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(2),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            alu: 0,
            vaddr: VirtAddr::new(0x8000_0000),
            paddr: PhysAddr::new(0x8000_0000),
            store_data: 42,
            ctrl: ctrl_sc,
            trap: None,
            exception_stage: None,
            fp_flags: 0,
            complete_cycle: 10,
            pte_update: None,
            sfence_vma: None,
        }];

        memory2_stage(&mut cpu, &mut input_sc, &mut output, &mut store_buffer, &mut rob, None);
        // SC optimistically returns 0 (success) — actual check deferred to commit
        assert_eq!(output[0].load_data, 0);
        assert!(matches!(output[0].lr_sc, Some(LrScRecord::Sc { paddr: PhysAddr(0x8000_0000) })));
        // Reservation unchanged at Memory2 (still not set — LR was deferred)
        assert!(!cpu.check_reservation(PhysAddr::new(0x8000_0000)));
    }

    #[test]
    fn test_memory2_ordering_violation() {
        let config = Config::default();
        let system = System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);
        let mut store_buffer = StoreBuffer::new(4);
        let mut rob = Rob::new(4);
        let mut load_queue = LoadQueue::new(4);

        // A younger load already executed to the same address
        load_queue.allocate(RobTag(5), crate::core::pipeline::signals::MemWidth::Word);
        load_queue.fill_address(RobTag(5), VirtAddr::new(0x8000_0000), PhysAddr::new(0x8000_0000));
        load_queue.fill_data(RobTag(5), 0);

        let ctrl_store = ControlSignals {
            mem_write: true,
            width: crate::core::pipeline::signals::MemWidth::Word,
            ..Default::default()
        };

        store_buffer.allocate(RobTag(2), crate::core::pipeline::signals::MemWidth::Word);

        let mut input = vec![Mem1Mem2Entry {
            rob_tag: RobTag(2), // Older store
            pc: 0x1000,
            inst: 0,
            inst_size: InstSize::Standard,
            rd: RegIdx::new(0),
            rd_phys: crate::core::pipeline::prf::PhysReg(0),
            alu: 0,
            vaddr: VirtAddr::new(0x8000_0000),
            paddr: PhysAddr::new(0x8000_0000),
            store_data: 42,
            ctrl: ctrl_store,
            trap: None,
            exception_stage: None,
            fp_flags: 0,
            complete_cycle: 10,
            pte_update: None,
            sfence_vma: None,
        }];
        let mut output = Vec::new();

        let violation = memory2_stage(
            &mut cpu,
            &mut input,
            &mut output,
            &mut store_buffer,
            &mut rob,
            Some(&mut load_queue),
        );

        assert_eq!(violation, Some(RobTag(5))); // Older store detected overlap with younger load
    }
}
