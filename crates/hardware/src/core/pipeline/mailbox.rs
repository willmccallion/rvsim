//! Mailbox-drain stage: matches `MemResp` packets against the engine's
//! outstanding-request tables and feeds the M1→M2 latch (loads) or the
//! frontend's F1→F2 latch (fetches).
//!
//! Runs at the top of [`Pipeline::tick`](crate::core::pipeline::engine::Pipeline::tick).
//! Each `MemResp` resolves to one of four cases:
//!
//! 1. **Walk response** — read the PTE bytes from RAM at `walk.pte_addr`,
//!    hand them to [`Cpu::translate_continue`](crate::core::cpu::memory::Cpu::translate_continue),
//!    then either issue the next PTE request (multi-level walk) or trigger
//!    the parked continuation (fetch / load / store).
//! 2. **Fetch response** — push a fully-formed
//!    [`Fetch1Fetch2Entry`](crate::core::pipeline::latches::Fetch1Fetch2Entry)
//!    into the fetch1→fetch2 latch.
//! 3. **Load response** — read the raw load value (RAM fast-path or
//!    `MemResp.data` for MMIO) and push a `Mem1Mem2Entry` into the M1→M2
//!    latch with `load_data` filled. Memory2 takes over from there for
//!    sign-extension, AMO RMW, and SB ordering checks.
//! 4. **Store ack** — fire-and-forget; drop the outstanding entry.

use crate::common::{ExceptionStage, PhysAddr, VirtAddr};
use crate::core::Cpu;
use crate::core::cpu::memory::TranslateResult;
use crate::core::pipeline::engine::{ExecutionEngine, Pipeline};
use crate::core::pipeline::latches::{Fetch1Fetch2Entry, Mem1Mem2Entry};
use crate::core::pipeline::outstanding::{
    OutstandingFetch, OutstandingLoad, OutstandingWalk, WalkContinuation,
};
use crate::core::pipeline::signals::MemWidth;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::packet::{AccessSize, MemOp, MemRespData, Packet};

/// Processes every packet currently in the engine's mailbox.
pub fn drain<E: ExecutionEngine>(pipeline: &mut Pipeline<E>, cpu: &mut Cpu) {
    let mailbox = std::mem::take(&mut pipeline.engine.common_mut().mailbox);
    for (_source, packet) in mailbox {
        let Packet::MemResp { req_id, data, .. } = packet else {
            continue;
        };

        if let Some(walk) = pipeline.engine.common_mut().outstanding_walks.remove(&req_id) {
            complete_walk(pipeline, cpu, walk);
        } else if let Some(fetch) = pipeline.engine.common_mut().outstanding_fetches.remove(&req_id)
        {
            buffer_fetch(pipeline, fetch);
        } else if let Some(load) = pipeline.engine.common_mut().outstanding_loads.remove(&req_id) {
            complete_load(pipeline, cpu, load, &data);
        } else {
            // outstanding_stores ack or stale post-flush response — drop.
            let _ = pipeline.engine.common_mut().outstanding_stores.remove(&req_id);
        }
    }

    drain_fetch_reorder(pipeline);
}

/// Inserts a completed fetch into the reorder buffer at its `fetch_seq`.
/// Out-of-order arrivals (e.g. an L1I-hit fetch returning before an older
/// fetch that missed and went all the way to DRAM) sit here until every
/// older fetch has completed.
fn buffer_fetch<E: ExecutionEngine>(pipeline: &mut Pipeline<E>, fetch: OutstandingFetch) {
    let common = pipeline.engine.common_mut();
    // Stale-after-flush responses have fetch_seqs below the post-flush
    // emit cursor; drop them rather than reintroducing wrong-path entries.
    if fetch.fetch_seq < common.next_emit_fetch_seq {
        return;
    }
    let _ = common.fetch_reorder.insert(fetch.fetch_seq, fetch);
}

/// Drains every contiguous run of completed fetches from the reorder buffer
/// into the fetch1→fetch2 latch in program order. Stops on the first gap
/// (a still-in-flight older fetch).
fn drain_fetch_reorder<E: ExecutionEngine>(pipeline: &mut Pipeline<E>) {
    loop {
        let next_seq = pipeline.engine.common().next_emit_fetch_seq;
        let Some(fetch) = pipeline.engine.common_mut().fetch_reorder.remove(&next_seq) else {
            break;
        };
        pipeline.engine.common_mut().next_emit_fetch_seq = next_seq.wrapping_add(1);
        complete_fetch(pipeline, fetch);
    }
}

/// Pushes a completed fetch into the frontend's fetch1→fetch2 latch.
fn complete_fetch<E: ExecutionEngine>(pipeline: &mut Pipeline<E>, fetch: OutstandingFetch) {
    pipeline.frontend.fetch1_fetch2.push(Fetch1Fetch2Entry {
        pc: fetch.pc,
        paddr: fetch.paddr,
        pred_taken: fetch.pred_taken,
        pred_target: fetch.pred_target,
        trap: fetch.trap,
        exception_stage: fetch.exception_stage,
        ghr_snapshot: fetch.ghr_snapshot,
        ras_snapshot: fetch.ras_snapshot,
    });
}

/// Reads the load's raw bytes from RAM (fast path) or the device-supplied
/// `MemResp` payload (MMIO) and pushes a `Mem1Mem2Entry` into the M1→M2
/// latch. Memory2 handles sign-extension, AMO RMW, and SB resolution.
fn complete_load<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    cpu: &Cpu,
    load: OutstandingLoad,
    resp_data: &MemRespData,
) {
    let entry = load.entry;
    let paddr = load.paddr;
    let load_raw = read_load_bytes(cpu, paddr.val(), entry.ctrl.width, resp_data);
    let cycle = cpu.cycle;

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
    });
}

/// Advances an in-flight page-table walk. Either completes (triggering the
/// continuation) or issues the next PTE `MemReq`.
fn complete_walk<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    cpu: &mut Cpu,
    walk: OutstandingWalk,
) {
    let raw_pte = read_pte_bytes(cpu, walk.pte_addr);
    let bus_transit = cpu.bus.calculate_transit_time(8);
    let outcome = cpu.translate_continue(walk.state, raw_pte, bus_transit);
    match outcome {
        TranslateResult::Ready(result) => {
            dispatch_walk_continuation(pipeline, cpu, walk.continuation, result);
        }
        TranslateResult::NeedPte { pte_addr, state } => {
            let common = pipeline.engine.common_mut();
            let req_id = common.alloc_req_id();
            let _ = common.outstanding_walks.insert(
                req_id,
                OutstandingWalk { state, pte_addr, continuation: walk.continuation },
            );
            emit_pte_req(pipeline, cpu, req_id, pte_addr);
        }
    }
}

/// Runs the appropriate continuation once a walk reaches Ready.
fn dispatch_walk_continuation<E: ExecutionEngine>(
    pipeline: &mut Pipeline<E>,
    cpu: &mut Cpu,
    continuation: WalkContinuation,
    result: crate::common::TranslationResult,
) {
    match continuation {
        WalkContinuation::Fetch(mut fetch) => {
            pipeline.engine.common_mut().fetch_walk_pending = false;
            if let Some(trap) = result.trap {
                fetch.trap = Some(trap);
                fetch.exception_stage = Some(ExceptionStage::Fetch);
                fetch.paddr = PhysAddr::new(0);
                buffer_fetch(pipeline, fetch);
            } else {
                fetch.paddr = result.paddr;
                let common = pipeline.engine.common_mut();
                let req_id = common.alloc_req_id();
                let paddr = fetch.paddr;
                let pc = fetch.pc;
                let _ = common.outstanding_fetches.insert(req_id, fetch);
                emit_fetch_req(pipeline, cpu, req_id, paddr, VirtAddr::new(pc));
            }
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
fn read_pte_bytes(cpu: &Cpu, pte_addr: PhysAddr) -> u64 {
    let raw = pte_addr.val();
    cpu.bus.ram_region().filter(|r| r.contains(raw, 8)).map_or(0u64, |r| {
        // SAFETY: `RamRegion::contains(raw, 8)` bounds-checks the access.
        unsafe { r.ptr(raw).cast::<u64>().read_unaligned() }
    })
}

/// Reads the raw bytes of a load. RAM accesses use the fast-path pointer;
/// MMIO loads take their data from the device's `MemResp` payload.
fn read_load_bytes(cpu: &Cpu, paddr: u64, width: MemWidth, resp_data: &MemRespData) -> u64 {
    let size = match width {
        MemWidth::Byte => 1u64,
        MemWidth::Half => 2,
        MemWidth::Word => 4,
        MemWidth::Double => 8,
        MemWidth::Nop => 0,
    };
    if size > 0
        && let Some(r) = cpu.bus.ram_region_for(paddr, size)
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
    cpu: &mut Cpu,
    req_id: ReqId,
    pte_addr: PhysAddr,
) {
    let common = pipeline.engine.common();
    let cycle = cpu.cycle;
    cpu.event_queue.schedule(
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

/// Emits an instruction fetch request to the L1 instruction cache.
fn emit_fetch_req<E: ExecutionEngine>(
    pipeline: &Pipeline<E>,
    cpu: &mut Cpu,
    req_id: ReqId,
    paddr: PhysAddr,
    vaddr: VirtAddr,
) {
    let common = pipeline.engine.common();
    let cycle = cpu.cycle;
    cpu.event_queue.schedule(
        cycle,
        ComponentId::Cache(common.l1_i_id),
        ComponentId::Pipeline(common.pipeline_id),
        Packet::MemReq {
            req_id,
            paddr,
            vaddr: Some(vaddr),
            size: AccessSize::B4,
            op: MemOp::Fetch,
        },
    );
}
