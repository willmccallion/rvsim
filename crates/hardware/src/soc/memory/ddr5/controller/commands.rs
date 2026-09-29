//! Issuing PRECHARGE, ACTIVATE and column commands under the JEDEC timing
//! constraints, and placing their data on the bus.

use crate::sim::components::{BankGroupId, ChannelId, RankId, RowId, SubchannelId};
use crate::sim::packet::DramCmdKind;
use crate::soc::memory::ddr5::state::{Bank, BankState, BusOp, PendingReq};

use super::Ddr5Controller;
use super::{
    ACT_CMD_CYCLES, ActivateBounds, BankCmdCtx, COLUMN_CMD_CYCLES, EmittedCommand,
    PRECHARGE_CMD_CYCLES, ScheduledResponse, bank_index_u8, column_lead, index_to_u8,
};

impl Ddr5Controller {
    pub(super) fn bank_ctx(
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

    /// Earliest clock a PRECHARGE of `snapshot`'s bank is legal: tRAS from
    /// its ACT, tRTP from its last read, tWR from its last write burst,
    /// tPPD from the rank's last precharge, and the rank's command bus.
    pub(super) fn precharge_earliest(&self, ctx: &BankCmdCtx, snapshot: &Bank) -> u64 {
        let t = &self.config.timing;
        let ras_bound = snapshot.last_activate + t.t_ras;
        let rtp_bound =
            if snapshot.last_read_cmd == 0 { 0 } else { snapshot.last_read_cmd + t.t_rtp };
        let wr_bound =
            if snapshot.last_write_end == 0 { 0 } else { snapshot.last_write_end + t.t_wr };
        let rank_ref = &self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()].ranks
            [ctx.rank.as_index()];
        let ppd_bound =
            if rank_ref.last_precharge == 0 { 0 } else { rank_ref.last_precharge + t.t_ppd };
        ras_bound.max(rtp_bound).max(wr_bound).max(rank_ref.command_floor()).max(ppd_bound)
    }

    /// Issues PRECHARGE if legal at `now`; otherwise leaves the bank alone.
    pub(super) fn try_issue_precharge(&mut self, ctx: &BankCmdCtx, snapshot: &Bank, now: u64) {
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

    pub(super) fn commit_precharge(&mut self, ctx: &BankCmdCtx, fire_at: u64) {
        let open_row = self.channels[ctx.chan.as_index()].subchannels[ctx.subch.as_index()].ranks
            [ctx.rank.as_index()]
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
    pub(super) fn activate_bounds(&self, ctx: &BankCmdCtx) -> ActivateBounds {
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
    pub(super) fn activate_earliest(&self, ctx: &BankCmdCtx, not_before: u64) -> u64 {
        self.activate_bounds(ctx).earliest(not_before)
    }

    /// Issues ACTIVATE if legal at `now`; `not_before` covers state-dependent
    /// bounds already known by the caller (e.g. `last_precharge + tRP`).
    /// Returns `true` iff the command issued.
    pub(super) fn try_issue_activate(
        &mut self,
        ctx: &BankCmdCtx,
        not_before: u64,
        now: u64,
    ) -> bool {
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

    pub(super) fn commit_activate(&mut self, ctx: &BankCmdCtx, fire_at: u64) {
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
    pub(super) fn try_issue_column(
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
        let data_start_aligned =
            self.data_bus_start(ctx.chan, ctx.subch, ctx.rank, fire_at, is_read);
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
    pub(super) fn account_column(
        &mut self,
        ctx: &BankCmdCtx,
        request: &PendingReq,
        is_read: bool,
        data_end: u64,
    ) {
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
                sc.counters
                    .read_latency_samples
                    .push(data_end.saturating_sub(request.arrival_cycle));
            }
        } else {
            bank.counters.writes += 1;
            sc.writes_this_drain += 1;
        }
        sc.counters.bus_busy_clocks += bl_half;
    }

    pub(super) fn commit_column(
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
    pub(super) fn column_issue_earliest(
        &self,
        ctx: &BankCmdCtx,
        bank: &Bank,
        is_read: bool,
    ) -> u64 {
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
    pub(super) fn column_earliest(
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
            let spacing =
                if same_bg { if is_read { t.t_ccd_l } else { t.t_ccd_l_wr } } else { t.t_ccd_s };
            last_col + spacing
        };
        let mut result = not_before.max(rank_ref.command_floor()).max(ccd);
        if is_read && sc.last_data_op == BusOp::Write && sc.last_write_end > 0 {
            let same_bg = sc.last_column_bg == Some(bg);
            let bound =
                if same_bg { sc.last_write_end + t.t_wtr_l } else { sc.last_write_end + t.t_wtr_s };
            result = result.max(bound);
        }
        result
    }

    pub(super) fn data_bus_start(
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
    pub(super) const fn align_column_to_data_bus(
        &self,
        column: u64,
        data_start: u64,
        is_read: bool,
    ) -> (u64, u64) {
        let lead = column_lead(&self.config.timing, is_read);
        if data_start > column + lead {
            let new_col = data_start - lead;
            (new_col, data_start)
        } else {
            (column, column + lead)
        }
    }
}
