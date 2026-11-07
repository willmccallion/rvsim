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
use crate::common::trace::Hex;
use crate::exec::memory::load_result;
use crate::isa::op::{AtomicOp, MemWidth};
use crate::trace_fwd;
use crate::trace_mem;
use crate::trace_trap;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::backend::shared::vec_mem::mem_width_from_eew_bytes;
use crate::uarch::pipeline::latches::{Mem1Mem2Entry, Mem2WbEntry, VecMemTarget};
use crate::uarch::pipeline::lsq::load_queue::{LoadQueue, OrderViolation};
use crate::uarch::pipeline::lsq::store_buffer::StoreBuffer;
use crate::uarch::pipeline::lsq::vec_store_buffer::VecStoreBuffer;

/// The queues Memory2 resolves stores and checks load ordering in; the
/// in-order backend has no load queue.
pub struct MemoryQueues<'a> {
    /// Scalar stores awaiting commit.
    pub store_buffer: &'a mut StoreBuffer,
    /// Executed loads, checked for ordering violations.
    pub load_queue: Option<&'a mut LoadQueue>,
    /// Vector stores awaiting commit.
    pub vec_store_buffer: &'a mut VecStoreBuffer,
}

/// Executes the Memory2 stage.
///
/// Returns the oldest memory-ordering violation observed this cycle (older
/// `RobTag`, lower index). The caller flushes the pipeline at that tag.
pub fn memory2_stage(
    state: &StageCtx<'_>,
    input: &mut Vec<Mem1Mem2Entry>,
    output: &mut Vec<Mem2WbEntry>,
    queues: MemoryQueues<'_>,
) -> Option<OrderViolation> {
    let MemoryQueues { store_buffer, mut load_queue, vec_store_buffer } = queues;
    let mut violation: Option<OrderViolation> = None;
    let mut entries = std::mem::take(input);

    // Stores and younger loads need to be visible to each other in program
    // order so the SB resolve in this cycle still flags an ordering violation
    // against a load that already executed.
    entries.sort_by(|a, b| a.op.rob_tag.age_cmp(b.op.rob_tag));

    output.clear();

    for mem in entries {
        if let Some(ref trap) = mem.op.trap {
            trace_trap!(state.trace_trap_enabled(trap);
                event   = "propagate",
                stage   = "M2",
                pc      = %Hex(mem.op.pc.val()),
                rob_tag = %mem.op.rob_tag,
                trap    = ?trap,
                "M2: trap propagated through memory2"
            );
            output.push(Mem2WbEntry::from_memory2(mem, 0, None));
            continue;
        }

        if mem.op.ctrl.atomic_op.is_none() && mem.op.ctrl.mem_read {
            output.push(finalize_load(state, mem, load_queue.as_deref_mut(), "M2"));
            continue;
        }

        let mut load_data = 0u64;
        let mut lr_sc: Option<LrScRecord> = None;

        if mem.op.ctrl.atomic_op == Some(AtomicOp::Lr) {
            load_data =
                load_result(mem.load_data, mem.op.ctrl.width, mem.op.ctrl.signed_load, false);
            lr_sc = Some(LrScRecord::Lr { paddr: mem.paddr });
        } else if mem.op.ctrl.atomic_op.is_some() {
            // The cache has performed the SC or AMO: `load_data` is the SC's
            // result or the AMO's old value, and nothing is left for the
            // store buffer to write. A younger load that already read the
            // location read it too early.
            store_buffer.remove_performed(mem.op.rob_tag);
            if let Some(ref lq) = load_queue
                && let Some(violating_tag) =
                    lq.check_ordering_violation(mem.paddr, mem.op.ctrl.width, mem.op.rob_tag)
            {
                let found = OrderViolation { load: violating_tag, store_pc: mem.op.pc };
                violation = OrderViolation::older(violation, Some(found));
            }
            load_data =
                load_result(mem.load_data, mem.op.ctrl.width, mem.op.ctrl.signed_load, false);
        } else if mem.op.ctrl.mem_write {
            // A scalar store resolved in memory1; a vector store element
            // resolves here and checks the load queue for ordering violations.
            for (paddr, data, width) in vector_store_elements(&mem) {
                vec_store_buffer.resolve_element(mem.op.rob_tag, paddr, data, width);
                if let Some(ref lq) = load_queue
                    && let Some(violating_tag) =
                        lq.check_ordering_violation(paddr, width, mem.op.rob_tag)
                {
                    trace_fwd!(state.config.general.trace_instructions;
                        event           = "violation",
                        store_pc        = %Hex(mem.op.pc.val()),
                        store_tag       = %mem.op.rob_tag,
                        paddr           = %Hex(paddr.val()),
                        width           = ?width,
                        violation_flush = %violating_tag,
                        "M2: memory ordering VIOLATION — younger load executed with stale data"
                    );
                    let found = OrderViolation { load: violating_tag, store_pc: mem.op.pc };
                    violation = OrderViolation::older(violation, Some(found));
                }
            }
        } else {
            trace_mem!(state.config.general.trace_instructions;
                stage   = "M2",
                rob_tag = %mem.op.rob_tag,
                pc      = %Hex(mem.op.pc.val()),
                op      = "passthrough",
                "M2: non-memory instruction pass-through"
            );
        }

        output.push(Mem2WbEntry::from_memory2(mem, load_data, lr_sc));
    }

    violation
}

/// Finalizes a demand load's value: sign or zero extends the raw bytes
/// memory1, the mailbox drain or a forwarding store put in `load_data`,
/// NaN-boxes an FP load, and gives the load queue the value. `stage` names
/// the stage doing it in the trace.
pub fn finalize_load(
    state: &StageCtx<'_>,
    mem: Mem1Mem2Entry,
    load_queue: Option<&mut LoadQueue>,
    stage: &'static str,
) -> Mem2WbEntry {
    let load_data = load_result(
        mem.load_data,
        mem.op.ctrl.width,
        mem.op.ctrl.signed_load,
        mem.op.ctrl.fp_reg_write,
    );
    if mem.sb_forwarded {
        trace_fwd!(state.config.general.trace_instructions;
            event         = "forward",
            stage         = stage,
            load_pc       = %Hex(mem.op.pc.val()),
            load_tag      = %mem.op.rob_tag,
            paddr         = %Hex(mem.paddr.val()),
            width         = ?mem.op.ctrl.width,
            signed        = mem.op.ctrl.signed_load,
            forwarded_val = %Hex(load_data),
            "load satisfied from store buffer (memory1 hit)"
        );
    } else {
        trace_mem!(state.config.general.trace_instructions;
            stage         = stage,
            rob_tag   = %mem.op.rob_tag,
            pc        = %Hex(mem.op.pc.val()),
            op        = "load",
            paddr     = %Hex(mem.paddr.val()),
            width     = ?mem.op.ctrl.width,
            load_data = %Hex(load_data),
            "load value finalized"
        );
    }
    if let Some(lq) = load_queue {
        let micro_op = mem.op.vec_mem.as_ref().map(|v| v.micro_op);
        lq.fill_data(mem.op.rob_tag, micro_op, load_data, mem.observed);
    }
    Mem2WbEntry::from_memory2(mem, load_data, None)
}

/// The element writes a vector store micro-op resolves: its own, or each
/// of its span's at its place in the span. A scalar store has none here.
fn vector_store_elements(mem: &Mem1Mem2Entry) -> Vec<(PhysAddr, u64, MemWidth)> {
    let Some(access) = mem.op.vec_mem.as_ref() else { return Vec::new() };
    match &access.target {
        VecMemTarget::Element { .. } => vec![(mem.paddr, mem.store_data, mem.op.ctrl.width)],
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
