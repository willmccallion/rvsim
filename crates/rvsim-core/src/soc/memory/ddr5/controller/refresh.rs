//! Refresh: arming due refreshes, precharging for them, and issuing them.

use crate::sim::components::{ChannelId, RankId, SubchannelId};
use crate::sim::packet::DramCmdKind;
use crate::soc::memory::ddr5::state::{BankState, RefreshPhase};

use super::Ddr5Controller;
use super::{EmittedCommand, PRECHARGE_CMD_CYCLES, REFRESH_CMD_CYCLES, index_to_u8, mask_has};

impl Ddr5Controller {
    /// Returns banks whose refresh has completed to the idle state.
    pub(super) fn release_refreshed_banks(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        now: u64,
    ) {
        let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
        for rank in &mut sc.ranks {
            for bank in &mut rank.banks {
                if bank.state == BankState::Refreshing && bank.refresh_end <= now {
                    bank.state = BankState::Idle;
                }
            }
        }
    }

    /// Drives every rank's refresh state machine for one clock. A rank
    /// whose refresh is due first stops taking new commands for the
    /// covered banks, precharges any open rows among them (PRECHARGE-ALL,
    /// once tRAS / tRTP / tWR allow), then issues REFRESH once tRP has
    /// elapsed. Returns `true` iff a command was issued.
    pub(super) fn advance_refresh(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        now: u64,
    ) -> bool {
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
    pub(super) fn arm_due_refresh(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        rank: RankId,
        now: u64,
    ) {
        let timing = self.config.timing;
        let layout = self.layout;
        let rank_mut = &mut self.channels[chan.as_index()].subchannels[subch.as_index()].ranks
            [rank.as_index()];
        if rank_mut.refresh_phase != RefreshPhase::Idle || rank_mut.next_refresh > now {
            return;
        }
        let target = self.refresh_policy.target(&timing, layout, rank_mut.refresh_seq);
        rank_mut.refresh_phase =
            RefreshPhase::Pending { bank_mask: target.bank_mask, duration: target.duration };
    }

    /// Precharges the open rows a pending refresh covers as soon as every
    /// one of them may legally close. Returns `true` iff PRECHARGE-ALL issued.
    pub(super) fn try_precharge_for_refresh(
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
    pub(super) fn try_issue_refresh(
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
}
