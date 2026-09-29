//! Mailbox-drain stage: matches `MemResp` packets against the engine's
//! outstanding-request tables and feeds the M1→M2 latch (loads) or the
//! frontend's F1→F2 latch (fetches).
//!
//! Runs at the top of [`Pipeline::tick`](crate::uarch::pipeline::engine::Pipeline::tick).
//! Each `MemResp` resolves to one of four cases:
//!
//! 1. **Walk response** — read the PTE bytes from RAM at `walk.pte_addr`,
//!    hand them to [`StageCtx::translate_continue`](crate::uarch::ctx::StageCtx::translate_continue),
//!    then either issue the next PTE request (multi-level walk) or trigger
//!    the parked continuation (fetch / load / store).
//! 2. **Fetch response** — release the fetch group's
//!    [`Fetch1Fetch2Entry`](crate::uarch::pipeline::latches::Fetch1Fetch2Entry)
//!    values into the fetch1→fetch2 latch, in program order.
//! 3. **Load response** — read the raw load value (RAM fast-path or
//!    `MemResp.data` for MMIO) and push a `Mem1Mem2Entry` into the M1→M2
//!    latch with `load_data` filled. Memory2 takes over from there for
//!    sign-extension, AMO RMW, and SB ordering checks.
//! 4. **Store ack** — fire-and-forget; drop the outstanding entry.

use crate::arch::translation::{PteUpdate, TranslationResult};
use crate::common::{PAGE_SHIFT, PhysAddr};
use crate::exec::cbo;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::packet::{AccessSize, MemOp, MemRespData, Packet, WriteData, WriteOrigin};
use crate::uarch::ctx::StageCtx;
use crate::uarch::ctx::stage::PteUpdateOutcome;
use crate::uarch::mmu::TranslateOutcome;
use crate::uarch::pipeline::engine::{BackendCommon, ExecutionEngine, Pipeline};
use crate::uarch::pipeline::exception::ExceptionStage;
use crate::uarch::pipeline::frontend::fetch1::{dispatch_fetch_group, drain_fetch_reorder};
use crate::uarch::pipeline::latches::Mem1Mem2Entry;
use crate::uarch::pipeline::outstanding::{
    DelayedAccess, OutstandingFetch, OutstandingLoad, OutstandingStore, OutstandingWalk, PartRead,
    StoreOwner, WalkContinuation,
};

/// Processes every packet currently in the engine's mailbox.
pub fn drain<E: ExecutionEngine>(pipeline: &mut Pipeline<E>, state: &mut StageCtx<'_>) {
    let mailbox = std::mem::take(&mut pipeline.engine.common_mut().mailbox);
    for (_source, packet) in mailbox {
        let Packet::MemResp { req_id, data, .. } = packet else {
            continue;
        };

        if let Some(walk) = pipeline.engine.common_mut().outstanding_walks.remove(&req_id) {
            complete_walk(pipeline, state, walk, &data);
        } else if let Some(fetch) = pipeline.engine.common_mut().outstanding_fetches.remove(&req_id)
        {
            buffer_fetch(pipeline, fetch);
        } else if let Some((mut load, read)) =
            take_completed_load(pipeline.engine.common_mut(), req_id, &data)
        {
            if let MemRespData::PerformedBytes { bytes, .. } = data {
                load.set_span_data(bytes);
            }
            complete_load(pipeline, state, load, read);
        } else if let Some(store) = pipeline.engine.common_mut().outstanding_stores.remove(&req_id)
        {
            acknowledge_write(pipeline, state, store.owner, req_id);
        }
    }
    release_forwarded_loads(&mut pipeline.engine, state.cycle);

    drain_fetch_reorder(
        state.cycle,
        pipeline.engine.common_mut(),
        &mut pipeline.frontend.fetch_buffer,
        &mut pipeline.frontend.fetch1_fetch2,
    );
}

/// Moves forwarded loads whose L1D latency has elapsed into the M1→M2 latch.
fn release_forwarded_loads<E: ExecutionEngine>(engine: &mut E, now: u64) {
    let mut ready = Vec::new();
    engine.common_mut().forwarded_loads.retain(|load| {
        if load.ready_cycle <= now {
            ready.push(load.entry.clone());
            false
        } else {
            true
        }
    });
    engine.mem1_mem2_mut().extend(ready);
}

/// Tells the buffer a write came from that the memory system has taken it.
fn acknowledge_write<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    state: &mut StageCtx<'_>,
    owner: StoreOwner,
    req: ReqId,
) {
    match owner {
        StoreOwner::StoreBuffer => {
            let _ = pipeline.engine.store_buffer_mut().write_acked(req);
        }
        StoreOwner::VecStoreBuffer => pipeline.engine.vec_store_buffer_mut().write_acked(req),
        StoreOwner::WriteCombining => state.core_mut().wcb.acked(req),
        StoreOwner::Untracked => {}
    }
}

/// Records what the part of a load `req_id` answered read, and returns the
/// load with its whole read once every part has answered.
fn take_completed_load(
    common: &mut BackendCommon,
    req_id: ReqId,
    data: &MemRespData,
) -> Option<(OutstandingLoad, PartRead)> {
    let (primary, high) =
        common.load_parts.remove(&req_id).map_or((req_id, false), |primary| (primary, true));
    let load = common.outstanding_loads.get_mut(&primary)?;
    load.parts.record(high, PartRead::of(data));
    let read = load.parts.assembled()?;
    common.outstanding_loads.remove(&primary).map(|load| (load, read))
}

/// Inserts a returned fetch group into the reorder buffer at its
/// `fetch_seq`; the drain after the mailbox releases it once every older
/// group has completed.
fn buffer_fetch<E: ExecutionEngine>(pipeline: &mut Pipeline<E>, group: OutstandingFetch) {
    let common = pipeline.engine.common_mut();
    // Stale-after-flush responses have fetch_seqs below the post-flush
    // emit cursor; drop them rather than reintroducing wrong-path entries.
    if group.fetch_seq < common.next_emit_fetch_seq {
        return;
    }
    let _ = common.fetch_reorder.insert(group.fetch_seq, group);
}

/// Pushes a load with what it read when the memory system served it (or
/// the device's answer) into the M1→M2 latch. Memory2 handles
/// sign-extension, AMO RMW, and SB resolution.
fn complete_load<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    state: &StageCtx<'_>,
    load: OutstandingLoad,
    read: PartRead,
) {
    let entry = load.entry;
    let paddr = load.paddr;
    let cycle = state.cycle;

    if let Some(log) = state.memory.write_log()
        && let Some(load_queue) = pipeline.engine.load_queue_mut()
        && let Some(violator) =
            load_queue.check_coherence_violation(entry.rob_tag, paddr, log, state.hart().hart_id)
    {
        pipeline.engine.common_mut().note_coherence_violation(violator);
    }

    pipeline.engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        load_data: read.value,
        complete_cycle: cycle,
        dirty_updates: load.dirty_updates,
        observed: read.observed,
        ..Mem1Mem2Entry::from_execute(entry, load.vaddr, paddr)
    });
}

/// Advances an in-flight page-table walk. Either completes (triggering the
/// continuation) or issues the next PTE `MemReq`.
fn complete_walk<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    state: &mut StageCtx<'_>,
    walk: OutstandingWalk,
    data: &MemRespData,
) {
    let raw_pte = PartRead::of(data).value;
    let bus_transit = state.bus.calculate_transit_time(8);
    let walked_page = walk.state.vaddr.val() >> PAGE_SHIFT;
    let outcome = state.translate_continue(walk.state, raw_pte, bus_transit);
    match outcome {
        TranslateOutcome::Ready(result) => {
            if let Some(update) = result.accessed_update {
                set_accessed_bit(pipeline, state, &update);
            }
            dispatch_walk_continuation(pipeline, state, walk.continuation, walked_page, result);
        }
        TranslateOutcome::NeedPte { pte_addr, state: walk_state } => {
            let common = pipeline.engine.common_mut();
            let req_id = common.alloc_req_id();
            let _ = common.outstanding_walks.insert(
                req_id,
                OutstandingWalk { state: walk_state, pte_addr, continuation: walk.continuation },
            );
            emit_pte_req(pipeline, state, req_id, pte_addr);
        }
    }
}

/// Runs the appropriate continuation once a walk reaches Ready.
fn dispatch_walk_continuation<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    state: &mut StageCtx<'_>,
    continuation: WalkContinuation,
    walked_page: u64,
    result: TranslationResult,
) {
    match continuation {
        WalkContinuation::Fetch { fetch_seq, mut entry } => {
            pipeline.engine.common_mut().fetch_walk_pending = false;
            if result.trap.is_none() {
                // The TLB now holds the page. Fetch the instruction again
                // from fetch1 so its size, its upper half-word and its
                // prediction are formed the normal way; the sequence
                // number reserved for it drains empty.
                pipeline.engine.common_mut().fetch_resume_pc = Some(entry.pc);
                dispatch_fetch_group(
                    state,
                    &mut pipeline.engine,
                    &mut pipeline.frontend.fetch_buffer,
                    &mut pipeline.frontend.fetch1_fetch2,
                    OutstandingFetch { fetch_seq, line: None, entries: Vec::new() },
                );
                return;
            }
            entry.trap = result.trap;
            entry.exception_stage = Some(ExceptionStage::Fetch);
            entry.paddr = PhysAddr::new(0);
            dispatch_fetch_group(
                state,
                &mut pipeline.engine,
                &mut pipeline.frontend.fetch_buffer,
                &mut pipeline.frontend.fetch1_fetch2,
                OutstandingFetch { fetch_seq, line: None, entries: vec![entry] },
            );
        }
        WalkContinuation::LoadStore { mut entry, mut translations } => {
            if result.trap.is_none() {
                // Memory1 continues with the walk's own translation: it
                // carries the D-bit update a store applies at commit, which
                // translating again through the TLB would lose.
                let translation =
                    Some(TranslationResult { cycles: 0, accessed_update: None, ..result });
                if walked_page == entry.alu >> PAGE_SHIFT {
                    translations.first = translation;
                } else {
                    translations.second = translation;
                }
                pipeline.engine.common_mut().mem1_delayed.push(DelayedAccess {
                    ready_cycle: state.cycle,
                    entry,
                    translations: *translations,
                });
                return;
            }
            if let Some(trap) = result.trap {
                let op = entry.ctrl.system_op;
                let trap = if op.is_cbo() {
                    cbo::as_store_fault(trap, cbo::fault_address(op, entry.alu))
                } else {
                    trap
                };
                // Stamp the trap on the entry so memory1 propagates it to
                // memory2 / writeback rather than walking again.
                entry.trap = Some(trap);
                entry.exception_stage = Some(ExceptionStage::Memory);
            }
            pipeline.engine.execute_mem1_mut().push(entry);
        }
    }
}

/// Sets the A bit the walk found clear and sends the PTE write to the L1D,
/// as the walker's own store.
fn set_accessed_bit<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    state: &mut StageCtx<'_>,
    update: &PteUpdate,
) {
    if let PteUpdateOutcome::Written(pte) = state.apply_pte_update(update) {
        send_pte_write(pipeline.engine.common_mut(), state, update.pte_addr, pte);
    }
}

/// Sends the L1D the timing of a PTE the walker has already written.
pub(crate) fn send_pte_write(
    common: &mut BackendCommon,
    state: &mut StageCtx<'_>,
    pte_addr: PhysAddr,
    pte: u64,
) {
    let req_id = common.alloc_req_id();
    let _ = common
        .outstanding_stores
        .insert(req_id, OutstandingStore { owner: StoreOwner::Untracked, paddr: pte_addr });
    let (l1_d_id, pipeline_id) = (common.l1_d_id, common.pipeline_id);
    let cycle = state.cycle;
    state.events().schedule(
        cycle,
        ComponentId::Cache(l1_d_id),
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr: pte_addr,
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Write { data: WriteData::Small(pte), origin: WriteOrigin::Placed },
        },
    );
}

/// Emits a PTE read request to the L1 data cache.
fn emit_pte_req<E: ExecutionEngine>(
    pipeline: &Pipeline<E>,
    state: &mut StageCtx<'_>,
    req_id: ReqId,
    pte_addr: PhysAddr,
) {
    let common = pipeline.engine.common();
    let cycle = state.cycle;
    state.events().schedule(
        cycle,
        ComponentId::Cache(common.l1_d_id),
        ComponentId::Pipeline(common.pipeline_id),
        Packet::MemReq {
            req_id,
            paddr: pte_addr,
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Read,
        },
    );
}
