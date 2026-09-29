//! The coherence fabric: the home agent's transaction engine on top of a
//! tracking policy and an interconnect.
//!
//! Requests from the private L2s arrive as [`Packet::Coh`] events, travel
//! through the interconnect to the home, and become transactions: snoops
//! go out, responses come back, data is fetched from the LLC (an ordinary
//! cache level the fabric talks to with `MemReq` packets) or forwarded
//! from the owner that answered dirty, and a completion travels back.
//! One transaction is live per line; later requests for the line queue
//! behind it, which is the serialisation point that makes the protocol's
//! invariants hold.

use std::collections::VecDeque;

use super::home::{HomeAgent, Room};
use super::interconnect::Interconnect;
use super::protocol::{CoherenceProtocol, CoreSet, Holders};
use super::stats::CoherenceStatPaths;
use crate::common::{CoreId, LineAddr};
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::coherence::{CoherenceMsg, Node, ReqKind, SnoopKind};
use crate::sim::packet::{
    AccessSize, Maintenance, MemOp, MemRespData, MesiState, Packet, WriteData,
};

/// Where a transaction is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Waiting for a recall of `victim` to free tracking room.
    Recalling {
        /// The recalled line.
        victim: LineAddr,
    },
    /// Waiting for snoop responses.
    Snooping,
    /// Waiting for the LLC's data.
    FetchingData,
    /// Waiting for the LLC to acknowledge a writeback.
    WritingBack,
    /// Waiting for the LLC and memory to acknowledge a maintenance
    /// operation.
    Maintaining,
    /// Completion sent; waiting for the requester to acknowledge it before
    /// the line is released.
    AwaitingAck,
}

/// What a transaction does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TxnKind {
    /// A requester's request.
    Request {
        /// What was asked.
        kind: ReqKind,
        /// Who asked.
        requester: CoreId,
    },
    /// The home recalling a line to free tracking room.
    Recall,
}

/// One live transaction.
#[derive(Clone, Copy, Debug)]
struct Txn {
    /// The requester's correlator for requests, a fabric id for recalls.
    id: ReqId,
    line: LineAddr,
    kind: TxnKind,
    phase: Phase,
    /// Correlator of the request this fabric sent the LLC, if any.
    llc_req: Option<ReqId>,
    snoops_outstanding: usize,
    /// Some snooped core keeps a shared copy.
    others_remain: bool,
    /// A snooped owner answered with modified data.
    dirty_from: Option<CoreId>,
    started_at: u64,
}

/// A request waiting for its line or for a transaction entry.
#[derive(Clone, Copy, Debug)]
struct Waiting {
    msg: CoherenceMsg,
    arrived: u64,
}

/// A non-coherent access parked at the fabric while its marker crosses
/// the interconnect: the request on its way in (then at the LLC, with no
/// packet held), or the response on its way back.
#[derive(Debug)]
struct Parked {
    txn: ReqId,
    requester: CoreId,
    packet: Option<Packet>,
}

/// Where the fabric sits: the LLC it fetches from, the agents it serves,
/// and its sizes.
#[derive(Clone, Debug)]
pub struct FabricLayout {
    /// The LLC the home fetches data from and writes back into.
    pub llc: ComponentId,
    /// Each core's requesting agent (its L2), indexed by `CoreId`.
    pub agents: Vec<ComponentId>,
    /// Cache line size.
    pub line_bytes: usize,
    /// Transactions the home can have live at once.
    pub txn_capacity: usize,
}

/// Home agent plus interconnect.
pub struct CoherenceFabric {
    protocol: Box<dyn CoherenceProtocol>,
    tracking: Box<dyn HomeAgent>,
    interconnect: Box<dyn Interconnect>,
    /// The LLC the home fetches data from and writes back into.
    llc: ComponentId,
    /// Each core's requesting agent (its L2), indexed by `CoreId`.
    agents: Vec<ComponentId>,
    line_bytes: usize,
    txns: Vec<Txn>,
    txn_capacity: usize,
    waiting: VecDeque<Waiting>,
    /// Non-coherent requests on their way to the home.
    parked_requests: Vec<Parked>,
    /// Non-coherent responses on their way back to a core.
    parked_responses: Vec<Parked>,
    next_seq: u64,
    stat_paths: CoherenceStatPaths,
}

impl std::fmt::Debug for CoherenceFabric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoherenceFabric")
            .field("protocol", &self.protocol.name())
            .field("tracking", &self.tracking.name())
            .field("interconnect", &self.interconnect.topology())
            .field("txns", &self.txns)
            .field("waiting", &self.waiting)
            .field("parked_requests", &self.parked_requests.len())
            .field("parked_responses", &self.parked_responses.len())
            .finish_non_exhaustive()
    }
}

impl CoherenceFabric {
    /// Builds a fabric on `layout`.
    #[must_use]
    pub fn new(
        protocol: Box<dyn CoherenceProtocol>,
        tracking: Box<dyn HomeAgent>,
        interconnect: Box<dyn Interconnect>,
        layout: FabricLayout,
        stat_paths: CoherenceStatPaths,
    ) -> Self {
        Self {
            protocol,
            tracking,
            interconnect,
            llc: layout.llc,
            agents: layout.agents,
            line_bytes: layout.line_bytes,
            txns: Vec::new(),
            txn_capacity: layout.txn_capacity.max(1),
            waiting: VecDeque::new(),
            parked_requests: Vec::new(),
            parked_responses: Vec::new(),
            next_seq: 0,
            stat_paths,
        }
    }

    /// Stat paths of the fabric.
    #[must_use]
    pub const fn stat_paths(&self) -> &CoherenceStatPaths {
        &self.stat_paths
    }

    /// Exact holders of `line` as the home tracks them, when tracked
    /// exactly.
    #[must_use]
    pub fn tracked_holders(&self, line: LineAddr) -> Option<Holders> {
        self.tracking.exact_holders(line)
    }

    /// Tells the home that every core dropped every line, after the caches
    /// were emptied behind its back (a checkpoint restore).
    pub fn forget_cached_lines(&mut self) {
        self.tracking.forget_all();
    }

    /// Every line the home tracks, when it tracks exactly.
    #[must_use]
    pub fn tracked_lines(&self) -> Option<Vec<LineAddr>> {
        self.tracking.tracked_lines()
    }

    /// Lines with a live transaction or a queued request.
    #[must_use]
    pub fn lines_in_flight(&self) -> Vec<LineAddr> {
        self.txns.iter().map(|t| t.line).chain(self.waiting.iter().map(|w| w.msg.line())).collect()
    }

    /// True when no transaction, queued request or message is pending.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.txns.is_empty()
            && self.waiting.is_empty()
            && self.parked_requests.is_empty()
            && self.parked_responses.is_empty()
            && self.interconnect.is_idle()
    }

    /// True when idle and ticking at cycle `now` or later changes nothing.
    #[must_use]
    pub fn is_quiet(&self, now: u64) -> bool {
        self.is_idle() && self.interconnect.is_quiet(now)
    }

    /// Advances the interconnect one cycle and processes what arrives.
    pub fn tick(&mut self, ctx: &mut HandleCtx<'_>) {
        let mut arrived = Vec::new();
        self.interconnect.tick(ctx.cycle, ctx.stats, &mut |to, msg| arrived.push((to, msg)));
        for (to, msg) in arrived {
            match (to, msg) {
                (Node::Home, msg) => self.on_home_message(msg, ctx),
                (Node::Core(_), CoherenceMsg::NoSnpData { txn, .. }) => {
                    self.deliver_parked_response(txn, ctx);
                }
                (Node::Core(core), msg) => {
                    let agent = self.agents[core.as_index()];
                    ctx.scheduler.schedule(ctx.cycle, agent, ctx.self_id, Packet::Coh(msg));
                }
            }
        }
    }

    const fn fabric_req_id(&mut self) -> ReqId {
        let seq = self.next_seq;
        self.next_seq += 1;
        ReqId::for_fabric(seq)
    }

    fn line_busy(&self, line: LineAddr) -> bool {
        self.txns.iter().any(|t| t.line == line)
    }

    fn txn_mut(&mut self, id: ReqId) -> Option<&mut Txn> {
        self.txns.iter_mut().find(|t| t.id == id)
    }

    fn all_cores(&self) -> CoreSet {
        let mut set = CoreSet::EMPTY;
        for index in 0..self.agents.len() {
            set.insert(CoreId::new(u32::try_from(index).unwrap_or(u32::MAX)));
        }
        set
    }

    /// A message delivered to the home.
    fn on_home_message(&mut self, msg: CoherenceMsg, ctx: &mut HandleCtx<'_>) {
        match msg {
            CoherenceMsg::Req { .. } => self.admit(Waiting { msg, arrived: ctx.cycle }, ctx),
            CoherenceMsg::SnoopResp { txn, from, had_copy, dirty, .. } => {
                self.on_snoop_resp(txn, from, had_copy, dirty, ctx);
            }
            CoherenceMsg::CompAck { txn, .. } => self.on_comp_ack(txn, ctx),
            CoherenceMsg::NoSnp { txn, .. } => self.forward_parked_request(txn, ctx),
            CoherenceMsg::Snoop { .. }
            | CoherenceMsg::CompData { .. }
            | CoherenceMsg::Comp { .. }
            | CoherenceMsg::NoSnpData { .. } => {}
        }
    }

    /// Starts a request's transaction, or queues it while its line is busy
    /// or the transaction table is full.
    fn admit(&mut self, waiting: Waiting, ctx: &mut HandleCtx<'_>) {
        let CoherenceMsg::Req { txn, line, kind, requester } = waiting.msg else { return };
        if kind == ReqKind::Evict {
            ctx.stats.counter(self.stat_paths.home.evicts).inc();
            self.tracking.on_release(line, requester);
            return;
        }
        if self.line_busy(line) {
            ctx.stats.counter(self.stat_paths.home.serialised).inc();
            self.waiting.push_back(waiting);
            return;
        }
        if self.txns.len() >= self.txn_capacity {
            ctx.stats.counter(self.stat_paths.home.txn_full_stalls).inc();
            self.waiting.push_back(waiting);
            return;
        }
        let stat = match kind {
            ReqKind::ReadShared => self.stat_paths.home.read_shared,
            ReqKind::ReadUnique => self.stat_paths.home.read_unique,
            ReqKind::CleanUnique => self.stat_paths.home.clean_unique,
            ReqKind::WriteBack { .. } => self.stat_paths.home.writebacks,
            ReqKind::Evict => self.stat_paths.home.evicts,
            ReqKind::Maintain { .. } => self.stat_paths.home.maintenance,
        };
        ctx.stats.counter(stat).inc();
        let mut txn = Txn {
            id: txn,
            line,
            kind: TxnKind::Request { kind, requester },
            phase: Phase::Snooping,
            llc_req: None,
            snoops_outstanding: 0,
            others_remain: false,
            dirty_from: None,
            started_at: waiting.arrived,
        };
        match kind {
            ReqKind::WriteBack { dirty } => {
                // A snoop that already collected the line made this
                // writeback stale: its data went with the snoop response.
                let stale = self
                    .tracking
                    .exact_holders(line)
                    .is_some_and(|h| !h.sharers.contains(requester));
                self.tracking.on_release(line, requester);
                if stale {
                    ctx.stats.counter(self.stat_paths.home.stale_writebacks).inc();
                    self.txns.push(txn);
                    let comp = CoherenceMsg::Comp {
                        txn: txn.id,
                        line,
                        to: requester,
                        state: MesiState::Invalid,
                    };
                    self.complete(txn.id, comp, MesiState::Invalid, ctx);
                    return;
                }
                let llc_req = self.fabric_req_id();
                txn.llc_req = Some(llc_req);
                txn.phase = Phase::WritingBack;
                self.txns.push(txn);
                self.send_llc(llc_req, line, MemOp::Writeback { dirty }, ctx);
            }
            ReqKind::Evict => {}
            ReqKind::Maintain { op, .. } => {
                // The requester dropped its copy before asking; a clean
                // leaves it holding the line as before.
                if op != Maintenance::Clean {
                    self.tracking.on_release(line, requester);
                }
                self.txns.push(txn);
                self.start_snoops(txn.id, ctx);
            }
            ReqKind::ReadShared | ReqKind::ReadUnique | ReqKind::CleanUnique => {
                let in_flight: Vec<LineAddr> = self.txns.iter().map(|t| t.line).collect();
                match self.tracking.room_for(line, &in_flight) {
                    Room::Available => {
                        self.txns.push(txn);
                        self.start_snoops(txn.id, ctx);
                    }
                    Room::Recall(victim) => {
                        txn.phase = Phase::Recalling { victim };
                        self.txns.push(txn);
                        self.start_recall(victim, ctx);
                    }
                    Room::AllBusy => {
                        ctx.stats.counter(self.stat_paths.home.txn_full_stalls).inc();
                        self.waiting.push_back(waiting);
                    }
                }
            }
        }
    }

    /// Invalidates every holder of `victim` so its tracking entry can be
    /// reused.
    fn start_recall(&mut self, victim: LineAddr, ctx: &mut HandleCtx<'_>) {
        ctx.stats.counter(self.stat_paths.home.recalls).inc();
        let id = self.fabric_req_id();
        let holders =
            self.tracking.holders(victim, ctx.stats, &self.stat_paths.home).unwrap_or_default();
        let mut txn = Txn {
            id,
            line: victim,
            kind: TxnKind::Recall,
            phase: Phase::Snooping,
            llc_req: None,
            snoops_outstanding: 0,
            others_remain: false,
            dirty_from: None,
            started_at: ctx.cycle,
        };
        let targets: Vec<CoreId> = holders.sharers.iter().collect();
        txn.snoops_outstanding = targets.len();
        self.txns.push(txn);
        for core in targets {
            self.send_snoop(id, victim, SnoopKind::Invalid, core, ctx);
        }
        if self.txn_mut(id).is_some_and(|t| t.snoops_outstanding == 0) {
            self.complete_recall(id, ctx);
        }
    }

    /// Sends the snoops a request needs; goes straight to the data phase
    /// when there are none.
    fn start_snoops(&mut self, id: ReqId, ctx: &mut HandleCtx<'_>) {
        let Some(txn) = self.txn_mut(id).copied() else { return };
        let TxnKind::Request { kind, requester } = txn.kind else { return };
        let holders = self.tracking.holders(txn.line, ctx.stats, &self.stat_paths.home);
        let snoops = if let Some(holders) = holders {
            self.protocol.snoops_for(kind, requester, holders)
        } else {
            let snoop_kind = broadcast_snoop(kind);
            self.all_cores().without(requester).iter().map(|core| (core, snoop_kind)).collect()
        };
        // Sharers a read need not snoop still keep their copies.
        let sharers_stay = kind == ReqKind::ReadShared
            && holders.is_some_and(|h| !h.sharers.without(requester).is_empty());
        if let Some(t) = self.txn_mut(id) {
            t.phase = Phase::Snooping;
            t.snoops_outstanding = snoops.len();
            t.others_remain = sharers_stay;
        }
        for (core, snoop_kind) in snoops {
            self.send_snoop(id, txn.line, snoop_kind, core, ctx);
        }
        if self.txn_mut(id).is_some_and(|t| t.snoops_outstanding == 0) {
            self.start_data_phase(id, ctx);
        }
    }

    fn send_snoop(
        &mut self,
        txn: ReqId,
        line: LineAddr,
        kind: SnoopKind,
        target: CoreId,
        ctx: &mut HandleCtx<'_>,
    ) {
        ctx.stats.counter(self.stat_paths.home.snoops_sent).inc();
        self.interconnect.send(
            ctx.cycle,
            Node::Home,
            CoherenceMsg::Snoop { txn, line, kind, target },
        );
    }

    fn send_llc(&self, req_id: ReqId, line: LineAddr, op: MemOp, ctx: &mut HandleCtx<'_>) {
        ctx.scheduler.schedule(
            ctx.cycle,
            self.llc,
            ctx.self_id,
            Packet::MemReq { req_id, paddr: line.phys(), vaddr: None, size: AccessSize::Line, op },
        );
    }

    fn on_snoop_resp(
        &mut self,
        id: ReqId,
        from: CoreId,
        had_copy: bool,
        dirty: bool,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(txn) = self.txn_mut(id).copied() else { return };
        let snoop_kind = match txn.kind {
            TxnKind::Recall => SnoopKind::Invalid,
            TxnKind::Request { kind, .. } => broadcast_snoop(kind),
        };
        match snoop_kind {
            SnoopKind::Shared if had_copy => self.tracking.on_downgrade(txn.line, from),
            SnoopKind::Clean => {}
            SnoopKind::Shared | SnoopKind::Unique | SnoopKind::Invalid | SnoopKind::MakeInvalid => {
                self.tracking.on_release(txn.line, from);
            }
        }
        let Some(t) = self.txn_mut(id) else { return };
        t.others_remain |= had_copy && snoop_kind == SnoopKind::Shared;
        if dirty && snoop_kind != SnoopKind::MakeInvalid {
            t.dirty_from = Some(from);
        }
        t.snoops_outstanding = t.snoops_outstanding.saturating_sub(1);
        if t.snoops_outstanding > 0 {
            return;
        }
        match txn.kind {
            TxnKind::Recall => self.complete_recall(id, ctx),
            TxnKind::Request { .. } => self.start_data_phase(id, ctx),
        }
    }

    /// The recall of `victim` finished: resume every request that waited
    /// for it.
    fn complete_recall(&mut self, id: ReqId, ctx: &mut HandleCtx<'_>) {
        let Some(index) = self.txns.iter().position(|t| t.id == id) else { return };
        let recall = self.txns.remove(index);
        if recall.dirty_from.is_some() {
            let llc_req = self.fabric_req_id();
            self.send_llc(llc_req, recall.line, MemOp::Writeback { dirty: true }, ctx);
        }
        let resumed: Vec<ReqId> = self
            .txns
            .iter()
            .filter(|t| t.phase == Phase::Recalling { victim: recall.line })
            .map(|t| t.id)
            .collect();
        for txn_id in resumed {
            self.start_snoops(txn_id, ctx);
        }
        self.admit_waiting(ctx);
    }

    /// Snoops are done: get the data (from the owner that answered dirty,
    /// or from the LLC) or grant permission.
    fn start_data_phase(&mut self, id: ReqId, ctx: &mut HandleCtx<'_>) {
        let Some(txn) = self.txn_mut(id).copied() else { return };
        let TxnKind::Request { kind, requester } = txn.kind else { return };
        match kind {
            ReqKind::CleanUnique => {
                let state = self.protocol.grant(kind, false);
                self.complete(
                    id,
                    CoherenceMsg::Comp { txn: id, line: txn.line, to: requester, state },
                    state,
                    ctx,
                );
            }
            ReqKind::ReadShared | ReqKind::ReadUnique => {
                if txn.dirty_from.is_some() {
                    ctx.stats.counter(self.stat_paths.home.c2c_transfers).inc();
                    let llc_req = self.fabric_req_id();
                    self.send_llc(llc_req, txn.line, MemOp::Writeback { dirty: true }, ctx);
                    let state = self.protocol.grant(kind, txn.others_remain);
                    self.complete(
                        id,
                        CoherenceMsg::CompData { txn: id, line: txn.line, to: requester, state },
                        state,
                        ctx,
                    );
                } else {
                    let llc_req = self.fabric_req_id();
                    if let Some(t) = self.txn_mut(id) {
                        t.phase = Phase::FetchingData;
                        t.llc_req = Some(llc_req);
                    }
                    let op = if kind == ReqKind::ReadUnique { MemOp::ReadOwn } else { MemOp::Read };
                    self.send_llc(llc_req, txn.line, op, ctx);
                }
            }
            ReqKind::Maintain { op, dirty } => {
                let llc_req = self.fabric_req_id();
                if let Some(t) = self.txn_mut(id) {
                    t.phase = Phase::Maintaining;
                    t.llc_req = Some(llc_req);
                }
                let dirty = dirty || txn.dirty_from.is_some();
                self.send_llc(llc_req, txn.line, MemOp::Maintain { op, dirty }, ctx);
            }
            ReqKind::WriteBack { .. } | ReqKind::Evict => {}
        }
    }

    /// The LLC answered one of the fabric's requests.
    fn on_llc_response(&mut self, llc_req: ReqId, ctx: &mut HandleCtx<'_>) {
        let Some(txn) = self.txns.iter().find(|t| t.llc_req == Some(llc_req)).copied() else {
            return;
        };
        let TxnKind::Request { kind, requester } = txn.kind else { return };
        match txn.phase {
            Phase::FetchingData => {
                let state = self.protocol.grant(kind, txn.others_remain);
                self.complete(
                    txn.id,
                    CoherenceMsg::CompData { txn: txn.id, line: txn.line, to: requester, state },
                    state,
                    ctx,
                );
            }
            Phase::WritingBack | Phase::Maintaining => {
                let state = MesiState::Invalid;
                self.complete(
                    txn.id,
                    CoherenceMsg::Comp { txn: txn.id, line: txn.line, to: requester, state },
                    state,
                    ctx,
                );
            }
            Phase::Recalling { .. } | Phase::Snooping | Phase::AwaitingAck => {}
        }
    }

    /// Sends the completion and records the grant; the line stays busy
    /// until the requester acknowledges.
    fn complete(
        &mut self,
        id: ReqId,
        msg: CoherenceMsg,
        granted: MesiState,
        ctx: &mut HandleCtx<'_>,
    ) {
        let Some(txn) = self.txn_mut(id) else { return };
        txn.phase = Phase::AwaitingAck;
        let txn = *txn;
        if let TxnKind::Request { requester, .. } = txn.kind {
            match granted {
                MesiState::Invalid => {}
                state => self.tracking.on_grant(txn.line, requester, state, ctx.cycle),
            }
        }
        ctx.stats
            .histogram(self.stat_paths.home.txn_latency)
            .record(ctx.cycle.saturating_sub(txn.started_at));
        self.interconnect.send(ctx.cycle, Node::Home, msg);
    }

    /// A core's uncached access: park it and send its marker to the home.
    fn on_non_coherent_request(&mut self, packet: Packet, from: Node, ctx: &mut HandleCtx<'_>) {
        let (Packet::MemReq { req_id, paddr, op, .. }, Node::Core(requester)) = (&packet, from)
        else {
            return;
        };
        let txn = *req_id;
        let line = LineAddr::from_phys(*paddr, self.line_bytes as u64);
        let bytes = match op {
            MemOp::Write { data: WriteData::Line { .. }, .. }
            | MemOp::Maintain { dirty: true, .. } => self.line_bytes,
            MemOp::Write { .. } | MemOp::Atomic { .. } => 8,
            MemOp::Read
            | MemOp::ReadOwn
            | MemOp::Fetch
            | MemOp::Writeback { .. }
            | MemOp::Maintain { dirty: false, .. } => 0,
        };
        ctx.stats.counter(self.stat_paths.home.non_coherent).inc();
        self.parked_requests.push(Parked { txn, requester, packet: Some(packet) });
        self.interconnect.send(
            ctx.cycle,
            from,
            CoherenceMsg::NoSnp { txn, line, requester, bytes },
        );
    }

    /// The marker of a parked request reached the home: the request itself
    /// goes on to the LLC.
    fn forward_parked_request(&mut self, txn: ReqId, ctx: &mut HandleCtx<'_>) {
        let Some(parked) = self.parked_requests.iter_mut().find(|p| p.txn == txn) else { return };
        let Some(packet) = parked.packet.take() else { return };
        ctx.scheduler.schedule(ctx.cycle, self.llc, ctx.self_id, packet);
    }

    /// The LLC answered a parked request: park the answer and send its
    /// marker back to the core.
    fn on_non_coherent_response(&mut self, packet: Packet, now: u64) {
        let Packet::MemResp { req_id, line_addr, data, .. } = &packet else { return };
        let (txn, line) = (*req_id, *line_addr);
        let Some(index) = self.parked_requests.iter().position(|p| p.txn == txn) else { return };
        let to = self.parked_requests.remove(index).requester;
        let bytes = match data {
            MemRespData::Line(bytes) | MemRespData::PerformedBytes { bytes, .. } => bytes.len(),
            MemRespData::Small(_) | MemRespData::Performed { .. } => 8,
        };
        self.parked_responses.push(Parked { txn, requester: to, packet: Some(packet) });
        self.interconnect.send(now, Node::Home, CoherenceMsg::NoSnpData { txn, line, to, bytes });
    }

    fn deliver_parked_response(&mut self, txn: ReqId, ctx: &mut HandleCtx<'_>) {
        let Some(index) = self.parked_responses.iter().position(|p| p.txn == txn) else { return };
        let parked = self.parked_responses.remove(index);
        let Some(packet) = parked.packet else { return };
        ctx.scheduler.schedule(
            ctx.cycle,
            self.agents[parked.requester.as_index()],
            ctx.self_id,
            packet,
        );
    }

    /// The requester took up its completion: free the transaction and admit
    /// waiting requests.
    fn on_comp_ack(&mut self, id: ReqId, ctx: &mut HandleCtx<'_>) {
        let Some(index) =
            self.txns.iter().position(|t| t.id == id && t.phase == Phase::AwaitingAck)
        else {
            return;
        };
        let _acked = self.txns.remove(index);
        self.admit_waiting(ctx);
    }

    /// Re-presents queued requests whose line is free while the table has
    /// room, in arrival order.
    fn admit_waiting(&mut self, ctx: &mut HandleCtx<'_>) {
        // `admit` re-queues what it cannot start, so the queue is rebuilt
        // behind it in arrival order.
        let queue = std::mem::take(&mut self.waiting);
        for waiting in queue {
            let line = waiting.msg.line();
            if self.txns.len() >= self.txn_capacity
                || self.line_busy(line)
                || self.waiting.iter().any(|w| w.msg.line() == line)
            {
                self.waiting.push_back(waiting);
                continue;
            }
            self.admit(waiting, ctx);
        }
    }
}

impl CoherenceFabric {
    fn node_of_agent(&self, source: ComponentId) -> Option<Node> {
        self.agents
            .iter()
            .position(|a| *a == source)
            .map(|index| Node::Core(CoreId::new(u32::try_from(index).unwrap_or(u32::MAX))))
    }
}

/// The snoop a request sends a holder when the home does not know who holds
/// the line and snoops every other core.
const fn broadcast_snoop(kind: ReqKind) -> SnoopKind {
    match kind {
        ReqKind::ReadShared => SnoopKind::Shared,
        ReqKind::Maintain { op: Maintenance::Clean, .. } => SnoopKind::Clean,
        ReqKind::Maintain { op: Maintenance::Invalidate, .. } => SnoopKind::MakeInvalid,
        ReqKind::ReadUnique
        | ReqKind::CleanUnique
        | ReqKind::WriteBack { .. }
        | ReqKind::Evict
        | ReqKind::Maintain { op: Maintenance::Flush, .. } => SnoopKind::Unique,
    }
}

impl Handle for CoherenceFabric {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        match packet {
            Packet::Coh(msg) => {
                let from = self.node_of_agent(source).unwrap_or(Node::Home);
                self.interconnect.send(ctx.cycle, from, msg);
            }
            Packet::MemReq { .. } => {
                let Some(from) = self.node_of_agent(source) else { return };
                self.on_non_coherent_request(packet, from, ctx);
            }
            Packet::MemResp { req_id, .. } => {
                if self.txns.iter().any(|t| t.llc_req == Some(req_id)) {
                    self.on_llc_response(req_id, ctx);
                } else {
                    self.on_non_coherent_response(packet, ctx.cycle);
                }
            }
            _ => {}
        }
    }
}
