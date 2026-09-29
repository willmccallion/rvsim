//! DDR5 memory controller with per-bank command state machines.
//!
//! [`Ddr5Controller`] implements the [`MemoryController`] trait and services
//! `MemReq` packets by translating each request into a JEDEC-compliant
//! sequence of ACTIVATE / PRECHARGE / READ / WRITE / REFRESH commands. Every
//! command respects the DDR5 timing constants declared in
//! [`crate::config::ddr5::timing::Ddr5Timing`]. In debug builds each
//! command issue is guarded by `debug_assert!` calls that name the constraint
//! being enforced; release builds compile these checks out.
//!
//! The controller runs in the DRAM command-clock domain. Requests arrive
//! stamped with the simulator (core) cycle and are converted through a
//! [`ClockRatio`]; command scheduling advances one DRAM clock at a time and
//! responses are converted back. Reads pay the DRAM access plus the fixed
//! front-end and back-end controller latencies. Writes are posted: the
//! requester is acknowledged once the write enters the write queue, and the
//! data reaches DRAM whenever the scheduler drains it, as in gem5's
//! `MemCtrl`. A read to a line still in the write queue is answered from
//! the queue; a write to a line already queued merges into it.
//!
//! The scheduler issues **at most one command per subchannel per DRAM clock**,
//! mirroring the DDR5 command bus. ACT, RD and WR are two-clock commands,
//! PRE and REF one-clock. A request that requires N commands to retire
//! therefore takes at least N clocks.
//!
//! Every command issued emits a [`Packet::DramCmd`] event on the scheduler,
//! targeted at the controller itself, so command traces surface in the event
//! log without any handler consuming them.

mod commands;
mod power;
mod refresh;
mod schedule;

use std::sync::Arc;

use crate::common::{LineAddr, PhysAddr};
use crate::config::ddr5::Ddr5Config;
use crate::sim::components::{
    BankGroupId, ChannelId, ComponentId, MemCtrlId, RankId, ReqId, RowId, SubchannelId,
};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{
    AccessSize, DramCmdKind, HitLevel, MemOp, MemRespData, MesiState, Packet,
};
use crate::soc::memory::address::AddressMapper;
use crate::soc::memory::buffer::DramBuffer;
use crate::soc::memory::controller::MemoryController;
use crate::soc::memory::ddr5::ecc::EccPolicy;
use crate::soc::memory::ddr5::refresh::{RankLayout, RefreshPolicy};
use crate::soc::memory::ddr5::scheduler::MemScheduler;
use crate::soc::memory::ddr5::state::{
    Bank, BankState, DramChannel, PendingReq, RefreshPhase, Subchannel, WriteDrainState,
};
use crate::soc::memory::ddr5::stats::ControllerStatPaths;

/// Cache-line size used when constructing `LineAddr` in responses.
const CACHE_LINE_BYTES: u64 = 64;

/// Command-bus cycles an ACTIVATE occupies (DDR5 two-cycle command).
const ACT_CMD_CYCLES: u64 = 2;
/// Command-bus cycles a READ or WRITE occupies (DDR5 two-cycle command).
const COLUMN_CMD_CYCLES: u64 = 2;
/// Command-bus cycles a PRECHARGE occupies.
const PRECHARGE_CMD_CYCLES: u64 = 1;
/// Command-bus cycles a REFRESH occupies.
const REFRESH_CMD_CYCLES: u64 = 1;
/// Command-bus cycles a power-down entry or exit occupies.
const POWER_CMD_CYCLES: u64 = 1;

/// Converts between simulator (core) cycles and DRAM command clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockRatio {
    cpu_mhz: u64,
    dram_mhz: u64,
}

impl ClockRatio {
    /// Ratio for a core at `cpu_mhz` driving DRAM at `data_rate_mts`
    /// (command clock = half the data rate).
    ///
    /// # Panics
    ///
    /// Panics if either clock is zero.
    #[must_use]
    pub const fn new(cpu_mhz: u64, data_rate_mts: u64) -> Self {
        assert!(cpu_mhz > 0, "core clock must be non-zero");
        assert!(data_rate_mts >= 2, "DRAM data rate must be non-zero");
        Self { cpu_mhz, dram_mhz: data_rate_mts / 2 }
    }

    /// The DRAM clock in progress at simulator cycle `cpu_cycle`.
    #[inline]
    #[must_use]
    pub const fn to_dram(self, cpu_cycle: u64) -> u64 {
        cpu_cycle * self.dram_mhz / self.cpu_mhz
    }

    /// The first simulator cycle at or after DRAM clock `dram_cycle`.
    #[inline]
    #[must_use]
    pub const fn to_cpu(self, dram_cycle: u64) -> u64 {
        (dram_cycle * self.cpu_mhz).div_ceil(self.dram_mhz)
    }
}

/// DDR5 memory controller.
#[derive(Debug)]
pub struct Ddr5Controller {
    buffer: Arc<DramBuffer>,
    base: PhysAddr,
    channels: Vec<DramChannel>,
    mapper: AddressMapper,
    config: Ddr5Config,
    self_id: MemCtrlId,
    clock: ClockRatio,
    scheduler: Box<dyn MemScheduler>,
    refresh_policy: Box<dyn RefreshPolicy>,
    layout: RankLayout,
    refresh_interval: u64,
    scrubber: Option<Scrubber>,
    stat_paths: ControllerStatPaths,
    stats_registered: bool,
    pending_commands: Vec<EmittedCommand>,
    pending_responses: Vec<ScheduledResponse>,
    /// Next DRAM clock the scheduler will process.
    next_dram_cycle: u64,
}

impl Ddr5Controller {
    /// Constructs a controller. `base` is the physical address at which the
    /// backing buffer's first byte is mapped. `self_id` names the controller
    /// so the emitted `DramCmd` events can target it. `cpu_clock_mhz` fixes
    /// the ratio between simulator cycles and the DRAM command clock.
    ///
    /// # Panics
    ///
    /// Panics if any topology count in `config` is not a power of two (see
    /// [`AddressMapper::new`]) or if `cpu_clock_mhz` is zero.
    #[must_use]
    pub fn new(
        buffer: Arc<DramBuffer>,
        base: PhysAddr,
        config: Ddr5Config,
        self_id: MemCtrlId,
        cpu_clock_mhz: u64,
    ) -> Self {
        let mapper = AddressMapper::new(
            config.address_mapping,
            config.channels,
            config.subchannels_per_channel,
            config.ranks_per_channel,
            config.bank_groups_per_rank,
            config.banks_per_group,
            config.row_bits,
            config.column_bits,
        );
        let layout = RankLayout {
            bank_groups: config.bank_groups_per_rank,
            banks_per_group: config.banks_per_group,
        };
        assert!(layout.bank_count() <= 64, "refresh bank masks cover at most 64 banks per rank");
        let refresh_policy = config.refresh.build();
        let refresh_interval = refresh_policy.interval(&config.timing, layout);
        let ecc: Box<dyn EccPolicy> = config.ecc.build();
        let scrubber = ecc.scrub_interval(&config.timing).map(|interval| Scrubber {
            interval,
            next_at: interval,
            cursor: 0,
            line_count: (buffer.len() as u64 / CACHE_LINE_BYTES).max(1),
        });
        let bank_count = layout.bank_count() as usize;
        let channels = (0..config.channels)
            .map(|_| {
                DramChannel::new(
                    usize::from(config.subchannels_per_channel),
                    usize::from(config.ranks_per_channel),
                    bank_count,
                    refresh_interval,
                )
            })
            .collect();
        let stat_paths = ControllerStatPaths::new(
            self_id.val(),
            usize::from(config.channels),
            usize::from(config.subchannels_per_channel),
            usize::from(config.ranks_per_channel),
            bank_count,
        );
        Self {
            buffer,
            base,
            channels,
            mapper,
            config,
            self_id,
            clock: ClockRatio::new(cpu_clock_mhz, config.timing.data_rate_mts),
            scheduler: config.scheduler.build(),
            refresh_policy,
            layout,
            refresh_interval,
            scrubber,
            stat_paths,
            stats_registered: false,
            pending_commands: Vec::new(),
            pending_responses: Vec::new(),
            next_dram_cycle: 0,
        }
    }

    /// Clone handle for the backing buffer (used by RAM fast-path aliasing).
    #[must_use]
    pub fn buffer(&self) -> Arc<DramBuffer> {
        Arc::clone(&self.buffer)
    }

    /// Static configuration snapshot.
    #[must_use]
    pub const fn config(&self) -> &Ddr5Config {
        &self.config
    }

    /// Core-to-DRAM clock conversion in use.
    #[must_use]
    pub const fn clock(&self) -> ClockRatio {
        self.clock
    }
}

impl Handle for Ddr5Controller {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let arrival = self.clock.to_dram(ctx.cycle);
            if matches!(op, MemOp::Maintain { dirty: false, .. }) {
                // The point of coherence acknowledges a maintenance
                // operation that brings no data without a DRAM access.
                self.pending_responses.push(ScheduledResponse {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, CACHE_LINE_BYTES),
                    payload: Payload::Ready(MemRespData::Small(0)),
                    hit_level: HitLevel::Dram,
                    fire_at: arrival + self.config.frontend_latency,
                    target: source,
                });
                return;
            }
            self.enqueue(req_id, paddr, size, op, source, arrival);
        }
        // Other packet kinds (DramCmd / RefreshTick trace events, plus any
        // stray packets not addressed to memory controllers) are ignored.
    }
}

impl MemoryController for Ddr5Controller {
    fn tick(&mut self, ctx: &mut HandleCtx<'_>) {
        let target = self.clock.to_dram(ctx.cycle);
        while self.next_dram_cycle <= target {
            let now = self.next_dram_cycle;
            self.tick_dram_cycle(now);
            self.next_dram_cycle += 1;
        }
        self.flush(ctx);
    }

    fn quiet_until(&self, cycle: u64) -> Option<u64> {
        if !self.pending_commands.is_empty() || !self.pending_responses.is_empty() {
            return Some(cycle);
        }
        let now = self.next_dram_cycle;
        let subchannels = self.channels.iter().flat_map(|c| c.subchannels.iter());
        let horizon = subchannels
            .filter_map(|subchannel| self.subchannel_quiet_until(subchannel, now))
            .chain(self.scrubber.as_ref().map(|scrubber| scrubber.next_at))
            .min()?;
        Some(if horizon <= now { cycle } else { self.clock.to_cpu(horizon).max(cycle) })
    }

    fn skip_quiet(&mut self, ctx: &mut HandleCtx<'_>) {
        let target = self.clock.to_dram(ctx.cycle);
        if target >= self.next_dram_cycle {
            let clocks = target + 1 - self.next_dram_cycle;
            for subchannel in self.channels.iter_mut().flat_map(|c| c.subchannels.iter_mut()) {
                subchannel.counters.clocks += clocks;
            }
            self.next_dram_cycle = target + 1;
        }
        self.flush(ctx);
    }

    fn resume_at(&mut self, cycle: u64) {
        let origin = self.clock.to_dram(cycle);
        for subchannel in self.channels.iter_mut().flat_map(|c| c.subchannels.iter_mut()) {
            subchannel.restart_at(origin, self.refresh_interval);
        }
        if let Some(scrubber) = &mut self.scrubber {
            scrubber.next_at = origin + scrubber.interval;
        }
        self.next_dram_cycle = origin;
    }
}

impl Ddr5Controller {
    fn enqueue(
        &mut self,
        req_id: ReqId,
        paddr: PhysAddr,
        size: AccessSize,
        op: MemOp,
        source: ComponentId,
        arrival: u64,
    ) {
        let loc = self.mapper.decompose(paddr);
        let line = LineAddr::from_phys(paddr, CACHE_LINE_BYTES);
        let scrub = source == ComponentId::MemCtrl(self.self_id);
        let pending = PendingReq {
            req_id,
            arrival_cycle: arrival,
            paddr,
            line,
            loc,
            size,
            op,
            source,
            scrub,
            activated: false,
        };
        let sc = &mut self.channels[loc.channel.as_index()].subchannels[loc.subchannel.as_index()];
        sc.inbound.push_back(pending);
    }

    /// The first DRAM clock at or after `now` at which `subchannel` may do
    /// more than count the clock, or `None` if it waits for a request.
    fn subchannel_quiet_until(&self, subchannel: &Subchannel, now: u64) -> Option<u64> {
        let queued = !subchannel.inbound.is_empty()
            || !subchannel.read_queue.is_empty()
            || !subchannel.write_queue.is_empty();
        let drain_ends = subchannel.drain_state == WriteDrainState::Draining
            && subchannel.writes_this_drain >= self.config.min_writes_per_switch;
        if queued || drain_ends {
            return Some(now);
        }
        let mut horizon: Option<u64> = None;
        for rank in &subchannel.ranks {
            if rank.refresh_phase != RefreshPhase::Idle {
                return Some(now);
            }
            let refreshed = rank.banks.iter().filter(|bank| bank.state == BankState::Refreshing);
            if let Some(end) = refreshed.map(|bank| bank.refresh_end).min() {
                horizon = Some(horizon.map_or(end, |h| h.min(end)));
            }
            if self.refresh_interval > 0 {
                horizon = Some(horizon.map_or(rank.next_refresh, |h| h.min(rank.next_refresh)));
            }
            if let Some(entry) = self.power_down_entry(subchannel, rank, now) {
                horizon = Some(horizon.map_or(entry, |h| h.min(entry)));
            }
        }
        horizon
    }

    /// Injects the next patrol-scrub read when its interval has elapsed.
    /// The sweep walks every line of the DRAM in address order and wraps.
    fn inject_scrub_read(&mut self, now: u64) {
        let Some(scrubber) = self.scrubber.as_mut() else { return };
        if now < scrubber.next_at {
            return;
        }
        scrubber.next_at = now + scrubber.interval;
        let paddr = PhysAddr::new(self.base.val() + scrubber.cursor * CACHE_LINE_BYTES);
        scrubber.cursor = (scrubber.cursor + 1) % scrubber.line_count;
        let source = ComponentId::MemCtrl(self.self_id);
        self.enqueue(ReqId::new(u64::MAX), paddr, AccessSize::Line, MemOp::Read, source, now);
    }

    fn tick_dram_cycle(&mut self, now: u64) {
        self.inject_scrub_read(now);
        let chan_count = self.channels.len();
        for chan_idx in 0..chan_count {
            let subch_count = self.channels[chan_idx].subchannels.len();
            for subch_idx in 0..subch_count {
                let chan = ChannelId::new(index_to_u8(chan_idx));
                let subch = SubchannelId::new(index_to_u8(subch_idx));
                self.channels[chan_idx].subchannels[subch_idx].counters.clocks += 1;
                self.admit(chan, subch, now);
                self.tick_subchannel(chan, subch, now);
            }
        }
    }

    /// Issues at most one command on `(chan, subch)` for DRAM clock `now`.
    /// If no ready request can advance legally at `now`, the subchannel goes
    /// idle for this clock.
    fn tick_subchannel(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) {
        self.release_refreshed_banks(chan, subch, now);
        self.update_drain_state(chan, subch);
        if self.command_bus_busy(chan, subch, now) {
            return;
        }
        if self.advance_power(chan, subch, now) {
            return;
        }
        if self.advance_refresh(chan, subch, now) {
            return;
        }
        let Some(pick_writes) = self.pick_queue(chan, subch) else { return };
        let Some(index) = self.pick_request_index(chan, subch, pick_writes, now) else {
            return;
        };
        self.step_request(chan, subch, pick_writes, index, now);
    }

    fn service_buffer(&self, request: &PendingReq) -> Payload {
        if request.op.takes_effect_when_served(request.size) {
            return Payload::Perform {
                paddr: request.paddr,
                size: request.size,
                op: request.op.clone(),
            };
        }
        let offset = (request.paddr.val().saturating_sub(self.base.val())) as usize;
        Payload::Ready(match &request.op {
            MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. } => {
                read_from_buffer(&self.buffer, offset, request.size)
            }
            MemOp::Write { .. } | MemOp::Writeback { .. } | MemOp::Maintain { .. } => {
                MemRespData::Small(0)
            }
        })
    }

    fn bank_index(&self, bg: BankGroupId, bank: u8) -> usize {
        usize::from(bg.val()) * usize::from(self.config.banks_per_group) + usize::from(bank)
    }

    fn bank_snapshot(&self, ctx: BankCmdCtx) -> Bank {
        self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()].ranks
            [ctx.rank.as_index()]
        .banks[ctx.bank_index]
    }

    /// Publishes accumulated statistics, then converts queued command traces
    /// and responses from DRAM clocks to simulator cycles and schedules them.
    fn flush(&mut self, ctx: &mut HandleCtx<'_>) {
        if !self.stats_registered {
            self.stat_paths.register(ctx.stats);
            self.stats_registered = true;
        }
        for (chan_idx, channel) in self.channels.iter_mut().enumerate() {
            for (subch_idx, subchannel) in channel.subchannels.iter_mut().enumerate() {
                self.stat_paths.publish(ctx.stats, chan_idx, subch_idx, subchannel);
            }
        }
        let self_component = ComponentId::MemCtrl(self.self_id);
        let clock = self.clock;
        for cmd in self.pending_commands.drain(..) {
            ctx.scheduler.schedule(
                clock.to_cpu(cmd.fire_at),
                self_component,
                self_component,
                Packet::DramCmd {
                    channel: cmd.channel.val(),
                    rank: cmd.rank.val(),
                    bank: cmd.bank,
                    kind: cmd.kind,
                    row: cmd.row,
                },
            );
        }
        for resp in self.pending_responses.drain(..) {
            let data = match resp.payload {
                Payload::Ready(data) => data,
                Payload::Perform { paddr, size, op } => ctx.memory.perform(paddr, size, &op),
            };
            ctx.scheduler.schedule(
                clock.to_cpu(resp.fire_at),
                resp.target,
                self_component,
                Packet::MemResp {
                    req_id: resp.req_id,
                    line_addr: resp.line_addr,
                    data,
                    hit_level: resp.hit_level,
                    state: MesiState::Exclusive,
                },
            );
        }
    }
}

/// Timing floors for an ACTIVATE, each named after the constraint it comes
/// from so violations can be reported precisely.
#[derive(Copy, Clone, Debug)]
struct ActivateBounds {
    rrd: u64,
    rc: u64,
    faw: u64,
    refresh_end: u64,
    command_bus: u64,
}

impl ActivateBounds {
    const fn earliest(self, not_before: u64) -> u64 {
        let mut earliest = not_before;
        if self.command_bus > earliest {
            earliest = self.command_bus;
        }
        if self.rrd > earliest {
            earliest = self.rrd;
        }
        if self.rc > earliest {
            earliest = self.rc;
        }
        if self.faw > earliest {
            earliest = self.faw;
        }
        if self.refresh_end > earliest {
            earliest = self.refresh_end;
        }
        earliest
    }
}

/// Coordinates identifying a single (subchannel, rank, `bank_group`, bank,
/// row) target. Bundled so per-command helpers don't need eight positional
/// args.
#[derive(Copy, Clone, Debug)]
struct BankCmdCtx {
    chan: ChannelId,
    subch: SubchannelId,
    rank: RankId,
    bg: BankGroupId,
    bank_index: usize,
    row: RowId,
}

/// Patrol-scrub sweep state.
#[derive(Copy, Clone, Debug)]
struct Scrubber {
    /// DRAM clocks between scrub reads.
    interval: u64,
    /// Clock of the next scrub read.
    next_at: u64,
    /// Next line to scrub, as an index from the DRAM base.
    cursor: u64,
    /// Lines in the DRAM.
    line_count: u64,
}

#[derive(Copy, Clone, Debug)]
struct EmittedCommand {
    channel: ChannelId,
    rank: RankId,
    bank: u8,
    row: u32,
    kind: DramCmdKind,
    fire_at: u64,
}

/// What a response carries: data read already, or a hart's access that
/// takes effect as the response leaves, in the cycle its burst completes.
#[derive(Clone, Debug)]
enum Payload {
    Ready(MemRespData),
    Perform { paddr: PhysAddr, size: AccessSize, op: MemOp },
}

impl Payload {
    /// The answer to a write admitted to the write queue: a hart's store
    /// takes effect there, where later reads of the line are served from.
    fn acknowledging(request: &PendingReq) -> Self {
        if request.op.takes_effect_when_served(request.size) {
            Self::Perform { paddr: request.paddr, size: request.size, op: request.op.clone() }
        } else {
            Self::Ready(MemRespData::Small(0))
        }
    }
}

#[derive(Clone, Debug)]
struct ScheduledResponse {
    req_id: ReqId,
    line_addr: LineAddr,
    payload: Payload,
    hit_level: HitLevel,
    /// DRAM clock at which the response leaves the controller.
    fire_at: u64,
    target: ComponentId,
}

impl ScheduledResponse {
    const fn for_request(request: &PendingReq, fire_at: u64, payload: Payload) -> Self {
        Self {
            req_id: request.req_id,
            line_addr: request.line,
            payload,
            hit_level: HitLevel::Dram,
            fire_at,
            target: request.source,
        }
    }
}

const fn is_read_op(op: &MemOp) -> bool {
    matches!(op, MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. })
}

const fn mask_has(mask: u64, bank_index: usize) -> bool {
    bank_index < 64 && (mask >> bank_index) & 1 == 1
}

const fn bank_index_u8(idx: usize) -> u8 {
    (idx & 0xff) as u8
}

const fn index_to_u8(idx: usize) -> u8 {
    (idx & 0xff) as u8
}

const fn column_lead(t: &crate::config::ddr5::timing::Ddr5Timing, is_read: bool) -> u64 {
    if is_read { t.t_cas } else { t.t_cwl }
}

fn read_from_buffer(buffer: &Arc<DramBuffer>, offset: usize, size: AccessSize) -> MemRespData {
    if size == AccessSize::Line {
        let s = buffer.read_slice(offset, CACHE_LINE_BYTES as usize);
        return MemRespData::Line(s.to_vec().into_boxed_slice());
    }
    let bytes = buffer.read_slice(offset, size.bytes());
    MemRespData::Small(bytes.iter().rev().fold(0, |value, &byte| (value << 8) | u64::from(byte)))
}
