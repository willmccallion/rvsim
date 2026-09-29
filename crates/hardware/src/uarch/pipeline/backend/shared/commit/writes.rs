//! Draining committed writes: scalar stores, cache-block operations,
//! write-combining lines and vector store lines, to memory.

use super::{is_pure_ram, older_stores_pending};
use crate::common::{PhysAddr, crosses_cache_line};
use crate::exec::cbo::CboEffect;
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::op::MemWidth;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::packet::{AccessSize, Maintenance, MemOp, Packet, WriteData, WriteOrigin};
use crate::system::CoreCtx;
use crate::trace_commit;
use crate::uarch::pipeline::engine::BackendCommon;
use crate::uarch::pipeline::lsq::store_buffer::{StoreBuffer, StoreData, width_to_bytes};
use crate::uarch::pipeline::lsq::vec_store_buffer::{VSB_LINE_BYTES, VecStoreBuffer};
use crate::uarch::pipeline::lsq::write_buffer::WcbLine;
use crate::uarch::pipeline::outstanding::{OutstandingStore, StoreOwner};

/// Sends the write of the oldest committed store not yet sent. The entry
/// keeps its slot until the write is acknowledged. Returns true if a store
/// was taken (so the caller does not also drain the vec-store buffer this
/// cycle).
pub(super) fn try_drain_one_store(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    store_buffer: &mut StoreBuffer,
) -> bool {
    let Some(write) = store_buffer.begin_write() else { return false };
    let requests = match write.data {
        StoreData::Bytes(data) => send_data_store(state, common, write.paddr, data, write.width),
        StoreData::Block(effect) => {
            // A CBO follows the older stores the WCB holds for its block.
            if state.core.wcb.request_send(write.paddr, CBOZ_BLOCK_SIZE as usize) {
                return false;
            }
            vec![send_block_op(state, common, write.paddr, effect)]
        }
    };
    store_buffer.issue_write(write, &requests);
    true
}

/// Sends a committed store's data: into the WCB for RAM when it is
/// enabled, else as its own writes. Returns the writes the store buffer
/// waits for.
pub(super) fn send_data_store(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
) -> Vec<ReqId> {
    // MMIO, including the HTIF window over RAM, bypasses the WCB so its
    // device sees each store.
    let via_wcb = !state.core.wcb.is_disabled() && is_pure_ram(state, paddr, width);
    trace_commit!(state.config.general.trace_instructions;
        paddr      = %crate::common::trace::Hex(paddr.val()),
        data       = %crate::common::trace::Hex(data),
        width      = ?width,
        via_wcb    = via_wcb,
        "CM: committed store's write sent to memory"
    );
    if !via_wcb {
        let hart = WriteOrigin::Hart(state.hart.hart_id);
        let store = StoreWrite { origin: hart, owner: StoreOwner::StoreBuffer };
        return emit_store_write_packet(state, common, paddr, data, width, store);
    }
    let span = state.core.wcb.entry_bytes();
    for (part_paddr, part_data, part_bytes) in span_parts(paddr, data, width_to_bytes(width), span)
    {
        match state.core.wcb.merge_store(part_paddr, part_data, part_bytes) {
            Some(evicted) => send_wcb_line(state, common, &evicted),
            None => state.uncore.stats.counter(state.core.stat_paths.wcb.coalesces).inc(),
        }
    }
    Vec::new()
}

/// Sends a committed CBO to the L1D: `cbo.zero` as the hart's write of a
/// zeroed block, the others as a maintenance operation carried to memory.
pub(super) fn send_block_op(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    block: PhysAddr,
    effect: CboEffect,
) -> ReqId {
    let maintain = |op| MemOp::Maintain { op, dirty: false };
    let op = match effect {
        CboEffect::Zero => MemOp::Write {
            data: WriteData::Line {
                bytes: vec![0; CBOZ_BLOCK_SIZE as usize].into(),
                mask: u64::MAX,
            },
            origin: WriteOrigin::Hart(state.hart.hart_id),
        },
        CboEffect::Clean => maintain(Maintenance::Clean),
        CboEffect::Flush => maintain(Maintenance::Flush),
        CboEffect::Invalidate => maintain(Maintenance::Invalidate),
    };
    let req_id = common.alloc_req_id();
    let _ = common
        .outstanding_stores
        .insert(req_id, OutstandingStore { owner: StoreOwner::StoreBuffer, paddr: block });
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        ComponentId::Cache(common.l1_d_id),
        ComponentId::Pipeline(common.pipeline_id),
        Packet::MemReq { req_id, paddr: block, vaddr: None, size: AccessSize::Line, op },
    );
    req_id
}

/// Sends this cycle's write to the L1D: a line the WCB must send now, else
/// the oldest committed scalar store, else a vector store's line, else, the
/// port being idle, the WCB's oldest line.
pub fn send_one_write(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    store_buffer: &mut StoreBuffer,
    vec_store_buffer: &mut VecStoreBuffer,
) {
    if !send_urgent_wcb_line(state, common)
        && !try_drain_one_store(state, common, store_buffer)
        && !drain_vec_store_line(state, common, vec_store_buffer)
    {
        send_oldest_wcb_line(state, common);
    }
}

/// True while a committed store has yet to finish writing.
pub fn committed_writes_pending(
    state: &CoreCtx<'_>,
    store_buffer: &StoreBuffer,
    vec_store_buffer: &VecStoreBuffer,
) -> bool {
    older_stores_pending(store_buffer, vec_store_buffer, &state.core.wcb)
}

/// Sends a line the WCB must write now. Returns whether one went.
pub(super) fn send_urgent_wcb_line(state: &mut CoreCtx<'_>, common: &mut BackendCommon) -> bool {
    let Some(line) = state.core.wcb.take_urgent() else { return false };
    send_wcb_line(state, common, &line);
    true
}

/// Sends the WCB's least recently merged line, if it holds one.
pub(super) fn send_oldest_wcb_line(state: &mut CoreCtx<'_>, common: &mut BackendCommon) {
    if let Some(line) = state.core.wcb.take_oldest() {
        send_wcb_line(state, common, &line);
    }
}

/// Writes a WCB line to the L1D as the hart's store, taking effect where
/// the cache serves it.
pub(super) fn send_wcb_line(state: &mut CoreCtx<'_>, common: &mut BackendCommon, line: &WcbLine) {
    let span = state.core.wcb.entry_bytes();
    let paddr = PhysAddr::new(line.line_addr);
    let req_id = emit_line_write(
        state,
        common,
        paddr,
        &line.data[..span],
        line.mask,
        StoreOwner::WriteCombining,
    );
    state.core.wcb.sent(req_id, line.clone());
    state.uncore.stats.counter(state.core.stat_paths.wcb.drains).inc();
}

/// Writes the next line of a committed vector store: to RAM as one masked
/// line write, and to a device as the naturally aligned writes it must see.
/// Returns whether a line went.
pub(super) fn drain_vec_store_line(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    vec_store_buffer: &mut VecStoreBuffer,
) -> bool {
    let Some((rob_tag, line)) = vec_store_buffer.take_drainable_line() else { return false };
    let paddr = PhysAddr::new(line.line_addr);
    let owner = StoreOwner::VecStoreBuffer;
    let requests = if state.bus.ram_region_for(paddr.val(), VSB_LINE_BYTES as u64).is_some() {
        vec![emit_line_write(state, common, paddr, &line.data, line.valid_mask, owner)]
    } else {
        let write = StoreWrite { origin: WriteOrigin::Hart(state.hart.hart_id), owner };
        line.natural_writes()
            .into_iter()
            .flat_map(|(paddr, data, width)| {
                emit_store_write_packet(state, common, paddr, data, width, write)
            })
            .collect()
    };
    vec_store_buffer.line_sent(rob_tag, requests);
    true
}

/// Sends the hart's write of the bytes of the line at `line` that `mask`
/// selects to the L1D. Returns the request.
pub(super) fn emit_line_write(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    line: PhysAddr,
    bytes: &[u8],
    mask: u64,
    owner: StoreOwner,
) -> ReqId {
    let req_id = common.alloc_req_id();
    let _ = common.outstanding_stores.insert(req_id, OutstandingStore { owner, paddr: line });
    let origin = WriteOrigin::Hart(state.hart.hart_id);
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        ComponentId::Cache(common.l1_d_id),
        ComponentId::Pipeline(common.pipeline_id),
        Packet::MemReq {
            req_id,
            paddr: line,
            vaddr: None,
            size: AccessSize::Line,
            op: MemOp::Write { data: WriteData::Line { bytes: bytes.into(), mask }, origin },
        },
    );
    req_id
}

/// Writes a store's bytes to RAM at once and emits its `MemReq`s for their
/// timing only: for writes the hardware makes at a single instant (a
/// page-table D-bit update, `cbo.zero`) and for a checkpoint drain, which
/// does not wait for the memory system. An MMIO address is left to the
/// device the packet reaches.
pub(super) fn write_store_to_memory(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
    owner: StoreOwner,
) -> Vec<ReqId> {
    if width == MemWidth::Nop {
        return Vec::new();
    }
    state.publish_write(paddr, data, width);
    emit_store_write_packet(state, common, paddr, data, width, StoreWrite::placed(owner))
}

/// Emits the `MemReq`s (op = Write) carrying a store, one per cache line a
/// RAM store touches. Returns the requests.
pub(super) fn emit_store_write_packet(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    width: MemWidth,
    write: StoreWrite,
) -> Vec<ReqId> {
    if width == MemWidth::Nop {
        return Vec::new();
    }
    let width_bytes = width.bytes() as usize;
    if !is_pure_ram(state, paddr, width) {
        let target = ComponentId::Bus;
        return vec![emit_store_write_packet_to(
            state,
            common,
            paddr,
            data,
            width_bytes,
            write,
            target,
        )];
    }
    let l1d = ComponentId::Cache(common.l1_d_id);
    line_parts(state, paddr, data, width_bytes)
        .into_iter()
        .map(|(part_paddr, part_data, part_bytes)| {
            emit_store_write_packet_to(state, common, part_paddr, part_data, part_bytes, write, l1d)
        })
        .collect()
}

/// Whose write a store's requests carry and the buffer their
/// acknowledgements go back to.
#[derive(Clone, Copy)]
pub(super) struct StoreWrite {
    origin: WriteOrigin,
    owner: StoreOwner,
}

impl StoreWrite {
    /// A write whose bytes are already in RAM.
    const fn placed(owner: StoreOwner) -> Self {
        Self { origin: WriteOrigin::Placed, owner }
    }
}

/// The byte ranges of a store's data that fall in each cache line it
/// touches: `(address, data shifted to start at that address, bytes)`.
pub(super) fn line_parts(
    state: &CoreCtx<'_>,
    paddr: PhysAddr,
    data: u64,
    width_bytes: usize,
) -> Vec<(PhysAddr, u64, usize)> {
    span_parts(paddr, data, width_bytes, state.core.l1_d_cache.line_bytes())
}

/// The byte ranges of a store's data that fall in each aligned `span`-byte
/// block it touches: `(address, data shifted to start there, bytes)`.
pub(super) fn span_parts(
    paddr: PhysAddr,
    data: u64,
    width_bytes: usize,
    span: usize,
) -> Vec<(PhysAddr, u64, usize)> {
    let span = span as u64;
    if !crosses_cache_line(paddr.val(), width_bytes as u64, span) {
        return vec![(paddr, data, width_bytes)];
    }
    let second = (paddr.val() | (span - 1)) + 1;
    let first_bytes = (second - paddr.val()) as usize;
    vec![
        (paddr, data, first_bytes),
        (PhysAddr::new(second), data >> (8 * first_bytes), width_bytes - first_bytes),
    ]
}

/// Emits one `MemReq` (op = Write) of `bytes` bytes to `target` and returns
/// its request.
pub(super) fn emit_store_write_packet_to(
    state: &mut CoreCtx<'_>,
    common: &mut BackendCommon,
    paddr: PhysAddr,
    data: u64,
    bytes: usize,
    write: StoreWrite,
    target: ComponentId,
) -> ReqId {
    let req_id = common.alloc_req_id();
    let pipeline_id = common.pipeline_id;
    let _ =
        common.outstanding_stores.insert(req_id, OutstandingStore { owner: write.owner, paddr });
    let cycle = state.cycle;
    state.event_queue.schedule(
        cycle,
        target,
        ComponentId::Pipeline(pipeline_id),
        Packet::MemReq {
            req_id,
            paddr,
            vaddr: None,
            size: AccessSize::of_bytes(bytes),
            op: MemOp::Write { data: WriteData::Small(data), origin: write.origin },
        },
    );
    req_id
}
