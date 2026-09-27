//! Mailbox-drain stage: matches `MemResp` packets against the engine's
//! outstanding-request tables and feeds the M1→M2 latch (loads) or the
//! frontend's F1→F2 latch (fetches).
//!
//! Runs at the top of [`Pipeline::tick`](crate::core::pipeline::engine::Pipeline::tick).
//! Each `MemResp` resolves to one of four cases:
//!
//! 1. **Walk response** — read the PTE bytes from RAM at `walk.pte_addr`,
//!    hand them to [`StageCtx::translate_continue`](crate::sim::StageCtx::translate_continue),
//!    then either issue the next PTE request (multi-level walk) or trigger
//!    the parked continuation (fetch / load / store).
//! 2. **Fetch response** — release the fetch group's
//!    [`Fetch1Fetch2Entry`](crate::core::pipeline::latches::Fetch1Fetch2Entry)
//!    values into the fetch1→fetch2 latch, in program order.
//! 3. **Load response** — read the raw load value (RAM fast-path or
//!    `MemResp.data` for MMIO) and push a `Mem1Mem2Entry` into the M1→M2
//!    latch with `load_data` filled. Memory2 takes over from there for
//!    sign-extension, AMO RMW, and SB ordering checks.
//! 4. **Store ack** — fire-and-forget; drop the outstanding entry.

use crate::common::constants::PAGE_SHIFT;
use crate::common::{ExceptionStage, PhysAddr, PteUpdate, TranslationResult};
use crate::core::pipeline::backend::shared::cbo;
use crate::core::pipeline::engine::{BackendCommon, ExecutionEngine, Pipeline};
use crate::core::pipeline::frontend::fetch1::{dispatch_fetch_group, drain_fetch_reorder};
use crate::core::pipeline::latches::Mem1Mem2Entry;
use crate::core::pipeline::outstanding::{
    DelayedAccess, OutstandingFetch, OutstandingLoad, OutstandingStore, OutstandingWalk,
    WalkContinuation,
};
use crate::core::pipeline::rob::RobTag;
use crate::core::pipeline::signals::MemWidth;
use crate::sim::StageCtx;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::packet::{AccessSize, MemOp, MemRespData, Packet, WriteData};
use crate::sim::state::memory::TranslateResult;
use crate::sim::state::write_log::WriteLog;

/// Processes every packet currently in the engine's mailbox.
pub fn drain<E: ExecutionEngine>(pipeline: &mut Pipeline<E>, state: &mut StageCtx<'_>) {
    let mailbox = std::mem::take(&mut pipeline.engine.common_mut().mailbox);
    for (_source, packet) in mailbox {
        let Packet::MemResp { req_id, data, .. } = packet else {
            continue;
        };

        if let Some(walk) = pipeline.engine.common_mut().outstanding_walks.remove(&req_id) {
            complete_walk(pipeline, state, walk);
        } else if let Some(fetch) = pipeline.engine.common_mut().outstanding_fetches.remove(&req_id)
        {
            buffer_fetch(pipeline, fetch);
        } else if let Some(load) = take_completed_load(pipeline.engine.common_mut(), req_id) {
            complete_load(pipeline, state, load, &data);
        } else {
            // outstanding_stores ack or stale post-flush response — drop.
            let _ = pipeline.engine.common_mut().outstanding_stores.remove(&req_id);
        }
    }

    drain_fetch_reorder(
        state.cycle,
        pipeline.engine.common_mut(),
        &mut pipeline.frontend.fetch_buffer,
        &mut pipeline.frontend.fetch1_fetch2,
    );
}

/// Accounts one answered part of the load `req_id` belongs to and returns
/// the load once every part has answered.
fn take_completed_load(common: &mut BackendCommon, req_id: ReqId) -> Option<OutstandingLoad> {
    let primary = common.load_parts.remove(&req_id).unwrap_or(req_id);
    let load = common.outstanding_loads.get_mut(&primary)?;
    load.parts_outstanding = load.parts_outstanding.saturating_sub(1);
    if load.parts_outstanding > 0 {
        return None;
    }
    common.outstanding_loads.remove(&primary)
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

/// Reads the load's raw bytes from RAM (fast path) or the device-supplied
/// `MemResp` payload (MMIO) and pushes a `Mem1Mem2Entry` into the M1→M2
/// latch. Memory2 handles sign-extension, AMO RMW, and SB resolution.
fn complete_load<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    state: &StageCtx<'_>,
    load: OutstandingLoad,
    resp_data: &MemRespData,
) {
    let entry = load.entry;
    let paddr = load.paddr;
    let load_raw = read_load_bytes(state, paddr.val(), entry.ctrl.width, resp_data);
    let observed = state.write_log.as_ref().map(WriteLog::now);
    let cycle = state.cycle;

    if let Some(log) = state.write_log.as_ref()
        && let Some(load_queue) = pipeline.engine.load_queue_mut()
        && let Some(violator) =
            load_queue.check_coherence_violation(entry.rob_tag, paddr, log, state.hart().hart_id)
    {
        pipeline.engine.common_mut().note_coherence_violation(violator);
    }

    pipeline.engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        load_data: load_raw,
        complete_cycle: cycle,
        dirty_updates: load.dirty_updates,
        observed,
        ..Mem1Mem2Entry::from_execute(entry, load.vaddr, paddr)
    });
}

/// Advances an in-flight page-table walk. Either completes (triggering the
/// continuation) or issues the next PTE `MemReq`.
fn complete_walk<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    state: &mut StageCtx<'_>,
    walk: OutstandingWalk,
) {
    let raw_pte = read_pte_bytes(state, walk.pte_addr);
    let bus_transit = state.bus.calculate_transit_time(8);
    let walked_page = walk.state.vaddr.val() >> PAGE_SHIFT;
    let outcome = state.translate_continue(walk.state, raw_pte, bus_transit);
    match outcome {
        TranslateResult::Ready(result) => {
            if let Some(update) = result.accessed_update {
                set_accessed_bit(pipeline, state, &update);
            }
            dispatch_walk_continuation(pipeline, state, walk.continuation, walked_page, result);
        }
        TranslateResult::NeedPte { pte_addr, state: walk_state } => {
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
    let Some(pte) = state.set_pte_accessed(update) else { return };
    let common = pipeline.engine.common_mut();
    let req_id = common.alloc_req_id();
    let _ = common
        .outstanding_stores
        .insert(req_id, OutstandingStore { rob_tag: RobTag::default(), paddr: update.pte_addr });
    let (l1_d_id, pipeline_id) = (common.l1_d_id, common.pipeline_id);
    let cycle = state.cycle;
    state.events().schedule(
        cycle,
        ComponentId::Cache(l1_d_id),
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr: update.pte_addr,
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Write { data: WriteData::Small(pte) },
        },
    );
}

/// Reads a 64-bit PTE from the RAM fast path. RISC-V doesn't permit page
/// tables in MMIO, so the read is always backed by DRAM.
fn read_pte_bytes(state: &StageCtx<'_>, pte_addr: PhysAddr) -> u64 {
    let raw = pte_addr.val();
    state.bus.ram_region().filter(|r| r.contains(raw, 8)).map_or(0u64, |r| {
        // SAFETY: `RamRegion::contains(raw, 8)` bounds-checks the access.
        unsafe { r.ptr(raw).cast::<u64>().read_unaligned() }
    })
}

/// Reads the raw bytes of a load. RAM accesses use the fast-path pointer;
/// MMIO loads take their data from the device's `MemResp` payload.
fn read_load_bytes(
    state: &StageCtx<'_>,
    paddr: u64,
    width: MemWidth,
    resp_data: &MemRespData,
) -> u64 {
    let size = match width {
        MemWidth::Byte => 1u64,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
        MemWidth::Nop => 0,
    };
    if size > 0
        && let Some(r) = state.bus.ram_region_for(paddr, size)
    {
        // SAFETY: `ram_region_for` confirms pure-RAM coverage and bounds.
        return unsafe {
            let ptr = r.ptr(paddr);
            match width {
                MemWidth::Byte => u64::from(*ptr),
                MemWidth::Half => u64::from(ptr.cast::<u16>().read_unaligned()),
                MemWidth::Word => u64::from(ptr.cast::<u32>().read_unaligned()),
                MemWidth::Double => ptr.cast::<u64>().read_unaligned(),
                MemWidth::Nop => 0,
            }
        };
    }
    match resp_data {
        MemRespData::Small(v) => *v,
        MemRespData::Line(_) => 0,
    }
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
