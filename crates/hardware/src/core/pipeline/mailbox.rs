//! Mailbox-drain stage: matches `MemResp` packets against the engine's
//! outstanding-request tables and feeds the M1→M2 latch (loads) or the
//! frontend's F1→F2 latch (fetches).
//!
//! Runs at the top of [`Pipeline::tick`](crate::core::pipeline::engine::Pipeline::tick).
//! Each `MemResp` resolves to one of four cases:
//!
//! 1. **Walk response** — read the PTE bytes from RAM at `walk.pte_addr`,
//!    hand them to [`CoreCtx::translate_continue`](crate::sim::CoreCtx::translate_continue),
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

use crate::common::{ExceptionStage, InstSize, LineAddr, PhysAddr};
use crate::sim::CoreCtx;
use crate::sim::state::memory::TranslateResult;
use crate::sim::state::write_log::WriteLog;
use crate::core::pipeline::engine::{ExecutionEngine, Pipeline};
use crate::core::pipeline::frontend::fetch1::{
    FetchWalkHalf, dispatch_fetch_group, drain_fetch_reorder, inst_size_at,
};
use crate::core::pipeline::latches::Mem1Mem2Entry;
use crate::core::pipeline::outstanding::{
    OutstandingFetch, OutstandingLoad, OutstandingWalk, WalkContinuation,
};
use crate::core::pipeline::signals::MemWidth;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::packet::{AccessSize, MemOp, MemRespData, Packet};

/// Processes every packet currently in the engine's mailbox.
pub fn drain<E: ExecutionEngine>(pipeline: &mut Pipeline<E>, state: &mut CoreCtx<'_>) {
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
        } else if let Some(load) = pipeline.engine.common_mut().outstanding_loads.remove(&req_id) {
            complete_load(pipeline, state, load, &data);
        } else {
            // outstanding_stores ack or stale post-flush response — drop.
            let _ = pipeline.engine.common_mut().outstanding_stores.remove(&req_id);
        }
    }

    drain_fetch_reorder(
        pipeline.engine.common_mut(),
        &mut pipeline.frontend.fetch_buffer,
        &mut pipeline.frontend.fetch1_fetch2,
    );
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
    state: &CoreCtx<'_>,
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
            load_queue.check_coherence_violation(entry.rob_tag, paddr, log, state.hart.hart_id)
    {
        pipeline.engine.common_mut().note_coherence_violation(violator);
    }

    pipeline.engine.mem1_mem2_mut().push(Mem1Mem2Entry {
        rob_tag: entry.rob_tag,
        pc: entry.pc,
        inst: entry.inst,
        inst_size: entry.inst_size,
        rd: entry.rd,
        rd_phys: entry.rd_phys,
        alu: entry.alu,
        vaddr: load.vaddr,
        paddr,
        store_data: entry.store_data,
        load_data: load_raw,
        sb_forwarded: false,
        ctrl: entry.ctrl,
        trap: None,
        exception_stage: None,
        fp_flags: entry.fp_flags,
        complete_cycle: cycle,
        pte_update: load.pte_update,
        sfence_vma: entry.sfence_vma,
        vec_mem: entry.vec_mem,
        observed,
    });
}

/// Advances an in-flight page-table walk. Either completes (triggering the
/// continuation) or issues the next PTE `MemReq`.
fn complete_walk<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    state: &mut CoreCtx<'_>,
    walk: OutstandingWalk,
) {
    let raw_pte = read_pte_bytes(state, walk.pte_addr);
    let bus_transit = state.bus.calculate_transit_time(8);
    let outcome = state.translate_continue(walk.state, raw_pte, bus_transit);
    match outcome {
        TranslateResult::Ready(result) => {
            dispatch_walk_continuation(pipeline, state, walk.continuation, result);
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
    state: &mut CoreCtx<'_>,
    continuation: WalkContinuation,
    result: crate::common::TranslationResult,
) {
    match continuation {
        WalkContinuation::Fetch { fetch_seq, mut entry, half } => {
            pipeline.engine.common_mut().fetch_walk_pending = false;
            let line = if let Some(trap) = result.trap {
                entry.trap = Some(trap);
                entry.exception_stage = Some(ExceptionStage::Fetch);
                entry.paddr = PhysAddr::new(0);
                None
            } else {
                if half == FetchWalkHalf::Lower {
                    entry.paddr = result.paddr;
                }
                let line_bytes = state.core.l1_i_cache.line_bytes() as u64;
                Some(LineAddr::from_phys(entry.paddr, line_bytes))
            };
            // Fetch held at the parked PC while the lower half's
            // translation was outstanding; now the encoding is readable
            // and fetch resumes after this instruction. A faulting fetch
            // has no size: the trap redirects fetch anyway.
            if half == FetchWalkHalf::Lower {
                let size = if entry.trap.is_some() {
                    InstSize::Standard
                } else {
                    inst_size_at(state, entry.paddr)
                };
                pipeline.engine.common_mut().fetch_resume_pc = Some(entry.pc.wrapping_add(size.as_u64()));
            }
            dispatch_fetch_group(
                state,
                &mut pipeline.engine,
                &mut pipeline.frontend.fetch_buffer,
                &mut pipeline.frontend.fetch1_fetch2,
                OutstandingFetch { fetch_seq, line, entries: vec![entry] },
            );
        }
        WalkContinuation::LoadStore(mut entry) => {
            if let Some(trap) = result.trap {
                // Walker returned a page fault (e.g. software A/D unset).
                // Stamp the trap on the entry so memory1 propagates it to
                // memory2 / writeback rather than re-translating and
                // re-walking forever.
                entry.trap = Some(trap);
                entry.exception_stage = Some(ExceptionStage::Memory);
            }
            // Re-inject into Execute→Memory1 so the next memory1 tick runs
            // with the TLB now warm (success path) or surfaces the trap
            // through the normal stage transitions (fault path).
            pipeline.engine.execute_mem1_mut().push(entry);
        }
    }
}

/// Reads a 64-bit PTE from the RAM fast path. RISC-V doesn't permit page
/// tables in MMIO, so the read is always backed by DRAM.
fn read_pte_bytes(state: &CoreCtx<'_>, pte_addr: PhysAddr) -> u64 {
    let raw = pte_addr.val();
    state.bus.ram_region().filter(|r| r.contains(raw, 8)).map_or(0u64, |r| {
        // SAFETY: `RamRegion::contains(raw, 8)` bounds-checks the access.
        unsafe { r.ptr(raw).cast::<u64>().read_unaligned() }
    })
}

/// Reads the raw bytes of a load. RAM accesses use the fast-path pointer;
/// MMIO loads take their data from the device's `MemResp` payload.
fn read_load_bytes(state: &CoreCtx<'_>, paddr: u64, width: MemWidth, resp_data: &MemRespData) -> u64 {
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
    state: &mut CoreCtx<'_>,
    req_id: ReqId,
    pte_addr: PhysAddr,
) {
    let common = pipeline.engine.common();
    let cycle = state.cycle;
    state.event_queue.schedule(
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
