//! Shared helpers for the DDR5 controller tests.

use crate::common::{HartId, PhysAddr};
use crate::sim::packet::WriteOrigin;

use crate::config::Config;
use crate::config::ddr5::Ddr5Config;
use crate::sim::components::{ComponentId, MemCtrlId, PipelineId, ReqId};
use crate::sim::events::EventQueue;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::memory::GlobalMemory;
use crate::sim::packet::{AccessSize, DramCmdKind, MemOp, Packet, WriteData};
use crate::sim::stats::Stats;
use crate::soc::memory::controller::MemoryController;
use crate::soc::memory::ddr5::Ddr5Controller;

/// Core clock that makes one simulator cycle equal one DDR5-4800 command
/// clock, so test expectations can be written directly in DRAM clocks.
pub const ONE_TO_ONE_CPU_MHZ: u64 = 2400;

/// Small canonical topology used across the timing tests: 1 channel,
/// 1 sub-channel, 1 rank, 2 bank groups × 2 banks, 4-bit rows, 4-bit
/// columns. Keeps addresses small and the interleave predictable.
pub fn tiny_config() -> Ddr5Config {
    Ddr5Config {
        channels: 1,
        subchannels_per_channel: 1,
        ranks_per_channel: 1,
        bank_groups_per_rank: 2,
        banks_per_group: 2,
        row_bits: 4,
        column_bits: 4,
        ..Ddr5Config::default()
    }
}

/// Two-rank version of [`tiny_config`], for tRTRS and refresh isolation tests.
pub fn tworank_config() -> Ddr5Config {
    let mut cfg = tiny_config();
    cfg.ranks_per_channel = 2;
    cfg
}

/// Fixed controller pipeline latency every read pays on top of the DRAM
/// access.
pub fn controller_latency(cfg: &Ddr5Config) -> u64 {
    cfg.frontend_latency + cfg.backend_latency
}

/// Ddr5Controller over 16 MiB of RAM, clocked 1:1.
pub fn make_controller(config: Ddr5Config) -> Ddr5Controller {
    make_controller_with_clock(config, ONE_TO_ONE_CPU_MHZ)
}

/// Ddr5Controller over 16 MiB of RAM at a given core clock.
pub fn make_controller_with_clock(config: Ddr5Config, cpu_mhz: u64) -> Ddr5Controller {
    Ddr5Controller::new(PhysAddr::new(0), 1 << 24, &config, MemCtrlId::new(0), cpu_mhz)
}

/// Encodes a physical address for the tiny topology.
///
/// Bit layout (RoRaBaChCo, low→high above line offset 6):
/// column | subchannel | channel | bank | bank_group | rank | row.
///
/// `column` addresses within a row are in cache-line units (each column
/// occupies `1 << 6` bytes).
pub fn addr_from(cfg: &Ddr5Config, rank: u8, bg: u8, bank: u8, row: u32, column: u32) -> u64 {
    let line_bits = 6u32;
    let col_bits = u32::from(cfg.column_bits);
    let subch_bits = cfg.subchannels_per_channel.trailing_zeros();
    let chan_bits = cfg.channels.trailing_zeros();
    let bank_bits = cfg.banks_per_group.trailing_zeros();
    let bg_bits = cfg.bank_groups_per_rank.trailing_zeros();
    let rank_bits = cfg.ranks_per_channel.trailing_zeros();
    let subch_shift = line_bits + col_bits;
    let chan_shift = subch_shift + subch_bits;
    let bank_shift = chan_shift + chan_bits;
    let bg_shift = bank_shift + bank_bits;
    let rank_shift = bg_shift + bg_bits;
    let row_shift = rank_shift + rank_bits;

    (u64::from(column) << line_bits)
        | (u64::from(bank) << bank_shift)
        | (u64::from(bg) << bg_shift)
        | (u64::from(rank) << rank_shift)
        | (u64::from(row) << row_shift)
}

/// Wraps `Ddr5Controller` so tests can issue a request and read the resulting
/// event trace back out.
///
/// Mirrors the simulator's ordering within a cycle: packets are delivered to
/// the controller before its tick for that cycle runs.
pub struct Harness {
    pub controller: Ddr5Controller,
    pub queue: EventQueue,
    pub stats: Stats,
    pub memory: GlobalMemory,
    pub config: Config,
    pub next_req_id: u64,
    /// Next simulator cycle whose tick has not run yet.
    pub next_tick: u64,
}

impl Harness {
    pub fn new(cfg: Ddr5Config) -> Self {
        Self::with_controller(make_controller(cfg))
    }

    pub fn with_controller(controller: Ddr5Controller) -> Self {
        Self {
            controller,
            queue: EventQueue::new(),
            stats: Stats::new(),
            memory: GlobalMemory::new(None, 1, 64),
            config: Config::default(),
            next_req_id: 0,
            next_tick: 0,
        }
    }

    fn make_ctx<'s>(
        queue: &'s mut EventQueue,
        stats: &'s mut Stats,
        memory: &'s mut GlobalMemory,
        config: &'s Config,
        cycle: u64,
    ) -> HandleCtx<'s> {
        HandleCtx {
            scheduler: queue,
            stats,
            memory,
            config,
            cycle,
            self_id: ComponentId::MemCtrl(MemCtrlId::new(0)),
        }
    }

    fn tick_at(&mut self, cycle: u64) {
        let mut ctx =
            Self::make_ctx(&mut self.queue, &mut self.stats, &mut self.memory, &self.config, cycle);
        self.controller.tick(&mut ctx);
    }

    /// Runs every tick before `cycle` so earlier work has resolved.
    fn advance_to(&mut self, cycle: u64) {
        assert!(
            cycle >= self.next_tick,
            "cycle {cycle} is in the past; next tick is {}",
            self.next_tick
        );
        while self.next_tick < cycle {
            let tick = self.next_tick;
            self.tick_at(tick);
            self.next_tick += 1;
        }
    }

    /// Delivers one request at `cycle` and returns the ReqId assigned. The
    /// controller's tick for `cycle` runs on the next `response_at` /
    /// `run_until`.
    pub fn issue(&mut self, paddr: u64, cycle: u64, op: MemOp) -> ReqId {
        self.issue_sized(paddr, cycle, op, AccessSize::B8)
    }

    pub fn issue_sized(&mut self, paddr: u64, cycle: u64, op: MemOp, size: AccessSize) -> ReqId {
        self.advance_to(cycle);
        let req_id = ReqId::new(self.next_req_id);
        self.next_req_id += 1;
        let mut ctx =
            Self::make_ctx(&mut self.queue, &mut self.stats, &mut self.memory, &self.config, cycle);
        self.controller.handle(
            Packet::MemReq { req_id, paddr: PhysAddr::new(paddr), vaddr: None, size, op },
            ComponentId::Pipeline(PipelineId::new(0)),
            &mut ctx,
        );
        req_id
    }

    /// Ticks the controller until a `MemResp` for `req_id` appears on the
    /// queue. Returns the response's `fire_at` cycle. Non-matching events
    /// are left on the queue for subsequent inspection.
    pub fn response_at(&mut self, req_id: ReqId) -> u64 {
        loop {
            if let Some(fire_at) = self.take_response(req_id) {
                return fire_at;
            }
            let tick = self.next_tick;
            self.tick_at(tick);
            self.next_tick += 1;
            assert!(self.next_tick < 1_000_000, "no response for {req_id:?}");
        }
    }

    /// Ticks the controller through `cycle` inclusive; a no-op if the
    /// harness has already passed it.
    pub fn run_until(&mut self, cycle: u64) {
        if cycle + 1 > self.next_tick {
            self.advance_to(cycle + 1);
        }
    }

    /// Pops a `MemResp` for `req_id` from the queue if present, returning its
    /// `fire_at`. All other events are re-scheduled.
    fn take_response(&mut self, req_id: ReqId) -> Option<u64> {
        let mut retained = Vec::new();
        let mut found: Option<u64> = None;
        while let Some(event) = self.queue.pop_ready(u64::MAX) {
            match event.packet {
                Packet::MemResp { req_id: rid, .. } if rid == req_id => {
                    found = Some(event.fire_at);
                    break;
                }
                _ => retained.push(event),
            }
        }
        for evt in retained {
            self.queue.schedule(evt.fire_at, evt.target, evt.source, evt.packet);
        }
        found
    }

    /// Every DramCmd event the controller has emitted so far, in issue
    /// order. Leaves the event queue untouched.
    pub fn dram_cmds(&mut self) -> Vec<CommandRecord> {
        let mut retained = Vec::new();
        let mut cmds = Vec::new();
        while let Some(event) = self.queue.pop_ready(u64::MAX) {
            if let Packet::DramCmd { bank, kind, row, .. } = event.packet {
                cmds.push(CommandRecord { fire_at: event.fire_at, bank, kind, row });
            }
            retained.push(event);
        }
        for evt in retained {
            self.queue.schedule(evt.fire_at, evt.target, evt.source, evt.packet);
        }
        cmds.sort_by_key(|c| c.fire_at);
        cmds
    }

    /// DramCmd events of one kind, in issue order.
    pub fn commands_of(&mut self, kind: DramCmdKind) -> Vec<CommandRecord> {
        self.dram_cmds().into_iter().filter(|c| c.kind == kind).collect()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct CommandRecord {
    pub fire_at: u64,
    pub bank: u8,
    pub kind: DramCmdKind,
    pub row: u32,
}

pub fn write_op() -> MemOp {
    MemOp::Write { data: WriteData::Small(0), origin: WriteOrigin::Hart(HartId::new(0)) }
}

pub fn read_op() -> MemOp {
    MemOp::Read
}
