//! The bus's channels, each beat a bus clock: with AXI's five a read queues
//! only behind reads and a write's data only behind writes; with one
//! channel each way a write's data holds up the next read, as on `TileLink`;
//! and a slower bus clock stretches every beat.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use crate::common::{HartId, LineAddr, PhysAddr};
use crate::config::{BusChannels, Config};
use crate::sim::components::{ComponentId, MemCtrlId, PipelineId, ReqId};
use crate::sim::events::EventQueue;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::memory::GlobalMemory;
use crate::sim::packet::{
    AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData, WriteOrigin,
};
use crate::sim::stats::Stats;
use crate::soc::bus::Bus;

const RAM_BASE: u64 = 0x8000_0000;
const LINE: usize = 64;

/// An AXI bus `width` bytes wide with no latency, a beat every `divider`
/// core cycles, in front of RAM.
fn bus(width: u64, divider: u64) -> Bus {
    bus_with(width, divider, BusChannels::Axi)
}

fn bus_with(width: u64, divider: u64, channels: BusChannels) -> Bus {
    let mut bus = Bus::new(width, 0, NonZeroU64::new(divider).unwrap(), channels, 1);
    bus.attach_ram(MemCtrlId::new(0), PhysAddr::new(RAM_BASE), 1 << 20);
    bus
}

fn line_read(req: u64) -> Packet {
    Packet::MemReq {
        req_id: ReqId::new(req),
        paddr: PhysAddr::new(RAM_BASE + req * LINE as u64),
        vaddr: None,
        pc: None,
        size: AccessSize::Line,
        op: MemOp::Read,
    }
}

fn line_write(req: u64) -> Packet {
    Packet::MemReq {
        req_id: ReqId::new(req),
        paddr: PhysAddr::new(RAM_BASE + req * LINE as u64),
        vaddr: None,
        pc: None,
        size: AccessSize::Line,
        op: MemOp::Write {
            data: WriteData::Line { bytes: vec![0; LINE].into(), mask: u64::MAX },
            origin: WriteOrigin::Hart(HartId::new(0)),
        },
    }
}

/// Drives packets through a bus at cycle 0 and reports when each crosses.
struct Crossing {
    bus: Bus,
    queue: EventQueue,
    stats: Stats,
    memory: GlobalMemory,
    config: Config,
}

impl Crossing {
    fn new(bus: Bus) -> Self {
        Self {
            bus,
            queue: EventQueue::new(),
            stats: Stats::new(),
            memory: GlobalMemory::new(None, 1, 64),
            config: Config::minimal(),
        }
    }

    /// Hands `packet` to the bus at cycle 0 from the hart's side.
    fn send(&mut self, packet: Packet) {
        let mut ctx = HandleCtx {
            scheduler: &mut self.queue,
            stats: &mut self.stats,
            memory: &mut self.memory,
            config: &self.config,
            cycle: 0,
            self_id: ComponentId::Bus,
        };
        self.bus.handle(packet, ComponentId::Pipeline(PipelineId::new(0)), &mut ctx);
    }

    /// Answers request `req` from memory at cycle 0.
    fn answer(&mut self, req: u64) {
        let mut ctx = HandleCtx {
            scheduler: &mut self.queue,
            stats: &mut self.stats,
            memory: &mut self.memory,
            config: &self.config,
            cycle: 0,
            self_id: ComponentId::Bus,
        };
        let answer = Packet::MemResp {
            req_id: ReqId::new(req),
            line_addr: LineAddr::from_phys(PhysAddr::new(RAM_BASE + req * LINE as u64), 64),
            data: MemRespData::Small(0),
            hit_level: HitLevel::Dram,
            state: MesiState::Invalid,
        };
        self.bus.handle(answer, ComponentId::MemCtrl(MemCtrlId::new(0)), &mut ctx);
    }

    /// When each request's packet for `target` arrives.
    fn arrivals(&mut self, target: ComponentId) -> BTreeMap<ReqId, u64> {
        let mut arrivals = BTreeMap::new();
        while let Some(event) = self.queue.pop_ready(u64::MAX) {
            let (Packet::MemReq { req_id, .. } | Packet::MemResp { req_id, .. }) = event.packet
            else {
                continue;
            };
            if event.target == target {
                let _ = arrivals.insert(req_id, event.fire_at);
            }
        }
        arrivals
    }
}

#[test]
fn a_reads_data_does_not_wait_for_a_writes() {
    let mut crossing = Crossing::new(bus(8, 1));
    crossing.send(line_write(0));
    crossing.send(line_read(1));
    let _ = crossing.arrivals(ComponentId::MemCtrl(MemCtrlId::new(0)));

    crossing.answer(1);
    crossing.answer(0);

    let back = crossing.arrivals(ComponentId::Pipeline(PipelineId::new(0)));
    assert_eq!(back[&ReqId::new(1)], 8, "the read's eight data beats");
    assert_eq!(back[&ReqId::new(0)], 1, "the write's one-beat response beside them");
}

#[test]
fn requests_queue_only_behind_their_own_kind() {
    let mut crossing = Crossing::new(bus(8, 1));
    crossing.send(line_write(0));
    crossing.send(line_write(1));
    crossing.send(line_read(2));

    let out = crossing.arrivals(ComponentId::MemCtrl(MemCtrlId::new(0)));

    assert_eq!((out[&ReqId::new(0)], out[&ReqId::new(1)]), (8, 16), "the writes' data in turn");
    assert_eq!(out[&ReqId::new(2)], 1, "the read's address beside them");
}

#[test]
fn a_slower_bus_clock_stretches_every_beat() {
    let mut crossing = Crossing::new(bus(16, 8));
    crossing.send(line_read(0));
    let _ = crossing.arrivals(ComponentId::MemCtrl(MemCtrlId::new(0)));

    crossing.answer(0);

    let back = crossing.arrivals(ComponentId::Pipeline(PipelineId::new(0)));
    assert_eq!(back[&ReqId::new(0)], 4 * 8, "four 16-byte beats of eight core cycles each");
}

#[test]
fn on_one_channel_each_way_a_writes_data_holds_up_the_next_read() {
    let mut crossing = Crossing::new(bus_with(8, 1, BusChannels::RequestResponse));
    crossing.send(line_write(0));
    crossing.send(line_read(1));

    let out = crossing.arrivals(ComponentId::MemCtrl(MemCtrlId::new(0)));

    assert_eq!(out[&ReqId::new(0)], 9, "an 8-byte address and 64 bytes of data");
    assert_eq!(out[&ReqId::new(1)], 10, "the read's address after them");
}
