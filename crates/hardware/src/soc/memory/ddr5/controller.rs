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
//! The scheduler issues **at most one command per subchannel per tick**,
//! mirroring the DDR5 command bus (one command per command-clock cycle per
//! subchannel). A request that requires N commands to retire therefore takes
//! at least N tick invocations. This matches gem5's `MemCtrl::processNextReqEvent`
//! and is what real memory controllers physically do.
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
use crate::soc::memory::ddr5::config::Ddr5Config;
use crate::soc::memory::ddr5::state::{
    Bank, BankState, BusOp, DramChannel, PendingReq, WriteDrainState,
};

/// Cache-line size used when constructing `LineAddr` in responses.
const CACHE_LINE_BYTES: u64 = 64;

/// DDR5 memory controller.
#[derive(Debug)]
pub struct Ddr5Controller {
    buffer: Arc<DramBuffer>,
    base: PhysAddr,
    channels: Vec<DramChannel>,
    mapper: AddressMapper,
    config: Ddr5Config,
    self_id: MemCtrlId,
    pending_commands: Vec<EmittedCommand>,
    pending_responses: Vec<ScheduledResponse>,
    /// Highest simulator cycle the controller has processed so far. Advances
    /// each [`MemoryController::tick`] to the incoming `ctx.cycle`.
    now: u64,
}

impl Ddr5Controller {
    /// Constructs a controller. `base` is the physical address at which the
    /// backing buffer's first byte is mapped. `self_id` names the controller
    /// so the emitted `DramCmd` events can target it.
    ///
    /// # Panics
    ///
    /// Panics if any topology count in `config` is not a power of two (see
    /// [`AddressMapper::new`]).
    #[must_use]
    pub fn new(
        buffer: Arc<DramBuffer>,
        base: PhysAddr,
        config: Ddr5Config,
        self_id: MemCtrlId,
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
        let bank_count =
            usize::from(config.bank_groups_per_rank) * usize::from(config.banks_per_group);
        let channels = (0..config.channels)
            .map(|_| {
                DramChannel::new(
                    usize::from(config.subchannels_per_channel),
                    usize::from(config.ranks_per_channel),
                    bank_count,
                    config.timing.t_refi,
                )
            })
            .collect();
        Self {
            buffer,
            base,
            channels,
            mapper,
            config,
            self_id,
            pending_commands: Vec::new(),
            pending_responses: Vec::new(),
            now: 0,
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
}

impl Handle for Ddr5Controller {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            self.enqueue(req_id, paddr, size, op, source, ctx.cycle);
        }
        // Other packet kinds (DramCmd / RefreshTick trace events, plus any
        // stray packets not addressed to memory controllers) are ignored.
    }
}

impl MemoryController for Ddr5Controller {
    fn tick(&mut self, ctx: &mut HandleCtx<'_>) {
        if ctx.cycle > self.now {
            self.now = ctx.cycle;
        }
        let chan_count = self.channels.len();
        for chan_idx in 0..chan_count {
            let subch_count = self.channels[chan_idx].subchannels.len();
            for subch_idx in 0..subch_count {
                let chan = ChannelId::new(index_to_u8(chan_idx));
                let subch = SubchannelId::new(index_to_u8(subch_idx));
                self.tick_subchannel(chan, subch, self.now);
            }
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
        now: u64,
    ) {
        let loc = self.mapper.decompose(paddr);
        let is_read = matches!(op, MemOp::Read | MemOp::Fetch | MemOp::Atomic { .. });
        let pending = PendingReq { req_id, arrival_cycle: now, paddr, loc, size, op, source };
        let sc =
            &mut self.channels[loc.channel.as_index()].subchannels[loc.subchannel.as_index()];
        if is_read {
            sc.read_queue.push_back(pending);
        } else {
            sc.write_queue.push_back(pending);
        }
    }

    /// Issues at most one command on `(chan, subch)` for cycle `now`. Real DDR5
    /// command buses accept one command per command-clock cycle per subchannel;
    /// this method encodes that invariant. If no ready request can advance
    /// legally at `now`, the subchannel goes idle for this cycle.
    fn tick_subchannel(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) {
        self.update_drain_state(chan, subch);
        if self.command_bus_busy(chan, subch, now) {
            return;
        }
        if self.try_issue_refresh(chan, subch, now) {
            return;
        }
        let Some(pick_writes) = self.pick_queue(chan, subch) else { return };
        let Some(index) = self.pick_request_index(chan, subch, pick_writes, now) else {
            return;
        };
        self.step_request(chan, subch, pick_writes, index, now);
    }

    /// True iff the subchannel already emitted a command whose fire cycle is
    /// `now` or later — the command bus is occupied for this cycle.
    fn command_bus_busy(&self, chan: ChannelId, subch: SubchannelId, now: u64) -> bool {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        sc.last_command_cycle > now
    }

    /// Picks the queue index whose request can start earliest given current
    /// state. Ties broken by arrival order (i.e. lower index wins).
    fn pick_request_index(
        &self,
        chan: ChannelId,
        subch: SubchannelId,
        pick_writes: bool,
        now: u64,
    ) -> Option<usize> {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        let queue = if pick_writes { &sc.write_queue } else { &sc.read_queue };
        if queue.is_empty() {
            return None;
        }
        let mut best_index = 0usize;
        let mut best_start = u64::MAX;
        for (i, req) in queue.iter().enumerate() {
            if req.arrival_cycle > now {
                continue;
            }
            let start = self.estimated_start(chan, subch, req, now);
            if start < best_start {
                best_start = start;
                best_index = i;
            }
        }
        if best_start == u64::MAX { None } else { Some(best_index) }
    }

    /// Rough estimate of the earliest cycle at which `req` could fire its
    /// first command. Used only for reorder priority — the real timing is
    /// (re)computed in `step_request`. Only needs to reflect the biggest
    /// bottleneck (refresh, precharge, activate), not the full column pipeline.
    fn estimated_start(
        &self,
        chan: ChannelId,
        subch: SubchannelId,
        req: &PendingReq,
        now: u64,
    ) -> u64 {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        let rank = &sc.ranks[req.loc.rank.as_index()];
        let refresh_bound = if rank.next_refresh <= now {
            rank.next_refresh + self.config.timing.t_rfc()
        } else {
            rank.refresh_end
        };
        now.max(sc.last_command_cycle).max(rank.last_command_cycle).max(refresh_bound)
    }

    fn update_drain_state(&mut self, chan: ChannelId, subch: SubchannelId) {
        let high = self.config.write_high_watermark;
        let low = self.config.write_low_watermark;
        let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
        let depth = sc.write_queue.len();
        sc.drain_state = match sc.drain_state {
            WriteDrainState::Filling if depth >= high => WriteDrainState::Draining,
            WriteDrainState::Draining if depth <= low => WriteDrainState::Filling,
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

    /// Attempts to fire an all-bank refresh on any rank whose window has
    /// opened by `now`. Returns `true` iff a REFRESH command was issued.
    fn try_issue_refresh(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) -> bool {
        let t_refi = self.config.timing.t_refi;
        if t_refi == 0 {
            return false;
        }
        let t_rfc = self.config.timing.t_rfc();
        let rank_count = self.channels[chan.as_index()].subchannels[subch.as_index()].ranks.len();
        for rank_idx in 0..rank_count {
            let (due, earliest) = {
                let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
                let rank_ref = &sc.ranks[rank_idx];
                let due = rank_ref.next_refresh <= now;
                let earliest = rank_ref
                    .next_refresh
                    .max(rank_ref.refresh_end)
                    .max(rank_ref.last_command_cycle)
                    .max(sc.last_command_cycle);
                (due, earliest)
            };
            if !due || earliest > now {
                continue;
            }
            let rank = RankId::new(index_to_u8(rank_idx));
            let fire_at = earliest.max(now);
            let refresh_end = fire_at + t_rfc;
            self.commit_refresh(chan, subch, rank, fire_at, refresh_end);
            return true;
        }
        false
    }

    fn commit_refresh(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        rank: RankId,
        start: u64,
        end: u64,
    ) {
        let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
        sc.last_command_cycle = sc.last_command_cycle.max(start + 1);
        let rank_mut = &mut sc.ranks[rank.as_index()];
        rank_mut.last_command_cycle = rank_mut.last_command_cycle.max(start + 1);
        rank_mut.refresh_end = end;
        rank_mut.next_refresh += self.config.timing.t_refi;
        for bank in &mut rank_mut.banks {
            bank.open_row = None;
            bank.state = BankState::Idle;
        }
        self.pending_commands.push(EmittedCommand {
            channel: chan,
            rank,
            bank: 0,
            row: 0,
            kind: DramCmdKind::Refresh,
            fire_at: start,
        });
    }

    /// Advances a single request by one command step. Removes it from its
    /// queue only when the column command has just been issued (final step).
    /// Does nothing if the next command's earliest legal cycle exceeds `now`;
    /// the request stays in the queue for a subsequent tick.
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
        let is_read = matches!(request.op, MemOp::Read | MemOp::Fetch | MemOp::Atomic { .. });
        match snapshot.state {
            BankState::Active if snapshot.open_row == Some(request.loc.row) => {
                self.try_issue_column(&ctx, pick_writes, index, &request, is_read, now);
            }
            BankState::Active => {
                self.try_issue_precharge(&ctx, &snapshot, now);
            }
            BankState::Precharging => {
                let earliest = snapshot.last_precharge + self.config.timing.t_rp;
                self.try_issue_activate(&ctx, earliest, now);
            }
            BankState::Idle => {
                self.try_issue_activate(&ctx, 0, now);
            }
            // Refreshing / Activating: nothing to issue this cycle; the bank
            // is mid-transition and will become ready on a later tick.
            BankState::Refreshing | BankState::Activating => {}
        }
    }

    /// Issues PRECHARGE if legal at `now`; otherwise leaves the bank alone.
    fn try_issue_precharge(&mut self, ctx: &BankCmdCtx, snapshot: &Bank, now: u64) {
        let t = &self.config.timing;
        let ras_bound = snapshot.last_activate + t.t_ras;
        let rtp_bound =
            if snapshot.last_read_cmd == 0 { 0 } else { snapshot.last_read_cmd + t.t_rtp };
        let wr_bound =
            if snapshot.last_write_end == 0 { 0 } else { snapshot.last_write_end + t.t_wr };
        let cmd_bus = self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()]
            .ranks[ctx.rank.as_index()]
            .last_command_cycle;
        let earliest = ras_bound.max(rtp_bound).max(wr_bound).max(cmd_bus);
        if earliest > now {
            return;
        }
        let fire_at = earliest.max(now);
        debug_assert!(
            fire_at >= snapshot.last_activate + t.t_ras,
            "tRAS violation: PRE at {fire_at}, ACT at {}, tRAS={}",
            snapshot.last_activate,
            t.t_ras,
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
            sc.last_command_cycle = sc.last_command_cycle.max(fire_at + 1);
            let rank_mut = &mut sc.ranks[ctx.rank.as_index()];
            rank_mut.last_command_cycle = rank_mut.last_command_cycle.max(fire_at + 1);
            let bank = &mut rank_mut.banks[ctx.bank_index];
            bank.state = BankState::Precharging;
            bank.last_precharge = fire_at;
            bank.open_row = None;
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

    /// Issues ACTIVATE if legal at `now`; `not_before` covers state-dependent
    /// bounds already known by the caller (e.g. `last_precharge + tRP`).
    fn try_issue_activate(&mut self, ctx: &BankCmdCtx, not_before: u64, now: u64) {
        let t = self.config.timing;
        let banks_per_group = usize::from(self.config.banks_per_group);
        let (rrd_bound, tc_bound, faw_bound, refresh_end, cmd_bus) = {
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
            let tc = if same.last_activate == 0 { 0 } else { same.last_activate + t.t_rc };
            let faw = rank_ref.earliest_activate_faw(t.t_faw);
            (rrd, tc, faw, rank_ref.refresh_end, rank_ref.last_command_cycle)
        };
        let earliest = not_before
            .max(cmd_bus)
            .max(rrd_bound)
            .max(tc_bound)
            .max(faw_bound)
            .max(refresh_end);
        if earliest > now {
            return;
        }
        let fire_at = earliest.max(now);
        debug_assert!(
            fire_at >= rrd_bound,
            "tRRD violation: ACT at {fire_at}, min {rrd_bound}",
        );
        debug_assert!(
            fire_at >= faw_bound,
            "tFAW violation: ACT at {fire_at}, faw min {faw_bound}",
        );
        debug_assert!(
            fire_at >= tc_bound,
            "tRC violation: ACT at {fire_at}, prior ACT + tRC = {tc_bound}",
        );
        self.commit_activate(ctx, fire_at);
    }

    fn commit_activate(&mut self, ctx: &BankCmdCtx, fire_at: u64) {
        let t = self.config.timing;
        {
            let sc = &mut self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()];
            sc.last_command_cycle = sc.last_command_cycle.max(fire_at + 1);
            let rank_mut = &mut sc.ranks[ctx.rank.as_index()];
            rank_mut.last_command_cycle = rank_mut.last_command_cycle.max(fire_at + 1);
            rank_mut.record_activate(fire_at, t.t_faw);
            let bank = &mut rank_mut.banks[ctx.bank_index];
            bank.state = BankState::Active;
            bank.open_row = Some(ctx.row);
            bank.last_activate = fire_at;
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
    /// removes the request from its queue and schedules the response.
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
        let rcd_bound = bank.last_activate + self.config.timing.t_rcd;
        let column_earliest =
            self.column_earliest(ctx.chan, ctx.subch, ctx.rank, ctx.bg, rcd_bound, is_read);
        let data_earliest =
            self.data_bus_start(ctx.chan, ctx.subch, ctx.rank, column_earliest, is_read);
        let (column_aligned, data_start_aligned) =
            self.align_column_to_data_bus(column_earliest, data_earliest, is_read);
        if column_aligned > now {
            return;
        }
        let fire_at = column_aligned.max(now);
        let lead = column_lead(&self.config.timing, is_read);
        let data_start = (fire_at + lead).max(data_start_aligned);
        let data_end = data_start + self.config.timing.bl_half;
        self.commit_column(ctx, fire_at, data_start, data_end, is_read);
        let payload = self.service_buffer(request);
        let response = ScheduledResponse::for_request(request, data_end, payload);
        self.pending_responses.push(response);
        self.pop_request(ctx.chan, ctx.subch, pick_writes, index);
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
            sc.last_command_cycle = sc.last_command_cycle.max(column_cycle + 1);
            sc.ranks[ctx.rank.as_index()].last_command_cycle =
                sc.ranks[ctx.rank.as_index()].last_command_cycle.max(column_cycle + 1);
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

    /// Earliest column-command cycle honoring tCCD, tWTR/tRTW, and command-bus
    /// availability. `not_before` is the caller-supplied lower bound (typically
    /// `last_activate + tRCD`).
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
        let mut result = not_before.max(rank_ref.last_command_cycle).max(ccd);
        if is_read && sc.last_data_op == BusOp::Write && sc.last_write_end > 0 {
            let same_bg = sc.last_column_bg == Some(bg);
            let bound = if same_bg {
                sc.last_write_end + t.t_wtr_l
            } else {
                sc.last_write_end + t.t_wtr_s
            };
            result = result.max(bound);
        }
        if !is_read && sc.last_data_op == BusOp::Read && sc.last_read_cmd > 0 {
            result = result.max(sc.last_read_cmd + t.t_rtw);
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
        earliest
    }

    /// Walks the column-command cycle forward so RD→data == tCAS (WR→data == tCWL)
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
            MemOp::Write { .. } => MemRespData::Small(0),
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

    fn flush(&mut self, ctx: &mut HandleCtx<'_>) {
        let self_component = ComponentId::MemCtrl(self.self_id);
        for cmd in self.pending_commands.drain(..) {
            ctx.scheduler.schedule(
                cmd.fire_at,
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
                resp.fire_at,
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
    fire_at: u64,
    target: ComponentId,
}

impl ScheduledResponse {
    const fn for_request(request: &PendingReq, fire_at: u64, data: MemRespData) -> Self {
        Self {
            req_id: request.req_id,
            line_addr: LineAddr::from_phys(request.paddr, CACHE_LINE_BYTES),
            data,
            hit_level: HitLevel::Dram,
            fire_at,
            target: request.source,
        }
    }
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
