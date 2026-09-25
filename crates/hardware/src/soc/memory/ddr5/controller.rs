//! DDR5 memory controller with per-bank command state machines.
//!
//! [`Ddr5Controller`] implements the [`MemoryController`] trait and services
//! `MemReq` packets by translating each request into a JEDEC-compliant
//! sequence of ACTIVATE / PRECHARGE / READ / WRITE / REFRESH commands. Every
//! command respects the DDR5 timing constants declared in
//! [`crate::soc::memory::ddr5::timing::Ddr5Timing`]. In debug builds each
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

use std::sync::Arc;

use crate::common::{LineAddr, PhysAddr};
use crate::sim::components::{
    BankGroupId, ChannelId, ComponentId, MemCtrlId, RankId, ReqId, RowId, SubchannelId,
};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, DramCmdKind, HitLevel, MemOp, MemRespData, Packet};
use crate::soc::memory::address::AddressMapper;
use crate::soc::memory::buffer::DramBuffer;
use crate::soc::memory::controller::MemoryController;
use crate::soc::memory::ddr5::config::{Ddr5Config, PowerDownPolicy};
use crate::soc::memory::ddr5::ecc::EccPolicy;
use crate::soc::memory::ddr5::refresh::{RankLayout, RefreshPolicy};
use crate::soc::memory::ddr5::scheduler::{Candidate, MemScheduler};
use crate::soc::memory::ddr5::state::{
    Bank, BankState, BusOp, DramChannel, PendingReq, PowerState, RefreshPhase, WriteDrainState,
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
        let sc =
            &mut self.channels[loc.channel.as_index()].subchannels[loc.subchannel.as_index()];
        sc.inbound.push_back(pending);
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

    /// Moves arrived requests from `inbound` into the read / write queues in
    /// arrival order, stopping at the first one its queue cannot hold.
    /// Writes are acknowledged on admission; a read whose line is still in
    /// the write queue is answered from the queue.
    fn admit(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) {
        let read_cap = self.config.read_queue_entries;
        let write_cap = self.config.write_queue_entries;
        let frontend = self.config.frontend_latency;
        loop {
            let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
            let Some(request) = sc.inbound.pop_front() else { break };
            if request.arrival_cycle > now {
                sc.inbound.push_front(request);
                break;
            }
            if is_read_op(&request.op) {
                if sc.write_queue.iter().any(|w| w.line == request.line) {
                    sc.counters.reads_hit_write_queue += 1;
                    if !request.scrub {
                        let payload = self.service_buffer(&request);
                        self.pending_responses.push(ScheduledResponse::for_request(
                            &request,
                            now + frontend,
                            payload,
                        ));
                    }
                    continue;
                }
                if sc.read_queue.len() >= read_cap {
                    sc.counters.read_admission_stalls += 1;
                    sc.inbound.push_front(request);
                    break;
                }
                if request.scrub {
                    sc.counters.scrub_reads += 1;
                } else {
                    sc.counters.reads += 1;
                }
                sc.counters.read_queue_depth_samples.push(sc.read_queue.len() as u64);
                sc.read_queue.push_back(request);
                continue;
            }
            let merges = sc.write_queue.iter().any(|w| w.line == request.line);
            if !merges && sc.write_queue.len() >= write_cap {
                sc.counters.write_admission_stalls += 1;
                sc.inbound.push_front(request);
                break;
            }
            self.pending_responses.push(ScheduledResponse::for_request(
                &request,
                now + frontend,
                MemRespData::Small(0),
            ));
            if merges {
                sc.counters.writes_merged += 1;
            } else {
                sc.counters.writes += 1;
                sc.counters.write_queue_depth_samples.push(sc.write_queue.len() as u64);
                sc.write_queue.push_back(request);
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

    /// True iff the subchannel already emitted a command whose bus tenure
    /// covers `now`.
    fn command_bus_busy(&self, chan: ChannelId, subch: SubchannelId, now: u64) -> bool {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        sc.last_command_cycle > now
    }

    /// Asks the scheduler which request in the chosen queue to advance.
    fn pick_request_index(
        &self,
        chan: ChannelId,
        subch: SubchannelId,
        pick_writes: bool,
        now: u64,
    ) -> Option<usize> {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        let queue = if pick_writes { &sc.write_queue } else { &sc.read_queue };
        let mut indices = Vec::with_capacity(queue.len());
        let mut candidates = Vec::with_capacity(queue.len());
        for (index, req) in queue.iter().enumerate() {
            let rank = &sc.ranks[req.loc.rank.as_index()];
            if rank.bank_held_for_refresh(self.bank_index(req.loc.bank_group, req.loc.bank)) {
                continue;
            }
            indices.push(index);
            candidates.push(self.candidate(chan, subch, req));
        }
        self.scheduler.pick(&candidates, now).map(|chosen| indices[chosen])
    }

    /// Summarises `req` for the scheduler: whether its row is open and the
    /// earliest clock its column command could issue from the bank's
    /// current state.
    fn candidate(&self, chan: ChannelId, subch: SubchannelId, req: &PendingReq) -> Candidate {
        let t = &self.config.timing;
        let ctx = BankCmdCtx {
            chan,
            subch,
            rank: req.loc.rank,
            bg: req.loc.bank_group,
            bank_index: self.bank_index(req.loc.bank_group, req.loc.bank),
            row: req.loc.row,
        };
        let bank = self.bank_snapshot(ctx);
        let is_read = is_read_op(&req.op);
        match bank.state {
            BankState::Active if bank.open_row == Some(req.loc.row) => Candidate {
                row_hit: true,
                ready_at: self.column_issue_earliest(&ctx, &bank, is_read),
            },
            BankState::Active => {
                let precharge = self.precharge_earliest(&ctx, &bank);
                let activate = self.activate_earliest(&ctx, precharge + t.t_rp);
                Candidate { row_hit: false, ready_at: activate + t.t_rcd }
            }
            BankState::Precharging => {
                let activate = self.activate_earliest(&ctx, bank.last_precharge + t.t_rp);
                Candidate { row_hit: false, ready_at: activate + t.t_rcd }
            }
            BankState::Idle => {
                let activate = self.activate_earliest(&ctx, 0);
                Candidate { row_hit: false, ready_at: activate + t.t_rcd }
            }
            BankState::Refreshing => {
                let activate = self.activate_earliest(&ctx, bank.refresh_end);
                Candidate { row_hit: false, ready_at: activate + t.t_rcd }
            }
        }
    }

    fn update_drain_state(&mut self, chan: ChannelId, subch: SubchannelId) {
        let high = self.config.write_high_watermark;
        let low = self.config.write_low_watermark;
        let min_writes = self.config.min_writes_per_switch;
        let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
        let depth = sc.write_queue.len();
        sc.drain_state = match sc.drain_state {
            WriteDrainState::Filling if depth >= high => {
                sc.writes_this_drain = 0;
                WriteDrainState::Draining
            }
            WriteDrainState::Draining
                if depth <= low && sc.writes_this_drain >= min_writes =>
            {
                WriteDrainState::Filling
            }
            state => state,
        };
    }

    fn pick_queue(&self, chan: ChannelId, subch: SubchannelId) -> Option<bool> {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        match (sc.read_queue.is_empty(), sc.write_queue.is_empty()) {
            (true, true) => None,
            (false, true) => Some(false),
            (true, false) => Some(true),
            (false, false) => Some(matches!(sc.drain_state, WriteDrainState::Draining)),
        }
    }

    /// Returns banks whose refresh has completed to the idle state.
    fn release_refreshed_banks(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) {
        let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
        for rank in &mut sc.ranks {
            for bank in &mut rank.banks {
                if bank.state == BankState::Refreshing && bank.refresh_end <= now {
                    bank.state = BankState::Idle;
                }
            }
        }
    }

    /// Drives every rank's power state for one clock: exits power-down when
    /// the rank has work (a queued request or a due refresh) and tPD has
    /// elapsed; enters it when the rank has been idle for the policy's
    /// timer. Returns `true` iff a power command was issued.
    fn advance_power(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) -> bool {
        let PowerDownPolicy::AfterIdle { idle_clocks } = self.config.power_down else {
            return false;
        };
        let t = self.config.timing;
        let rank_count = self.channels[chan.as_index()].subchannels[subch.as_index()].ranks.len();
        for rank_idx in 0..rank_count {
            let rank = RankId::new(index_to_u8(rank_idx));
            let has_work = self.rank_has_work(chan, subch, rank, now);
            let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
            let data_bus_idle = sc.last_data_end <= now;
            let rank_mut = &mut sc.ranks[rank_idx];
            match rank_mut.power {
                PowerState::PowerDown { since, .. } => {
                    if !has_work || now < since + t.t_pd || rank_mut.command_floor() > now {
                        continue;
                    }
                    rank_mut.power = PowerState::Active;
                    rank_mut.power_up_at = now + t.t_xp;
                    rank_mut.last_command_cycle = rank_mut.last_command_cycle.max(now + POWER_CMD_CYCLES);
                    sc.last_command_cycle = sc.last_command_cycle.max(now + POWER_CMD_CYCLES);
                    sc.counters.power_down_exits += 1;
                    self.pending_commands.push(EmittedCommand {
                        channel: chan,
                        rank,
                        bank: 0,
                        row: 0,
                        kind: DramCmdKind::PowerDownExit,
                        fire_at: now,
                    });
                    return true;
                }
                PowerState::Active => {
                    let idle_since = rank_mut.command_floor();
                    let refreshing = rank_mut.refresh_phase != RefreshPhase::Idle
                        || rank_mut.banks.iter().any(|b| b.state == BankState::Refreshing);
                    if has_work
                        || refreshing
                        || !data_bus_idle
                        || now < idle_since + idle_clocks
                        || rank_mut.next_refresh <= now + t.t_pd
                    {
                        continue;
                    }
                    rank_mut.power =
                        PowerState::PowerDown { since: now, with_open_rows: rank_mut.has_open_row() };
                    rank_mut.last_command_cycle = rank_mut.last_command_cycle.max(now + POWER_CMD_CYCLES);
                    sc.last_command_cycle = sc.last_command_cycle.max(now + POWER_CMD_CYCLES);
                    sc.counters.power_down_entries += 1;
                    self.pending_commands.push(EmittedCommand {
                        channel: chan,
                        rank,
                        bank: 0,
                        row: 0,
                        kind: DramCmdKind::PowerDownEntry,
                        fire_at: now,
                    });
                    return true;
                }
            }
        }
        false
    }

    /// True if a queued request targets `rank` or its refresh is due.
    fn rank_has_work(&self, chan: ChannelId, subch: SubchannelId, rank: RankId, now: u64) -> bool {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        let rank_ref = &sc.ranks[rank.as_index()];
        rank_ref.next_refresh <= now
            || rank_ref.refresh_phase != RefreshPhase::Idle
            || sc.read_queue.iter().chain(sc.write_queue.iter()).any(|r| r.loc.rank == rank)
    }

    /// Drives every rank's refresh state machine for one clock. A rank
    /// whose refresh is due first stops taking new commands for the
    /// covered banks, precharges any open rows among them (PRECHARGE-ALL,
    /// once tRAS / tRTP / tWR allow), then issues REFRESH once tRP has
    /// elapsed. Returns `true` iff a command was issued.
    fn advance_refresh(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) -> bool {
        if self.refresh_interval == 0 {
            return false;
        }
        let rank_count = self.channels[chan.as_index()].subchannels[subch.as_index()].ranks.len();
        for rank_idx in 0..rank_count {
            let rank = RankId::new(index_to_u8(rank_idx));
            self.arm_due_refresh(chan, subch, rank, now);
            let RefreshPhase::Pending { bank_mask, duration } =
                self.channels[chan.as_index()].subchannels[subch.as_index()].ranks[rank_idx]
                    .refresh_phase
            else {
                continue;
            };
            if self.try_precharge_for_refresh(chan, subch, rank, bank_mask, now) {
                return true;
            }
            if self.try_issue_refresh(chan, subch, rank, bank_mask, duration, now) {
                return true;
            }
        }
        false
    }

    /// Moves a rank whose refresh interval has elapsed into the pending
    /// phase, freezing the covered banks.
    fn arm_due_refresh(&mut self, chan: ChannelId, subch: SubchannelId, rank: RankId, now: u64) {
        let timing = self.config.timing;
        let layout = self.layout;
        let rank_mut = &mut self.channels[chan.as_index()].subchannels[subch.as_index()]
            .ranks[rank.as_index()];
        if rank_mut.refresh_phase != RefreshPhase::Idle || rank_mut.next_refresh > now {
            return;
        }
        let target = self.refresh_policy.target(&timing, layout, rank_mut.refresh_seq);
        rank_mut.refresh_phase =
            RefreshPhase::Pending { bank_mask: target.bank_mask, duration: target.duration };
    }

    /// Precharges the open rows a pending refresh covers as soon as every
    /// one of them may legally close. Returns `true` iff PRECHARGE-ALL issued.
    fn try_precharge_for_refresh(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        rank: RankId,
        bank_mask: u64,
        now: u64,
    ) -> bool {
        let bank_count = self.layout.bank_count() as usize;
        let mut earliest = 0u64;
        let mut any_open = false;
        for bank_index in (0..bank_count).filter(|i| mask_has(bank_mask, *i)) {
            let ctx = self.bank_ctx(chan, subch, rank, bank_index);
            let bank = self.bank_snapshot(ctx);
            if bank.state == BankState::Active {
                any_open = true;
                earliest = earliest.max(self.precharge_earliest(&ctx, &bank));
            }
        }
        if !any_open || earliest > now {
            return false;
        }
        let fire_at = earliest.max(now);
        {
            let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
            sc.last_command_cycle = sc.last_command_cycle.max(fire_at + PRECHARGE_CMD_CYCLES);
            sc.counters.precharge_alls += 1;
            let rank_mut = &mut sc.ranks[rank.as_index()];
            rank_mut.last_command_cycle =
                rank_mut.last_command_cycle.max(fire_at + PRECHARGE_CMD_CYCLES);
            rank_mut.last_precharge = fire_at;
            for (bank_index, bank) in rank_mut.banks.iter_mut().enumerate() {
                if mask_has(bank_mask, bank_index) && bank.state == BankState::Active {
                    bank.state = BankState::Precharging;
                    bank.last_precharge = fire_at;
                    bank.open_row = None;
                }
            }
        }
        self.pending_commands.push(EmittedCommand {
            channel: chan,
            rank,
            bank: 0,
            row: 0,
            kind: DramCmdKind::PrechargeAll,
            fire_at,
        });
        true
    }

    /// Issues the REFRESH for a pending refresh once every covered bank is
    /// precharged with tRP elapsed. Returns `true` iff it issued.
    fn try_issue_refresh(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        rank: RankId,
        bank_mask: u64,
        duration: u64,
        now: u64,
    ) -> bool {
        let t_rp = self.config.timing.t_rp;
        let earliest = {
            let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
            let rank_ref = &sc.ranks[rank.as_index()];
            let mut earliest = rank_ref.command_floor().max(sc.last_command_cycle);
            for (bank_index, bank) in rank_ref.banks.iter().enumerate() {
                if !mask_has(bank_mask, bank_index) {
                    continue;
                }
                match bank.state {
                    BankState::Active => return false,
                    BankState::Precharging => earliest = earliest.max(bank.last_precharge + t_rp),
                    BankState::Refreshing => earliest = earliest.max(bank.refresh_end),
                    BankState::Idle => {}
                }
            }
            earliest
        };
        if earliest > now {
            return false;
        }
        let fire_at = earliest.max(now);
        let refresh_end = fire_at + duration;
        let interval = self.refresh_interval;
        let set = {
            let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
            sc.last_command_cycle = sc.last_command_cycle.max(fire_at + REFRESH_CMD_CYCLES);
            sc.counters.refreshes += 1;
            let rank_mut = &mut sc.ranks[rank.as_index()];
            rank_mut.last_command_cycle =
                rank_mut.last_command_cycle.max(fire_at + REFRESH_CMD_CYCLES);
            for (bank_index, bank) in rank_mut.banks.iter_mut().enumerate() {
                if mask_has(bank_mask, bank_index) {
                    bank.state = BankState::Refreshing;
                    bank.refresh_end = refresh_end;
                    bank.open_row = None;
                }
            }
            let set = rank_mut.refresh_seq;
            rank_mut.refresh_seq += 1;
            rank_mut.next_refresh += interval;
            rank_mut.refresh_phase = RefreshPhase::Idle;
            set
        };
        self.pending_commands.push(EmittedCommand {
            channel: chan,
            rank,
            bank: index_to_u8(set as usize % usize::from(self.config.banks_per_group.max(1))),
            row: 0,
            kind: DramCmdKind::Refresh,
            fire_at,
        });
        true
    }

    fn bank_ctx(
        &self,
        chan: ChannelId,
        subch: SubchannelId,
        rank: RankId,
        bank_index: usize,
    ) -> BankCmdCtx {
        let banks_per_group = usize::from(self.config.banks_per_group.max(1));
        BankCmdCtx {
            chan,
            subch,
            rank,
            bg: BankGroupId::new(index_to_u8(bank_index / banks_per_group)),
            bank_index,
            row: RowId::new(0),
        }
    }

    /// Advances a single request by one command step. Removes it from its
    /// queue only when the column command has just been issued (final step).
    /// Does nothing if the next command's earliest legal clock exceeds `now`;
    /// the request stays in the queue for a subsequent clock.
    fn step_request(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        pick_writes: bool,
        index: usize,
        now: u64,
    ) {
        let request = {
            let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
            let queue = if pick_writes { &sc.write_queue } else { &sc.read_queue };
            match queue.get(index) {
                Some(req) => req.clone(),
                None => return,
            }
        };
        let bank_index = self.bank_index(request.loc.bank_group, request.loc.bank);
        let ctx = BankCmdCtx {
            chan,
            subch,
            rank: request.loc.rank,
            bg: request.loc.bank_group,
            bank_index,
            row: request.loc.row,
        };
        let snapshot = self.bank_snapshot(ctx);
        let is_read = is_read_op(&request.op);
        let activated = match snapshot.state {
            BankState::Active if snapshot.open_row == Some(request.loc.row) => {
                self.try_issue_column(&ctx, pick_writes, index, &request, is_read, now);
                false
            }
            BankState::Active => {
                self.try_issue_precharge(&ctx, &snapshot, now);
                false
            }
            BankState::Precharging => {
                let earliest = snapshot.last_precharge + self.config.timing.t_rp;
                self.try_issue_activate(&ctx, earliest, now)
            }
            BankState::Idle => self.try_issue_activate(&ctx, 0, now),
            // Refreshing banks are never offered to the scheduler.
            BankState::Refreshing => false,
        };
        if activated {
            let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
            let queue = if pick_writes { &mut sc.write_queue } else { &mut sc.read_queue };
            if let Some(req) = queue.get_mut(index) {
                req.activated = true;
            }
        }
    }

    /// Earliest clock a PRECHARGE of `snapshot`'s bank is legal: tRAS from
    /// its ACT, tRTP from its last read, tWR from its last write burst,
    /// tPPD from the rank's last precharge, and the rank's command bus.
    fn precharge_earliest(&self, ctx: &BankCmdCtx, snapshot: &Bank) -> u64 {
        let t = &self.config.timing;
        let ras_bound = snapshot.last_activate + t.t_ras;
        let rtp_bound =
            if snapshot.last_read_cmd == 0 { 0 } else { snapshot.last_read_cmd + t.t_rtp };
        let wr_bound =
            if snapshot.last_write_end == 0 { 0 } else { snapshot.last_write_end + t.t_wr };
        let rank_ref = &self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()]
            .ranks[ctx.rank.as_index()];
        let ppd_bound =
            if rank_ref.last_precharge == 0 { 0 } else { rank_ref.last_precharge + t.t_ppd };
        ras_bound.max(rtp_bound).max(wr_bound).max(rank_ref.command_floor()).max(ppd_bound)
    }

    /// Issues PRECHARGE if legal at `now`; otherwise leaves the bank alone.
    fn try_issue_precharge(&mut self, ctx: &BankCmdCtx, snapshot: &Bank, now: u64) {
        let earliest = self.precharge_earliest(ctx, snapshot);
        if earliest > now {
            return;
        }
        let fire_at = earliest.max(now);
        debug_assert!(
            fire_at >= snapshot.last_activate + self.config.timing.t_ras,
            "tRAS violation: PRE at {fire_at}, ACT at {}, tRAS={}",
            snapshot.last_activate,
            self.config.timing.t_ras,
        );
        self.commit_precharge(ctx, fire_at);
    }

    fn commit_precharge(&mut self, ctx: &BankCmdCtx, fire_at: u64) {
        let open_row = self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()]
            .ranks[ctx.rank.as_index()]
            .banks[ctx.bank_index]
            .open_row
            .map_or(0, RowId::val);
        {
            let sc = &mut self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()];
            sc.last_command_cycle = sc.last_command_cycle.max(fire_at + PRECHARGE_CMD_CYCLES);
            let rank_mut = &mut sc.ranks[ctx.rank.as_index()];
            rank_mut.last_command_cycle =
                rank_mut.last_command_cycle.max(fire_at + PRECHARGE_CMD_CYCLES);
            rank_mut.last_precharge = fire_at;
            let bank = &mut rank_mut.banks[ctx.bank_index];
            bank.state = BankState::Precharging;
            bank.last_precharge = fire_at;
            bank.open_row = None;
            sc.counters.precharges += 1;
        }
        self.pending_commands.push(EmittedCommand {
            channel: ctx.chan,
            rank: ctx.rank,
            bank: bank_index_u8(ctx.bank_index),
            row: open_row,
            kind: DramCmdKind::Precharge,
            fire_at,
        });
    }

    /// Timing floors an ACTIVATE of `ctx`'s bank must clear.
    fn activate_bounds(&self, ctx: &BankCmdCtx) -> ActivateBounds {
        let t = self.config.timing;
        let banks_per_group = usize::from(self.config.banks_per_group);
        let sc = &self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()];
        let rank_ref = &sc.ranks[ctx.rank.as_index()];
        let mut rrd = 0u64;
        for (idx, other) in rank_ref.banks.iter().enumerate() {
            if other.last_activate == 0 {
                continue;
            }
            let other_bg = idx / banks_per_group;
            let bound = if other_bg == ctx.bg.as_index() {
                other.last_activate + t.t_rrd_l
            } else {
                other.last_activate + t.t_rrd_s
            };
            if bound > rrd {
                rrd = bound;
            }
        }
        let same = &rank_ref.banks[ctx.bank_index];
        ActivateBounds {
            rrd,
            rc: if same.last_activate == 0 { 0 } else { same.last_activate + t.t_rc },
            faw: rank_ref.earliest_activate_faw(t.t_faw),
            refresh_end: same.refresh_end,
            command_bus: rank_ref.command_floor(),
        }
    }

    /// Earliest clock an ACTIVATE of `ctx`'s bank is legal; `not_before`
    /// carries the caller's state-dependent floor (e.g. `last_precharge + tRP`).
    fn activate_earliest(&self, ctx: &BankCmdCtx, not_before: u64) -> u64 {
        self.activate_bounds(ctx).earliest(not_before)
    }

    /// Issues ACTIVATE if legal at `now`; `not_before` covers state-dependent
    /// bounds already known by the caller (e.g. `last_precharge + tRP`).
    /// Returns `true` iff the command issued.
    fn try_issue_activate(&mut self, ctx: &BankCmdCtx, not_before: u64, now: u64) -> bool {
        let bounds = self.activate_bounds(ctx);
        let earliest = bounds.earliest(not_before);
        if earliest > now {
            return false;
        }
        let fire_at = earliest.max(now);
        debug_assert!(
            fire_at >= bounds.rrd,
            "tRRD violation: ACT at {fire_at}, min {}",
            bounds.rrd,
        );
        debug_assert!(
            fire_at >= bounds.faw,
            "tFAW violation: ACT at {fire_at}, faw min {}",
            bounds.faw,
        );
        debug_assert!(
            fire_at >= bounds.rc,
            "tRC violation: ACT at {fire_at}, prior ACT + tRC = {}",
            bounds.rc,
        );
        self.commit_activate(ctx, fire_at);
        true
    }

    fn commit_activate(&mut self, ctx: &BankCmdCtx, fire_at: u64) {
        let t = self.config.timing;
        {
            let sc = &mut self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()];
            sc.last_command_cycle = sc.last_command_cycle.max(fire_at + ACT_CMD_CYCLES);
            let rank_mut = &mut sc.ranks[ctx.rank.as_index()];
            rank_mut.last_command_cycle = rank_mut.last_command_cycle.max(fire_at + ACT_CMD_CYCLES);
            rank_mut.record_activate(fire_at, t.t_faw);
            let bank = &mut rank_mut.banks[ctx.bank_index];
            bank.state = BankState::Active;
            bank.open_row = Some(ctx.row);
            bank.last_activate = fire_at;
            bank.counters.activates += 1;
            sc.counters.activates += 1;
        }
        self.pending_commands.push(EmittedCommand {
            channel: ctx.chan,
            rank: ctx.rank,
            bank: bank_index_u8(ctx.bank_index),
            row: ctx.row.val(),
            kind: DramCmdKind::Activate,
            fire_at,
        });
    }

    /// Issues a READ or WRITE column command if legal at `now`. On success,
    /// removes the request from its queue; reads schedule their response at
    /// the end of the data burst plus the controller latencies.
    fn try_issue_column(
        &mut self,
        ctx: &BankCmdCtx,
        pick_writes: bool,
        index: usize,
        request: &PendingReq,
        is_read: bool,
        now: u64,
    ) {
        let bank = self.bank_snapshot(*ctx);
        let column_aligned = self.column_issue_earliest(ctx, &bank, is_read);
        if column_aligned > now {
            return;
        }
        let fire_at = column_aligned.max(now);
        let lead = column_lead(&self.config.timing, is_read);
        let data_start_aligned = self.data_bus_start(ctx.chan, ctx.subch, ctx.rank, fire_at, is_read);
        let data_start = (fire_at + lead).max(data_start_aligned);
        let data_end = data_start + self.config.timing.bl_half;
        self.commit_column(ctx, fire_at, data_start, data_end, is_read);
        self.account_column(ctx, request, is_read, data_end);
        if is_read && !request.scrub {
            let payload = self.service_buffer(request);
            let ready = data_end + self.config.frontend_latency + self.config.backend_latency;
            self.pending_responses.push(ScheduledResponse::for_request(request, ready, payload));
        }
        self.pop_request(ctx.chan, ctx.subch, pick_writes, index);
    }

    /// Records the statistics of a column command that just issued.
    fn account_column(&mut self, ctx: &BankCmdCtx, request: &PendingReq, is_read: bool, data_end: u64) {
        let bl_half = self.config.timing.bl_half;
        let sc = &mut self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()];
        let bank = &mut sc.ranks[ctx.rank.as_index()].banks[ctx.bank_index];
        if request.activated {
            bank.counters.row_misses += 1;
            sc.counters.row_misses += 1;
        } else {
            bank.counters.row_hits += 1;
            sc.counters.row_hits += 1;
        }
        if is_read {
            bank.counters.reads += 1;
            if !request.scrub {
                sc.counters.read_latency_samples.push(data_end.saturating_sub(request.arrival_cycle));
            }
        } else {
            bank.counters.writes += 1;
            sc.writes_this_drain += 1;
        }
        sc.counters.bus_busy_clocks += bl_half;
    }

    fn pop_request(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        pick_writes: bool,
        index: usize,
    ) {
        let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
        let _ = if pick_writes {
            sc.write_queue.remove(index)
        } else {
            sc.read_queue.remove(index)
        };
    }

    fn commit_column(
        &mut self,
        ctx: &BankCmdCtx,
        column_cycle: u64,
        data_start: u64,
        data_end: u64,
        is_read: bool,
    ) {
        let t = &self.config.timing;
        debug_assert!(
            data_end == data_start + t.bl_half,
            "burst length mismatch: {data_end} != {data_start} + {}",
            t.bl_half,
        );
        let kind = if is_read { DramCmdKind::Read } else { DramCmdKind::Write };
        {
            let sc = &mut self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()];
            sc.last_command_cycle = sc.last_command_cycle.max(column_cycle + COLUMN_CMD_CYCLES);
            sc.ranks[ctx.rank.as_index()].last_command_cycle = sc.ranks[ctx.rank.as_index()]
                .last_command_cycle
                .max(column_cycle + COLUMN_CMD_CYCLES);
            if is_read {
                sc.last_read_cmd = column_cycle;
                sc.last_read_end = data_end;
            } else {
                sc.last_write_cmd = column_cycle;
                sc.last_write_end = data_end;
            }
            sc.last_data_end = data_end;
            sc.last_bus_rank = Some(ctx.rank);
            sc.last_data_op = if is_read { BusOp::Read } else { BusOp::Write };
            sc.last_column_bg = Some(ctx.bg);
            let bank = &mut sc.ranks[ctx.rank.as_index()].banks[ctx.bank_index];
            if is_read {
                bank.last_read_cmd = column_cycle;
                bank.last_read_end = data_end;
            } else {
                bank.last_write_cmd = column_cycle;
                bank.last_write_end = data_end;
            }
        }
        self.pending_commands.push(EmittedCommand {
            channel: ctx.chan,
            rank: ctx.rank,
            bank: bank_index_u8(ctx.bank_index),
            row: ctx.row.val(),
            kind,
            fire_at: column_cycle,
        });
    }

    /// Earliest clock the column command for `bank` (which holds the row
    /// open) can issue such that the command-bus, tCCD / tWTR and data-bus
    /// constraints all hold and the burst follows the command by exactly
    /// tCAS / tCWL.
    fn column_issue_earliest(&self, ctx: &BankCmdCtx, bank: &Bank, is_read: bool) -> u64 {
        let rcd_bound = bank.last_activate + self.config.timing.t_rcd;
        let column_earliest =
            self.column_earliest(ctx.chan, ctx.subch, ctx.rank, ctx.bg, rcd_bound, is_read);
        let data_earliest =
            self.data_bus_start(ctx.chan, ctx.subch, ctx.rank, column_earliest, is_read);
        let (column_aligned, _) =
            self.align_column_to_data_bus(column_earliest, data_earliest, is_read);
        column_aligned
    }

    /// Earliest column-command clock honoring tCCD, tWTR, and command-bus
    /// availability. `not_before` is the caller-supplied lower bound (typically
    /// `last_activate + tRCD`). Read-to-write turnaround is a data-bus
    /// constraint and lives in [`Self::data_bus_start`].
    fn column_earliest(
        &self,
        chan: ChannelId,
        subch: SubchannelId,
        rank: RankId,
        bg: BankGroupId,
        not_before: u64,
        is_read: bool,
    ) -> u64 {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        let rank_ref = &sc.ranks[rank.as_index()];
        let t = &self.config.timing;
        let last_col = sc.last_read_cmd.max(sc.last_write_cmd);
        let ccd = if last_col == 0 {
            0
        } else {
            let same_bg = sc.last_column_bg == Some(bg);
            let spacing = if same_bg {
                if is_read { t.t_ccd_l } else { t.t_ccd_l_wr }
            } else {
                t.t_ccd_s
            };
            last_col + spacing
        };
        let mut result = not_before.max(rank_ref.command_floor()).max(ccd);
        if is_read && sc.last_data_op == BusOp::Write && sc.last_write_end > 0 {
            let same_bg = sc.last_column_bg == Some(bg);
            let bound = if same_bg {
                sc.last_write_end + t.t_wtr_l
            } else {
                sc.last_write_end + t.t_wtr_s
            };
            result = result.max(bound);
        }
        result
    }

    fn data_bus_start(
        &self,
        chan: ChannelId,
        subch: SubchannelId,
        rank: RankId,
        column_cycle: u64,
        is_read: bool,
    ) -> u64 {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        let t = &self.config.timing;
        let intrinsic = column_cycle + column_lead(t, is_read);
        let mut earliest = intrinsic.max(sc.last_data_end);
        if let Some(last_rank) = sc.last_bus_rank
            && last_rank != rank
        {
            earliest = earliest.max(sc.last_data_end + t.t_rtrs);
        }
        if !is_read && sc.last_data_op == BusOp::Read {
            earliest = earliest.max(sc.last_read_end + t.t_rtw);
        }
        earliest
    }

    /// Walks the column-command clock forward so RD→data == tCAS (WR→data == tCWL)
    /// still holds when the data bus was the binding constraint.
    const fn align_column_to_data_bus(&self, column: u64, data_start: u64, is_read: bool) -> (u64, u64) {
        let lead = column_lead(&self.config.timing, is_read);
        if data_start > column + lead {
            let new_col = data_start - lead;
            (new_col, data_start)
        } else {
            (column, column + lead)
        }
    }

    fn service_buffer(&self, request: &PendingReq) -> MemRespData {
        let offset = (request.paddr.val().saturating_sub(self.base.val())) as usize;
        match &request.op {
            MemOp::Read | MemOp::Fetch | MemOp::Atomic { .. } => {
                read_from_buffer(&self.buffer, offset, request.size)
            }
            MemOp::Write { .. } | MemOp::Writeback { .. } => MemRespData::Small(0),
        }
    }

    fn bank_index(&self, bg: BankGroupId, bank: u8) -> usize {
        usize::from(bg.val()) * usize::from(self.config.banks_per_group) + usize::from(bank)
    }

    fn bank_snapshot(&self, ctx: BankCmdCtx) -> Bank {
        self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()]
            .ranks[ctx.rank.as_index()]
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
            ctx.scheduler.schedule(
                clock.to_cpu(resp.fire_at),
                resp.target,
                self_component,
                Packet::MemResp {
                    req_id: resp.req_id,
                    line_addr: resp.line_addr,
                    data: resp.data,
                    hit_level: resp.hit_level,
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

#[derive(Clone, Debug)]
struct ScheduledResponse {
    req_id: ReqId,
    line_addr: LineAddr,
    data: MemRespData,
    hit_level: HitLevel,
    /// DRAM clock at which the response leaves the controller.
    fire_at: u64,
    target: ComponentId,
}

impl ScheduledResponse {
    const fn for_request(request: &PendingReq, fire_at: u64, data: MemRespData) -> Self {
        Self {
            req_id: request.req_id,
            line_addr: request.line,
            data,
            hit_level: HitLevel::Dram,
            fire_at,
            target: request.source,
        }
    }
}

const fn is_read_op(op: &MemOp) -> bool {
    matches!(op, MemOp::Read | MemOp::Fetch | MemOp::Atomic { .. })
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

const fn column_lead(t: &crate::soc::memory::ddr5::timing::Ddr5Timing, is_read: bool) -> u64 {
    if is_read { t.t_cas } else { t.t_cwl }
}

fn read_from_buffer(buffer: &Arc<DramBuffer>, offset: usize, size: AccessSize) -> MemRespData {
    match size {
        AccessSize::B1 => MemRespData::Small(u64::from(buffer.read_u8(offset))),
        AccessSize::B2 => {
            let s = buffer.read_slice(offset, 2);
            MemRespData::Small(u64::from(u16::from_le_bytes([s[0], s[1]])))
        }
        AccessSize::B4 => {
            let s = buffer.read_slice(offset, 4);
            MemRespData::Small(u64::from(u32::from_le_bytes([s[0], s[1], s[2], s[3]])))
        }
        AccessSize::B8 => {
            let s = buffer.read_slice(offset, 8);
            MemRespData::Small(u64::from_le_bytes([
                s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
            ]))
        }
        AccessSize::Line => {
            let s = buffer.read_slice(offset, CACHE_LINE_BYTES as usize);
            MemRespData::Line(s.to_vec().into_boxed_slice())
        }
    }
}
