//! Rank power-down entry and exit.

use crate::config::ddr5::PowerDownPolicy;
use crate::sim::components::{ChannelId, RankId, SubchannelId};
use crate::sim::packet::DramCmdKind;
use crate::soc::memory::ddr5::state::{BankState, PowerState, Rank, RefreshPhase, Subchannel};

use super::Ddr5Controller;
use super::{EmittedCommand, POWER_CMD_CYCLES, index_to_u8};

impl Ddr5Controller {
    /// The first DRAM clock at or after `now` at which an idle, active
    /// `rank` could enter power-down, if it could before its next refresh.
    pub(super) fn power_down_entry(
        &self,
        subchannel: &Subchannel,
        rank: &Rank,
        now: u64,
    ) -> Option<u64> {
        let PowerDownPolicy::AfterIdle { idle_clocks } = self.config.power_down else {
            return None;
        };
        if rank.power != PowerState::Active {
            return None;
        }
        let entry = (rank.command_floor() + idle_clocks).max(subchannel.last_data_end).max(now);
        let blocked_by_refresh =
            self.refresh_interval > 0 && rank.next_refresh <= entry + self.config.timing.t_pd;
        (!blocked_by_refresh).then_some(entry)
    }

    /// Drives every rank's power state for one clock: exits power-down when
    /// the rank has work (a queued request or a due refresh) and tPD has
    /// elapsed; enters it when the rank has been idle for the policy's
    /// timer. Returns `true` iff a power command was issued.
    pub(super) fn advance_power(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) -> bool {
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
                    rank_mut.last_command_cycle =
                        rank_mut.last_command_cycle.max(now + POWER_CMD_CYCLES);
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
                    rank_mut.power = PowerState::PowerDown {
                        since: now,
                        with_open_rows: rank_mut.has_open_row(),
                    };
                    rank_mut.last_command_cycle =
                        rank_mut.last_command_cycle.max(now + POWER_CMD_CYCLES);
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
    pub(super) fn rank_has_work(
        &self,
        chan: ChannelId,
        subch: SubchannelId,
        rank: RankId,
        now: u64,
    ) -> bool {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        let rank_ref = &sc.ranks[rank.as_index()];
        rank_ref.next_refresh <= now
            || rank_ref.refresh_phase != RefreshPhase::Idle
            || sc.read_queue.iter().chain(sc.write_queue.iter()).any(|r| r.loc.rank == rank)
    }
}
