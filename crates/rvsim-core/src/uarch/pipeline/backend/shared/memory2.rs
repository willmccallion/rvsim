//! Memory2 stage: data-path finalization for memory operations.
//!
//! In the event-driven design, memory1 emits the `MemReq` packet and parks
//! loads / atomics until their response arrives. The mailbox-drain stage at
//! the top of [`Pipeline::tick`](crate::uarch::pipeline::engine::Pipeline::tick)
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
//! - **SC / AMO:** the cache has performed it, so `load_data` is the SC's
//!   result or the AMO's old value; free its store-buffer slot and squash
//!   any younger load that already read the location.
//! - **LR:** record `LrScRecord::Lr` so commit installs the reservation.
//! - **Non-memory ops:** pass through untouched.

use crate::arch::reservation::LrScRecord;
use crate::common::PhysAddr;
use crate::exec::memory::load_result;
use crate::isa::op::{AtomicOp, MemWidth};
use crate::trace_fwd;
use crate::trace_mem;
use crate::trace_trap;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::shared::vec_mem::mem_width_from_eew_bytes;
use crate::uarch::pipeline::latches::{Mem1Mem2Entry, Mem2WbEntry, VecMemTarget};
use crate::uarch::pipeline::lsq::load_queue::LoadQueue;
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::rob::RobTag;

/// Executes the Memory2 stage.
///
/// Returns the oldest memory-ordering violation observed this cycle (older
/// `RobTag`, lower index). The caller flushes the pipeline at that tag.
pub fn memory2_stage(
    state: &StageCtx<'_>,
    input: &mut Vec<Mem1Mem2Entry>,
    output: &mut Vec<Mem2WbEntry>,
    store_buffer: &mut StoreBuffer,
    mut load_queue: Option<&mut LoadQueue>,
    mut vec_store_buffer: Option<
        &mut crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer,
    >,
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
                pc      = %crate::common::trace::Hex(mem.pc),
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
                dirty_updates: mem.dirty_updates,
                sfence_vma: mem.sfence_vma,
                lr_sc: None,
                vec_mem: mem.vec_mem,
                observed: mem.observed,
            });
            continue;
        }

        let mut load_data = 0u64;
        let mut lr_sc: Option<LrScRecord> = None;

        if mem.ctrl.atomic_op == Some(AtomicOp::Lr) {
            load_data = load_result(mem.load_data, mem.ctrl.width, mem.ctrl.signed_load, false);
            lr_sc = Some(LrScRecord::Lr { paddr: mem.paddr });
        } else if mem.ctrl.atomic_op.is_some() {
            // The cache has performed the SC or AMO: `load_data` is the SC's
            // result or the AMO's old value, and nothing is left for the
            // store buffer to write. A younger load that already read the
            // location read it too early.
            store_buffer.remove_performed(mem.rob_tag);
            if let Some(ref lq) = load_queue
                && let Some(violating_tag) =
                    lq.check_ordering_violation(mem.paddr, mem.ctrl.width, mem.rob_tag)
            {
                merge_violation(&mut violation, (violating_tag, mem.pc));
            }
            load_data = load_result(mem.load_data, mem.ctrl.width, mem.ctrl.signed_load, false);
        } else if mem.ctrl.mem_read {
            // Demand load. Sign / zero extend `load_data` (which memory1 or
            // mailbox-drain populated) and apply FP NaN-boxing.
            load_data = load_result(
                mem.load_data,
                mem.ctrl.width,
                mem.ctrl.signed_load,
                mem.ctrl.fp_reg_write,
            );
            if mem.sb_forwarded {
                trace_fwd!(state.config.general.trace_instructions;
                    event         = "forward",
                    load_pc       = %crate::common::trace::Hex(mem.pc),
                    load_tag      = mem.rob_tag.0,
                    paddr         = %crate::common::trace::Hex(mem.paddr.val()),
                    width         = ?mem.ctrl.width,
                    signed        = mem.ctrl.signed_load,
                    forwarded_val = %crate::common::trace::Hex(load_data),
                    "M2: load satisfied from store buffer (memory1 hit)"
                );
            } else {
                trace_mem!(state.config.general.trace_instructions;
                    stage     = "M2",
                    rob_tag   = mem.rob_tag.0,
                    pc        = %crate::common::trace::Hex(mem.pc),
                    op        = "load",
                    paddr     = %crate::common::trace::Hex(mem.paddr.val()),
                    width     = ?mem.ctrl.width,
                    load_data = %crate::common::trace::Hex(load_data),
                    "M2: load value finalized"
                );
            }
            if let Some(ref mut lq) = load_queue {
                let micro_op = mem.vec_mem.as_ref().map(|v| v.micro_op);
                lq.fill_data(mem.rob_tag, micro_op, load_data, mem.observed);
            }
        } else if mem.ctrl.mem_write {
            // A scalar store resolved in memory1; a vector store element
            // resolves here and checks the load queue for ordering violations.
            for (paddr, data, width) in vector_store_elements(&mem) {
                if let Some(vsb) = vec_store_buffer.as_deref_mut() {
                    vsb.resolve_element(mem.rob_tag, paddr, data, width);
                }
                if let Some(ref lq) = load_queue
                    && let Some(violating_tag) =
                        lq.check_ordering_violation(paddr, width, mem.rob_tag)
                {
                    trace_fwd!(state.config.general.trace_instructions;
                        event           = "violation",
                        store_pc        = %crate::common::trace::Hex(mem.pc),
                        store_tag       = mem.rob_tag.0,
                        paddr           = %crate::common::trace::Hex(paddr.val()),
                        width           = ?width,
                        violation_flush = violating_tag.0,
                        "M2: memory ordering VIOLATION — younger load executed with stale data"
                    );
                    merge_violation(&mut violation, (violating_tag, mem.pc));
                }
            }
        } else {
            trace_mem!(state.config.general.trace_instructions;
                stage   = "M2",
                rob_tag = mem.rob_tag.0,
                pc      = %crate::common::trace::Hex(mem.pc),
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
            dirty_updates: mem.dirty_updates,
            sfence_vma: mem.sfence_vma,
            lr_sc,
            vec_mem: mem.vec_mem,
            observed: mem.observed,
        });
    }

    violation
}

/// The element writes a vector store micro-op resolves: its own, or each
/// of its span's at its place in the span. A scalar store has none here.
fn vector_store_elements(mem: &Mem1Mem2Entry) -> Vec<(PhysAddr, u64, MemWidth)> {
    let Some(access) = mem.vec_mem.as_ref() else { return Vec::new() };
    match &access.target {
        VecMemTarget::Element { .. } => vec![(mem.paddr, mem.store_data, mem.ctrl.width)],
        VecMemTarget::Span(span) => span
            .elements
            .iter()
            .map(|(_, element)| {
                let paddr = PhysAddr::new(mem.paddr.val() + span.offset_of(element) as u64);
                (paddr, element.store_data, mem_width_from_eew_bytes(element.eew.bytes()))
            })
            .collect(),
    }
}

const fn merge_violation(slot: &mut Option<(RobTag, u64)>, new: (RobTag, u64)) {
    match slot {
        None => *slot = Some(new),
        Some((existing, _)) if new.0.is_older_than(*existing) => *slot = Some(new),
        _ => {}
    }
}
