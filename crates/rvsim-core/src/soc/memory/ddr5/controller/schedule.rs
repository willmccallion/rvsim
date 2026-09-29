//! Admitting requests to the queues and choosing which to serve next.

use crate::sim::components::{ChannelId, SubchannelId};
use crate::soc::memory::ddr5::scheduler::Candidate;
use crate::soc::memory::ddr5::state::{BankState, PendingReq, WriteDrainState};

use super::Ddr5Controller;
use super::{BankCmdCtx, Payload, ScheduledResponse, is_read_op};

impl Ddr5Controller {
    /// Moves arrived requests from `inbound` into the read / write queues in
    /// arrival order, stopping at the first one its queue cannot hold.
    /// Writes are acknowledged on admission; a read whose line is still in
    /// the write queue is answered from the queue.
    pub(super) fn admit(&mut self, chan: ChannelId, subch: SubchannelId, now: u64) {
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
                Payload::acknowledging(&request),
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

    /// True iff the subchannel already emitted a command whose bus tenure
    /// covers `now`.
    pub(super) fn command_bus_busy(&self, chan: ChannelId, subch: SubchannelId, now: u64) -> bool {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        sc.last_command_cycle > now
    }

    /// Asks the scheduler which request in the chosen queue to advance.
    pub(super) fn pick_request_index(
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
    pub(super) fn candidate(
        &self,
        chan: ChannelId,
        subch: SubchannelId,
        req: &PendingReq,
    ) -> Candidate {
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

    pub(super) fn update_drain_state(&mut self, chan: ChannelId, subch: SubchannelId) {
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
            WriteDrainState::Draining if depth <= low && sc.writes_this_drain >= min_writes => {
                WriteDrainState::Filling
            }
            state => state,
        };
    }

    pub(super) fn pick_queue(&self, chan: ChannelId, subch: SubchannelId) -> Option<bool> {
        let sc = &self.channels[chan.as_index()].subchannels[subch.as_index()];
        match (sc.read_queue.is_empty(), sc.write_queue.is_empty()) {
            (true, true) => None,
            (false, true) => Some(false),
            (true, false) => Some(true),
            (false, false) => Some(matches!(sc.drain_state, WriteDrainState::Draining)),
        }
    }

    /// Advances a single request by one command step. Removes it from its
    /// queue only when the column command has just been issued (final step).
    /// Does nothing if the next command's earliest legal clock exceeds `now`;
    /// the request stays in the queue for a subsequent clock.
    pub(super) fn step_request(
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

    pub(super) fn pop_request(
        &mut self,
        chan: ChannelId,
        subch: SubchannelId,
        pick_writes: bool,
        index: usize,
    ) {
        let sc = &mut self.channels[chan.as_index()].subchannels[subch.as_index()];
        let _ =
            if pick_writes { sc.write_queue.remove(index) } else { sc.read_queue.remove(index) };
    }
}
