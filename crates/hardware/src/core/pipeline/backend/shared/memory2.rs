//! Memory2 stage: data-path finalization for memory operations.
//!
//! In the event-driven design, memory1 emits the `MemReq` packet and parks
//! loads / atomics until their response arrives. The mailbox-drain stage at
//! the top of [`Pipeline::tick`](crate::core::pipeline::engine::Pipeline::tick)
//! pulls each completed entry into the Memory1→Memory2 latch with
//! `load_data` populated from RAM (fast path) or the device's `MemResp`
//! payload.
//!
//! Memory2 owns the value-side bookkeeping that's independent of where the
//! bytes came from:
//!
//! - **Loads:** sign / zero extend `load_data`, apply FP NaN-boxing.
//!   Store-buffer forwarding has already happened at memory1 (the
//!   `sb_forwarded` flag short-circuits any second-look here).
//! - **Stores:** resolve the store buffer with `(paddr, store_data)` and
//!   ask the load queue whether a younger load has already executed with
//!   stale data; surface the oldest violation back to the caller for
//!   pipeline flush.
//! - **SC:** optimistic store buffer resolve, set the deferred
//!   `LrScRecord::Sc` so commit can verify the reservation.
//! - **LR:** record `LrScRecord::Lr` so commit installs the reservation.
//! - **AMO:** combine `load_data` (the old value) with `store_data`
//!   (the register operand) through the AMO ALU and resolve the store
//!   buffer with the new value; the old value is what the destination
//!   register receives.
//! - **Non-memory ops:** pass through untouched.

use crate::common::error::LrScRecord;
use crate::core::pipeline::latches::{Mem1Mem2Entry, Mem2WbEntry};
use crate::core::pipeline::load_queue::LoadQueue;
use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::{AtomicOp, MemWidth};
use crate::core::pipeline::store_buffer::StoreBuffer;
use crate::core::units::lsu::Lsu;
use crate::sim::StageCtx;
use crate::trace_fwd;
use crate::trace_mem;
use crate::trace_trap;

/// Executes the Memory2 stage.
///
/// Returns the oldest memory-ordering violation observed this cycle (older
/// `RobTag`, lower index). The caller flushes the pipeline at that tag.
pub fn memory2_stage(
    state: &mut StageCtx<'_>,
    input: &mut Vec<Mem1Mem2Entry>,
    output: &mut Vec<Mem2WbEntry>,
    store_buffer: &mut StoreBuffer,
    mut load_queue: Option<&mut LoadQueue>,
    mut vec_store_buffer: Option<&mut crate::core::pipeline::vec_store_buffer::VecStoreBuffer>,
) -> Option<(RobTag, u64)> {
    let mut violation: Option<(RobTag, u64)> = None;
    let mut entries = std::mem::take(input);

    // Stores and younger loads need to be visible to each other in program
    // order so the SB resolve in this cycle still flags an ordering violation
    // against a load that already executed.
    entries.sort_by_key(|e| e.rob_tag.0);

    output.clear();

    for mem in entries {
        if let Some(ref trap) = mem.trap {
            trace_trap!(state.trace_trap_enabled(trap);
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
                rd_phys: mem.rd_phys,
                alu: mem.alu,
                load_data: 0,
                ctrl: mem.ctrl,
                trap: mem.trap,
                exception_stage: mem.exception_stage,
                fp_flags: mem.fp_flags,
                pte_update: mem.pte_update,
                sfence_vma: mem.sfence_vma,
                lr_sc: None,
                vec_mem: mem.vec_mem,
                observed: mem.observed,
            });
            continue;
        }

        let mut load_data = 0u64;
        let mut lr_sc: Option<LrScRecord> = None;

        if mem.ctrl.atomic_op != AtomicOp::None {
            // Atomic operation: load_data (from memory1's MemReq) holds the
            // current value. Resolve / record / RMW based on op kind.
            match mem.ctrl.atomic_op {
                AtomicOp::Lr => {
                    load_data = sign_extend(mem.load_data, mem.ctrl.width, mem.ctrl.signed_load);
                    lr_sc = Some(LrScRecord::Lr { paddr: mem.paddr });
                }
                AtomicOp::Sc => {
                    // SC was resolved optimistically at memory1; commit
                    // verifies. The returned value is 0 (success).
                    store_buffer.resolve(mem.rob_tag, mem.vaddr, mem.paddr, mem.store_data);
                    lr_sc = Some(LrScRecord::Sc { paddr: mem.paddr });
                    if let Some(ref lq) = load_queue
                        && let Some(violating_tag) =
                            lq.check_ordering_violation(mem.paddr, mem.ctrl.width, mem.rob_tag)
                    {
                        merge_violation(&mut violation, (violating_tag, mem.pc));
                    }
                    load_data = 0;
                }
                _ => {
                    // AMO: load_data is the old value; RMW gives new value.
                    let old_val = mem.load_data;
                    let new_val = Lsu::atomic_alu(
                        mem.ctrl.atomic_op,
                        old_val,
                        mem.store_data,
                        mem.ctrl.width,
                    );
                    store_buffer.resolve(mem.rob_tag, mem.vaddr, mem.paddr, new_val);
                    if let Some(ref lq) = load_queue
                        && let Some(violating_tag) =
                            lq.check_ordering_violation(mem.paddr, mem.ctrl.width, mem.rob_tag)
                    {
                        merge_violation(&mut violation, (violating_tag, mem.pc));
                    }
                    // AMO returns the OLD value (sign-extended).
                    load_data = sign_extend(old_val, mem.ctrl.width, mem.ctrl.signed_load);
                }
            }
        } else if mem.ctrl.mem_read {
            // Demand load. Sign / zero extend `load_data` (which memory1 or
            // mailbox-drain populated) and apply FP NaN-boxing.
            load_data = sign_extend(mem.load_data, mem.ctrl.width, mem.ctrl.signed_load);
            if mem.ctrl.fp_reg_write {
                match mem.ctrl.width {
                    MemWidth::Word => load_data |= 0xFFFF_FFFF_0000_0000,
                    MemWidth::Half => {
                        load_data = (load_data & 0xFFFF) | 0xFFFF_FFFF_FFFF_0000;
                    }
                    _ => {}
                }
            }
            if mem.sb_forwarded {
                trace_fwd!(state.config.general.trace_instructions;
                    event         = "forward",
                    load_pc       = %crate::trace::Hex(mem.pc),
                    load_tag      = mem.rob_tag.0,
                    paddr         = %crate::trace::Hex(mem.paddr.val()),
                    width         = ?mem.ctrl.width,
                    signed        = mem.ctrl.signed_load,
                    forwarded_val = %crate::trace::Hex(load_data),
                    "M2: load satisfied from store buffer (memory1 hit)"
                );
            } else {
                trace_mem!(state.config.general.trace_instructions;
                    stage     = "M2",
                    rob_tag   = mem.rob_tag.0,
                    pc        = %crate::trace::Hex(mem.pc),
                    op        = "load",
                    paddr     = %crate::trace::Hex(mem.paddr.val()),
                    width     = ?mem.ctrl.width,
                    load_data = %crate::trace::Hex(load_data),
                    "M2: load value finalized"
                );
            }
            if let Some(ref mut lq) = load_queue {
                let elem = mem.vec_mem.as_ref().map(|v| v.elem_idx);
                lq.fill_data(mem.rob_tag, elem, load_data, mem.observed);
            }
        } else if mem.ctrl.mem_write {
            // Scalar store: resolve the store buffer slot and check the load
            // queue for ordering violations.
            if mem.vec_mem.as_ref().is_some_and(|v| v.is_store) {
                if let Some(vsb) = vec_store_buffer.as_deref_mut() {
                    vsb.resolve_element(mem.rob_tag, mem.paddr, mem.store_data, mem.ctrl.width);
                }
            } else {
                store_buffer.resolve(mem.rob_tag, mem.vaddr, mem.paddr, mem.store_data);
            }
            if let Some(ref lq) = load_queue
                && let Some(violating_tag) =
                    lq.check_ordering_violation(mem.paddr, mem.ctrl.width, mem.rob_tag)
            {
                trace_fwd!(state.config.general.trace_instructions;
                    event           = "violation",
                    store_pc        = %crate::trace::Hex(mem.pc),
                    store_tag       = mem.rob_tag.0,
                    paddr           = %crate::trace::Hex(mem.paddr.val()),
                    width           = ?mem.ctrl.width,
                    violation_flush = violating_tag.0,
                    "M2: memory ordering VIOLATION — younger load executed with stale data"
                );
                merge_violation(&mut violation, (violating_tag, mem.pc));
            }
            trace_mem!(state.config.general.trace_instructions;
                stage      = "M2",
                rob_tag    = mem.rob_tag.0,
                pc         = %crate::trace::Hex(mem.pc),
                op         = "store-resolve",
                paddr      = %crate::trace::Hex(mem.paddr.val()),
                vaddr      = %crate::trace::Hex(mem.vaddr.val()),
                width      = ?mem.ctrl.width,
                store_data = %crate::trace::Hex(mem.store_data),
                "M2: store resolved into store buffer (write deferred to commit)"
            );
        } else {
            trace_mem!(state.config.general.trace_instructions;
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
            load_data,
            ctrl: mem.ctrl,
            trap: None,
            exception_stage: None,
            fp_flags: mem.fp_flags,
            pte_update: mem.pte_update,
            sfence_vma: mem.sfence_vma,
            lr_sc,
            vec_mem: mem.vec_mem,
            observed: mem.observed,
        });
    }

    violation
}

const fn merge_violation(slot: &mut Option<(RobTag, u64)>, new: (RobTag, u64)) {
    match slot {
        None => *slot = Some(new),
        Some((existing, _)) if new.0.is_older_than(*existing) => *slot = Some(new),
        _ => {}
    }
}

/// Sign / zero-extends a raw load value according to the access width and
/// the signed-load control bit.
pub(crate) const fn sign_extend(raw: u64, width: MemWidth, signed: bool) -> u64 {
    if signed {
        match width {
            MemWidth::Byte => (raw as u8 as i8) as i64 as u64,
            MemWidth::Half => (raw as u16 as i16) as i64 as u64,
            MemWidth::Word => (raw as u32 as i32) as i64 as u64,
            MemWidth::Double => raw,
            MemWidth::Nop => 0,
        }
    } else {
        match width {
            MemWidth::Byte => raw & 0xFF,
            MemWidth::Half => raw & 0xFFFF,
            MemWidth::Word => raw & 0xFFFF_FFFF,
            MemWidth::Double => raw,
            MemWidth::Nop => 0,
        }
    }
}
