//! Shared helpers for the DDR5 controller tests.

use std::sync::Arc;

use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::sim::components::{ComponentId, MemCtrlId, PipelineId, ReqId};
use rvsim_core::sim::events::EventQueue;
use rvsim_core::sim::handle::{Handle, HandleCtx};
use rvsim_core::sim::packet::{AccessSize, DramCmdKind, MemOp, Packet, WriteData};
use rvsim_core::sim::stats::Stats;
use rvsim_core::soc::memory::buffer::DramBuffer;
use rvsim_core::soc::memory::controller::MemoryController;
use rvsim_core::soc::memory::ddr5::{Ddr5Config, Ddr5Controller};

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

/// Ddr5Controller wrapped with a large DRAM buffer.
pub fn make_controller(config: Ddr5Config) -> Ddr5Controller {
    let buffer = Arc::new(DramBuffer::new(1 << 24));
    Ddr5Controller::new(buffer, PhysAddr::new(0), config, MemCtrlId::new(0))
}

/// Encodes a physical address for the tiny topology.
///
/// Bit layout (RoRaBaChCo, low→high above line offset 6):
/// column | subchannel | channel | bank | bank_group | rank | row.
///
/// `column` addresses within a row are in cache-line units (each column
/// occupies `1 << 6` bytes).
pub fn addr_from(
    cfg: &Ddr5Config,
    rank: u8,
    bg: u8,
    bank: u8,
    row: u32,
    column: u32,
) -> u64 {
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
pub struct Harness {
    pub controller: Ddr5Controller,
    pub queue: EventQueue,
    pub stats: Stats,
    pub config: Config,
    pub next_req_id: u64,
    /// Current simulator cycle from the harness's POV. Advanced monotonically
    /// by [`Self::issue`] (to the request cycle) and by [`Self::response_at`]
    /// (as ticks progress until the response fires).
    pub sim_cycle: u64,
}

impl Harness {
    pub fn new(cfg: Ddr5Config) -> Self {
        Self {
            controller: make_controller(cfg),
            queue: EventQueue::new(),
            stats: Stats::new(),
            config: Config::default(),
            next_req_id: 0,
            sim_cycle: 0,
        }
    }

    fn make_ctx<'s>(
        queue: &'s mut EventQueue,
        stats: &'s mut Stats,
        config: &'s Config,
        cycle: u64,
    ) -> HandleCtx<'s> {
        HandleCtx {
            scheduler: queue,
            stats,
            config,
            cycle,
            self_id: ComponentId::MemCtrl(MemCtrlId::new(0)),
        }
    }

    fn tick_at(&mut self, cycle: u64) {
        let mut ctx = Self::make_ctx(&mut self.queue, &mut self.stats, &self.config, cycle);
        self.controller.tick(&mut ctx);
    }

    /// Injects one request at `cycle` and returns the ReqId assigned. Ticks
    /// the controller forward to `cycle` first so previously-queued work
    /// resolves before the new arrival is visible.
    pub fn issue(&mut self, paddr: u64, cycle: u64, op: MemOp) -> ReqId {
        assert!(
            cycle >= self.sim_cycle,
            "issue cycle {cycle} is in the past; sim_cycle={}",
            self.sim_cycle
        );
        while self.sim_cycle < cycle {
            self.sim_cycle += 1;
            self.tick_at(self.sim_cycle);
        }
        let req_id = ReqId::new(self.next_req_id);
        self.next_req_id += 1;
        let mut ctx =
            Self::make_ctx(&mut self.queue, &mut self.stats, &self.config, self.sim_cycle);
        self.controller.handle(
            Packet::MemReq {
                req_id,
                paddr: PhysAddr::new(paddr),
                vaddr: None,
                size: AccessSize::B8,
                op,
            },
            ComponentId::Pipeline(PipelineId::new(0)),
            &mut ctx,
        );
        req_id
    }

    /// Ticks the controller forward until a `MemResp` for `req_id` appears
    /// on the queue. Returns the response's `fire_at` cycle. Non-matching
    /// events are left on the queue for subsequent inspection.
    pub fn response_at(&mut self, req_id: ReqId) -> u64 {
        // First trigger a tick at the current cycle so requests queued at
        // this exact cycle get a chance to schedule commands.
        self.tick_at(self.sim_cycle);
        loop {
            if let Some(fire_at) = self.take_response(req_id) {
                self.sim_cycle = self.sim_cycle.max(fire_at);
                return fire_at;
            }
            self.sim_cycle += 1;
            self.tick_at(self.sim_cycle);
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

    /// Collects every DramCmd event currently pending on the queue, in
    /// scheduling order. Leaves other events (MemResp, ...) in place.
    pub fn take_dram_cmds(&mut self) -> Vec<CommandRecord> {
        let mut retained = Vec::new();
        let mut cmds = Vec::new();
        while let Some(event) = self.queue.pop_ready(u64::MAX) {
            match event.packet.clone() {
                Packet::DramCmd { channel, rank, bank, kind, row } => {
                    cmds.push(CommandRecord {
                        fire_at: event.fire_at,
                        channel,
                        rank,
                        bank,
                        kind,
                        row,
                    });
                }
                _ => retained.push(event),
            }
        }
        for evt in retained {
            self.queue.schedule(evt.fire_at, evt.target, evt.source, evt.packet);
        }
        cmds
    }
}

#[derive(Copy, Clone, Debug)]
pub struct CommandRecord {
    pub fire_at: u64,
    pub channel: u8,
    pub rank: u8,
    pub bank: u8,
    pub kind: DramCmdKind,
    pub row: u32,
}

pub fn write_op() -> MemOp {
    MemOp::Write { data: WriteData::Small(0) }
}

pub fn read_op() -> MemOp {
    MemOp::Read
}
