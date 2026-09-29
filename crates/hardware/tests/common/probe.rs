//! Synchronous packet probes for device-level unit tests.
//!
//! Devices in the packet-based design react to `MemReq` packets through
//! their [`Handle`] impl rather than exposing direct `read_u8`/`write_u8`
//! methods. These helpers construct a local event queue + `HandleCtx`,
//! dispatch a `MemReq`, drain the resulting `MemResp`, and return the
//! response data. Used by unit tests that exercise CLINT / PLIC / UART /
//! HTIF / SysCon / Goldfish RTC / VirtIO state directly without a full
//! `Simulator`.
//!
//! For tests that need a `Simulator`, prefer `sim.probe_mem_load` /
//! `sim.probe_mem_store` instead — those go through the `Bus` routing
//! layer.

use rvsim_core::common::{LineAddr, PhysAddr};
use rvsim_core::config::Config;
use rvsim_core::sim::components::{ComponentId, DeviceId, PipelineId, ReqId};
use rvsim_core::sim::events::EventQueue;
use rvsim_core::sim::handle::{Handle, HandleCtx};
use rvsim_core::sim::packet::WriteOrigin;
use rvsim_core::sim::packet::{
    AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData,
};
use rvsim_core::sim::stats::Stats;
use rvsim_core::system::state::global_memory::GlobalMemory;

/// Maps a `width_bytes` value (1/2/4/8) to the matching [`AccessSize`].
fn access_size_for(width: u8) -> AccessSize {
    match width {
        1 => AccessSize::B1,
        2 => AccessSize::B2,
        4 => AccessSize::B4,
        _ => AccessSize::B8,
    }
}

/// Dispatches a `MemReq::Read` to `device` and returns the response payload.
pub fn read<H: Handle>(device: &mut H, paddr: PhysAddr, width: u8) -> u64 {
    let req_id = ReqId::new(u64::MAX);
    let mut queue = EventQueue::new();
    let mut stats = Stats::new();
    let mut memory = GlobalMemory::new(None, 1, 64);
    let config = Config::default();
    let mut ctx = HandleCtx {
        scheduler: &mut queue,
        stats: &mut stats,
        memory: &mut memory,
        config: &config,
        cycle: 0,
        self_id: ComponentId::Device(DeviceId::new(0)),
    };
    device.handle(
        Packet::MemReq {
            req_id,
            paddr,
            vaddr: None,
            size: access_size_for(width),
            op: MemOp::Read,
        },
        ComponentId::Pipeline(PipelineId::new(0)),
        &mut ctx,
    );
    while let Some(event) = queue.pop_ready(u64::MAX) {
        if let Packet::MemResp { req_id: rid, data, .. } = event.packet
            && rid == req_id
        {
            return match data {
                MemRespData::Small(value) | MemRespData::Performed { value, .. } => value,
                MemRespData::Line(_) | MemRespData::PerformedBytes { .. } => 0,
            };
        }
    }
    0
}

/// Dispatches a `MemReq::Write` to `device` for its side effect.
pub fn write<H: Handle>(device: &mut H, paddr: PhysAddr, value: u64, width: u8) {
    let req_id = ReqId::new(u64::MAX);
    let mut queue = EventQueue::new();
    let mut stats = Stats::new();
    let mut memory = GlobalMemory::new(None, 1, 64);
    let config = Config::default();
    let mut ctx = HandleCtx {
        scheduler: &mut queue,
        stats: &mut stats,
        memory: &mut memory,
        config: &config,
        cycle: 0,
        self_id: ComponentId::Device(DeviceId::new(0)),
    };
    device.handle(
        Packet::MemReq {
            req_id,
            paddr,
            vaddr: None,
            size: access_size_for(width),
            op: MemOp::Write { data: WriteData::Small(value), origin: WriteOrigin::Host },
        },
        ComponentId::Pipeline(PipelineId::new(0)),
        &mut ctx,
    );
    // Drain the ack so the local queue doesn't leak it (the side effect
    // already fired inside `device.handle`).
    let _ = queue.pop_ready(u64::MAX);
}

/// Dispatches a `MemReq::Write` to `device`, then answers every DMA request
/// it puts on the bus until it has none left, so a request the write
/// notified runs to completion.
pub fn write_and_run_dma<H: Handle>(device: &mut H, paddr: PhysAddr, value: u64, width: u8) {
    let mut queue = EventQueue::new();
    let mut stats = Stats::new();
    let mut memory = GlobalMemory::new(None, 1, 64);
    let config = Config::default();
    let mut cycle = 0;
    let mut pending = vec![Packet::MemReq {
        req_id: ReqId::new(u64::MAX),
        paddr,
        vaddr: None,
        size: access_size_for(width),
        op: MemOp::Write { data: WriteData::Small(value), origin: WriteOrigin::Host },
    }];
    while !pending.is_empty() {
        for packet in std::mem::take(&mut pending) {
            let mut ctx = HandleCtx {
                scheduler: &mut queue,
                stats: &mut stats,
                memory: &mut memory,
                config: &config,
                cycle,
                self_id: ComponentId::Device(DeviceId::new(0)),
            };
            device.handle(packet, ComponentId::Bus, &mut ctx);
        }
        cycle += 1;
        while let Some(event) = queue.pop_ready(u64::MAX) {
            if let (ComponentId::Bus, Packet::MemReq { req_id, paddr, .. }) =
                (event.target, event.packet)
            {
                pending.push(Packet::MemResp {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, 64),
                    data: MemRespData::Small(0),
                    hit_level: HitLevel::Dram,
                    state: MesiState::Exclusive,
                });
            }
        }
    }
}
