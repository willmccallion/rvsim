//! The home agent's transaction engine, driven message by message: two
//! requesting agents on a crossbar, an LLC the bench answers itself, and
//! snoop responses the tests script.

use rvsim_core::common::{CoreId, LineAddr, PhysAddr};
use rvsim_core::config::Config;
use rvsim_core::sim::components::{CacheId, ComponentId, ReqId};
use rvsim_core::sim::events::EventQueue;
use rvsim_core::sim::handle::{Handle, HandleCtx};
use rvsim_core::sim::packet::{
    AccessSize, HitLevel, Maintenance, MemOp, MemRespData, MesiState, Packet,
};
use rvsim_core::sim::stats::Stats;
use rvsim_core::soc::coherence::fabric::{CoherenceFabric, FabricLayout};
use rvsim_core::soc::coherence::home::{Broadcast, HomeAgent, SnoopFilter};
use rvsim_core::soc::coherence::interconnect::Crossbar;
use rvsim_core::soc::coherence::messages::{CoherenceMsg, ReqKind, SnoopKind};
use rvsim_core::soc::coherence::protocol::Mesi;
use rvsim_core::soc::coherence::stats::CoherenceStatPaths;
use rvsim_core::system::state::global_memory::GlobalMemory;

const LLC: ComponentId = ComponentId::Cache(CacheId::new(6));
const AGENTS: [ComponentId; 2] =
    [ComponentId::Cache(CacheId::new(2)), ComponentId::Cache(CacheId::new(5))];
const LLC_LATENCY: u64 = 4;
const HOP: u64 = 1;

fn line(addr: u64) -> LineAddr {
    LineAddr::from_phys(PhysAddr::new(addr), 64)
}

fn core(index: usize) -> CoreId {
    CoreId::new(index as u32)
}

/// An LLC request the bench saw, as `(cycle, op, line address)`.
type LlcRequest = (u64, MemOp, u64);

struct Bench {
    fabric: CoherenceFabric,
    queue: EventQueue,
    stats: Stats,
    memory: GlobalMemory,
    config: Config,
    cycle: u64,
    /// Packets to hand the fabric at a cycle, with their source.
    inbox: Vec<(u64, ComponentId, Packet)>,
    llc_requests: Vec<LlcRequest>,
    /// Messages delivered to the cores, as `(cycle, core, msg)`.
    to_cores: Vec<(u64, usize, CoherenceMsg)>,
    /// Non-coherent responses delivered to the agents.
    to_agents: Vec<(u64, usize, Packet)>,
    unanswered_snoops: Vec<(usize, CoherenceMsg)>,
    next_req: u64,
}

impl Bench {
    fn new(tracking: Box<dyn HomeAgent>) -> Self {
        let stat_paths = CoherenceStatPaths::new();
        let interconnect = Box::new(Crossbar::new(2, 64, HOP, 64, stat_paths.interconnect));
        let layout =
            FabricLayout { llc: LLC, agents: AGENTS.to_vec(), line_bytes: 64, txn_capacity: 8 };
        let fabric =
            CoherenceFabric::new(Box::new(Mesi), tracking, interconnect, layout, stat_paths);
        Self {
            fabric,
            queue: EventQueue::new(),
            stats: Stats::new(),
            memory: GlobalMemory::new(None, 1, 64),
            config: Config::default(),
            cycle: 0,
            inbox: Vec::new(),
            llc_requests: Vec::new(),
            to_cores: Vec::new(),
            to_agents: Vec::new(),
            unanswered_snoops: Vec::new(),
            next_req: 1,
        }
    }

    fn precise() -> Self {
        Self::new(Box::new(SnoopFilter::new(64, 4, 64)))
    }

    fn handle(&mut self, packet: Packet, source: ComponentId) {
        let mut ctx = HandleCtx {
            scheduler: &mut self.queue,
            stats: &mut self.stats,
            memory: &mut self.memory,
            config: &self.config,
            cycle: self.cycle,
            self_id: ComponentId::Fabric,
        };
        self.fabric.handle(packet, source, &mut ctx);
    }

    fn req_id(&mut self, from: usize) -> ReqId {
        let seq = self.next_req;
        self.next_req += 1;
        ReqId::for_cache(CacheId::new(from as u32 * 3 + 2), seq)
    }

    /// A core's request; returns its correlator.
    fn request(&mut self, from: usize, addr: u64, kind: ReqKind) -> ReqId {
        let txn = self.req_id(from);
        self.handle(
            Packet::Coh(CoherenceMsg::Req { txn, line: line(addr), kind, requester: core(from) }),
            AGENTS[from],
        );
        txn
    }

    fn answer_snoops(&mut self, from: usize, had_copy: bool, dirty: bool) -> usize {
        let mine: Vec<CoherenceMsg> =
            self.unanswered_snoops.iter().filter(|(c, _)| *c == from).map(|(_, m)| *m).collect();
        self.unanswered_snoops.retain(|(c, _)| *c != from);
        for msg in &mine {
            let CoherenceMsg::Snoop { txn, line, .. } = *msg else { continue };
            self.inbox.push((
                self.cycle + 1,
                AGENTS[from],
                Packet::Coh(CoherenceMsg::SnoopResp {
                    txn,
                    line,
                    from: core(from),
                    had_copy,
                    dirty,
                }),
            ));
        }
        mine.len()
    }

    fn step(&mut self) {
        self.cycle += 1;
        let due: Vec<(u64, ComponentId, Packet)> = {
            let (due, later): (Vec<_>, Vec<_>) =
                self.inbox.drain(..).partition(|(at, _, _)| *at <= self.cycle);
            self.inbox = later;
            due
        };
        for (_, source, packet) in due {
            self.handle(packet, source);
        }
        {
            let mut ctx = HandleCtx {
                scheduler: &mut self.queue,
                stats: &mut self.stats,
                memory: &mut self.memory,
                config: &self.config,
                cycle: self.cycle,
                self_id: ComponentId::Fabric,
            };
            self.fabric.tick(&mut ctx);
        }
        while let Some(event) = self.queue.pop_ready(self.cycle) {
            if event.target == LLC {
                self.on_llc_request(event.packet);
            } else if let Some(index) = AGENTS.iter().position(|a| *a == event.target) {
                match event.packet {
                    Packet::Coh(msg) => self.on_core_message(index, msg),
                    other => self.to_agents.push((self.cycle, index, other)),
                }
            }
        }
    }

    fn on_llc_request(&mut self, packet: Packet) {
        let Packet::MemReq { req_id, paddr, op, .. } = packet else { return };
        let state =
            if matches!(op, MemOp::ReadOwn) { MesiState::Modified } else { MesiState::Exclusive };
        self.llc_requests.push((self.cycle, op, paddr.val()));
        let response = Packet::MemResp {
            req_id,
            line_addr: line(paddr.val()),
            data: MemRespData::Small(0),
            hit_level: HitLevel::L3,
            state,
        };
        self.inbox.push((self.cycle + LLC_LATENCY, LLC, response));
    }

    fn on_core_message(&mut self, index: usize, msg: CoherenceMsg) {
        self.to_cores.push((self.cycle, index, msg));
        match msg {
            CoherenceMsg::Snoop { .. } => self.unanswered_snoops.push((index, msg)),
            CoherenceMsg::CompData { txn, line, .. } | CoherenceMsg::Comp { txn, line, .. } => {
                let ack = Packet::Coh(CoherenceMsg::CompAck { txn, line, from: core(index) });
                self.inbox.push((self.cycle + 1, AGENTS[index], ack));
            }
            _ => {}
        }
    }

    fn run(&mut self, cycles: u64) {
        for _ in 0..cycles {
            self.step();
        }
    }

    fn run_until_idle(&mut self) {
        for _ in 0..200 {
            self.step();
            if self.fabric.is_idle() && self.inbox.is_empty() && self.unanswered_snoops.is_empty() {
                return;
            }
        }
        panic!("fabric did not go idle: {:?}", self.fabric);
    }

    fn completions_for(&self, txn: ReqId) -> Vec<(u64, MesiState, bool)> {
        self.to_cores
            .iter()
            .filter_map(|(cycle, _, msg)| match *msg {
                CoherenceMsg::CompData { txn: t, state, .. } if t == txn => {
                    Some((*cycle, state, true))
                }
                CoherenceMsg::Comp { txn: t, state, .. } if t == txn => {
                    Some((*cycle, state, false))
                }
                _ => None,
            })
            .collect()
    }

    fn snoops_to(&self, index: usize) -> Vec<(u64, SnoopKind, u64)> {
        self.to_cores
            .iter()
            .filter_map(|(cycle, c, msg)| match *msg {
                CoherenceMsg::Snoop { kind, line, .. } if *c == index => {
                    Some((*cycle, kind, line.val()))
                }
                _ => None,
            })
            .collect()
    }

    fn llc_writebacks(&self) -> usize {
        self.llc_requests.iter().filter(|(_, op, _)| matches!(op, MemOp::Writeback { .. })).count()
    }

    fn llc_reads(&self) -> usize {
        self.llc_requests
            .iter()
            .filter(|(_, op, _)| matches!(op, MemOp::Read | MemOp::ReadOwn))
            .count()
    }

    fn stat(&self, path: &str) -> u64 {
        self.stats.get(path).unwrap_or(0.0) as u64
    }

    /// Owner and sharers of `addr` as the home tracks them.
    fn holders(&self, addr: u64) -> (Option<CoreId>, Vec<CoreId>) {
        let h = self.fabric.tracked_holders(line(addr)).expect("precise tracking");
        (h.owner, h.sharers.iter().collect())
    }
}

#[test]
fn an_untracked_line_is_read_from_the_llc_and_granted_exclusive() {
    let mut bench = Bench::precise();

    let txn = bench.request(0, 0x1000, ReqKind::ReadShared);
    bench.run_until_idle();

    assert_eq!(bench.llc_reads(), 1);
    assert!(bench.snoops_to(1).is_empty(), "nobody holds the line: nothing to snoop");
    let done = bench.completions_for(txn);
    assert_eq!(done.len(), 1);
    assert_eq!((done[0].1, done[0].2), (MesiState::Exclusive, true));
    assert_eq!(bench.holders(0x1000), (Some(core(0)), vec![core(0)]));
    assert_eq!(bench.stat("coherence.ha.requests.read_shared"), 1);
}

#[test]
fn a_completion_waits_for_the_llc_and_crosses_the_interconnect_twice() {
    let mut bench = Bench::precise();

    let txn = bench.request(0, 0x1000, ReqKind::ReadShared);
    bench.run_until_idle();

    // Request: one hop plus its transfer. LLC: latency. Data: one hop plus
    // the transfer of a 72-byte message at 64 bytes per cycle.
    let request_at = bench.llc_requests[0].0;
    assert_eq!(request_at, 1 + HOP + 1);
    let done = bench.completions_for(txn)[0].0;
    assert_eq!(done, request_at + LLC_LATENCY + HOP + 2);
}

#[test]
fn a_second_reader_downgrades_the_owner_and_both_share() {
    let mut bench = Bench::precise();
    bench.request(0, 0x1000, ReqKind::ReadShared);
    bench.run_until_idle();

    let txn = bench.request(1, 0x1000, ReqKind::ReadShared);
    bench.run(6);
    assert_eq!(bench.snoops_to(0).len(), 1);
    assert_eq!(bench.snoops_to(0)[0].1, SnoopKind::Shared);
    assert!(bench.completions_for(txn).is_empty(), "the reader waits for the owner's answer");
    bench.answer_snoops(0, true, false);
    bench.run_until_idle();

    let done = bench.completions_for(txn);
    assert_eq!((done[0].1, done[0].2), (MesiState::Shared, true));
    assert_eq!(bench.llc_reads(), 2, "the clean owner's answer means the LLC copy is current");
    assert_eq!(bench.holders(0x1000), (None, vec![core(0), core(1)]));
}

#[test]
fn a_writer_invalidates_every_sharer() {
    let mut bench = Bench::precise();
    bench.request(0, 0x1000, ReqKind::ReadShared);
    bench.run_until_idle();
    bench.request(1, 0x1000, ReqKind::ReadShared);
    bench.run(6);
    bench.answer_snoops(0, true, false);
    bench.run_until_idle();

    let txn = bench.request(0, 0x1000, ReqKind::CleanUnique);
    bench.run(6);
    assert_eq!(bench.snoops_to(1).last().map(|s| s.1), Some(SnoopKind::Unique));
    bench.answer_snoops(1, true, false);
    bench.run_until_idle();

    let done = bench.completions_for(txn);
    assert_eq!((done[0].1, done[0].2), (MesiState::Modified, false), "permission only: no data");
    assert_eq!(bench.holders(0x1000), (Some(core(0)), vec![core(0)]));
    assert_eq!(bench.stat("coherence.ha.requests.clean_unique"), 1);
}

#[test]
fn a_dirty_owner_answers_the_requester_and_the_llc_is_updated() {
    let mut bench = Bench::precise();
    bench.request(0, 0x1000, ReqKind::ReadUnique);
    bench.run_until_idle();
    let reads_before = bench.llc_reads();

    let txn = bench.request(1, 0x1000, ReqKind::ReadUnique);
    bench.run(6);
    bench.answer_snoops(0, true, true);
    bench.run_until_idle();

    assert_eq!(bench.llc_reads(), reads_before, "the data came from the owner");
    assert_eq!(bench.llc_writebacks(), 1, "the owner's modified data goes into the LLC");
    let done = bench.completions_for(txn);
    assert_eq!((done[0].1, done[0].2), (MesiState::Modified, true));
    assert_eq!(bench.holders(0x1000), (Some(core(1)), vec![core(1)]));
    assert_eq!(bench.stat("coherence.ha.c2c_transfers"), 1);
}

#[test]
fn requests_for_one_line_wait_until_the_earlier_one_is_acknowledged() {
    let mut bench = Bench::precise();

    let first = bench.request(0, 0x1000, ReqKind::ReadShared);
    bench.run(2);
    let second = bench.request(1, 0x1000, ReqKind::ReadShared);
    bench.run(4);
    assert_eq!(bench.llc_reads(), 1, "the second request has not started");
    assert!(bench.snoops_to(0).is_empty());
    bench.run_until_idle_with_answers(0);

    let first_done = bench.completions_for(first)[0].0;
    let snoop_at = bench.snoops_to(0)[0].0;
    assert!(snoop_at > first_done + 1, "the second transaction starts only after the first's ack");
    assert_eq!(bench.completions_for(second).len(), 1);
    assert_eq!(bench.stat("coherence.ha.serialised"), 1);
}

impl Bench {
    /// Runs to idle, answering every snoop to `core` as a clean holder.
    fn run_until_idle_with_answers(&mut self, core: usize) {
        for _ in 0..200 {
            self.step();
            self.answer_snoops(core, true, false);
            if self.fabric.is_idle() && self.inbox.is_empty() {
                return;
            }
        }
        panic!("fabric did not go idle: {:?}", self.fabric);
    }
}

#[test]
fn a_full_filter_set_recalls_its_least_recently_used_line() {
    // Two entries in one set: every line maps to it.
    let mut bench = Bench::new(Box::new(SnoopFilter::new(2, 2, 64)));
    bench.request(0, 0x0000, ReqKind::ReadShared);
    bench.run_until_idle();
    bench.request(0, 0x0040, ReqKind::ReadShared);
    bench.run_until_idle();

    let txn = bench.request(1, 0x0080, ReqKind::ReadShared);
    bench.run(6);
    assert_eq!(
        bench.snoops_to(0),
        vec![(bench.snoops_to(0)[0].0, SnoopKind::Invalid, 0x0000)],
        "the older line is recalled"
    );
    bench.answer_snoops(0, true, false);
    bench.run_until_idle();

    assert_eq!(bench.completions_for(txn).len(), 1);
    assert_eq!(bench.holders(0x0000), (None, vec![]));
    assert_eq!(bench.holders(0x0080), (Some(core(1)), vec![core(1)]));
    assert_eq!(bench.stat("coherence.ha.recalls"), 1);
}

#[test]
fn a_writeback_after_a_snoop_emptied_the_core_is_only_acknowledged() {
    let mut bench = Bench::precise();
    bench.request(0, 0x1000, ReqKind::ReadUnique);
    bench.run_until_idle();
    bench.request(1, 0x1000, ReqKind::ReadUnique);
    bench.run(6);
    bench.answer_snoops(0, true, true);
    bench.run_until_idle();
    assert_eq!(bench.llc_writebacks(), 1);

    let txn = bench.request(0, 0x1000, ReqKind::WriteBack { dirty: true });
    bench.run_until_idle();

    assert_eq!(bench.llc_writebacks(), 1, "the snoop already carried the data");
    let done = bench.completions_for(txn);
    assert_eq!((done[0].1, done[0].2), (MesiState::Invalid, false));
    assert_eq!(bench.stat("coherence.ha.requests.stale_writebacks"), 1);
    assert_eq!(bench.holders(0x1000), (Some(core(1)), vec![core(1)]));
}

#[test]
fn a_writeback_from_the_owner_reaches_the_llc_and_frees_the_line() {
    let mut bench = Bench::precise();
    bench.request(0, 0x1000, ReqKind::ReadUnique);
    bench.run_until_idle();

    let txn = bench.request(0, 0x1000, ReqKind::WriteBack { dirty: true });
    bench.run_until_idle();

    assert_eq!(bench.llc_writebacks(), 1);
    assert_eq!(bench.completions_for(txn).len(), 1);
    assert_eq!(bench.holders(0x1000), (None, vec![]));
}

#[test]
fn a_silent_eviction_drops_the_core_from_the_tracking() {
    let mut bench = Bench::precise();
    bench.request(0, 0x1000, ReqKind::ReadShared);
    bench.run_until_idle();

    bench.request(0, 0x1000, ReqKind::Evict);
    bench.run_until_idle();

    assert_eq!(bench.holders(0x1000), (None, vec![]));
    bench.request(1, 0x1000, ReqKind::ReadShared);
    bench.run_until_idle();
    assert!(bench.snoops_to(0).is_empty(), "an evicted line is not snooped");
}

#[test]
fn an_uncached_access_crosses_the_fabric_to_the_llc_and_back() {
    let mut bench = Bench::precise();
    let req_id = ReqId::for_cache(CacheId::new(2), 77);
    bench.handle(
        Packet::MemReq {
            req_id,
            paddr: PhysAddr::new(0x80000400),
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Read,
        },
        AGENTS[0],
    );
    bench.run_until_idle();

    assert_eq!(bench.llc_requests.len(), 1);
    assert_eq!(bench.llc_requests[0].2, 0x80000400);
    assert_eq!(bench.to_agents.len(), 1);
    assert_eq!(bench.to_agents[0].1, 0);
    assert!(matches!(bench.to_agents[0].2, Packet::MemResp { req_id: r, .. } if r == req_id));
    assert!(bench.to_cores.is_empty(), "no coherence traffic for an uncached access");
    assert_eq!(bench.stat("coherence.ha.requests.non_coherent"), 1);
    assert!(bench.fabric.tracked_holders(line(0x80000400)).is_some_and(|h| h.sharers.is_empty()));
}

#[test]
fn a_broadcast_home_snoops_every_other_core() {
    let mut bench = Bench::new(Box::new(Broadcast));

    let txn = bench.request(0, 0x1000, ReqKind::ReadShared);
    bench.run(6);
    assert_eq!(bench.snoops_to(1).len(), 1, "no tracking: the other core is asked");
    bench.answer_snoops(1, false, false);
    bench.run_until_idle();

    let done = bench.completions_for(txn);
    assert_eq!((done[0].1, done[0].2), (MesiState::Exclusive, true));
    assert!(bench.fabric.tracked_holders(line(0x1000)).is_none());
}

/// The maintenance requests the LLC saw, as `(op, dirty)`.
fn llc_maintenance(bench: &Bench) -> Vec<(Maintenance, bool)> {
    bench
        .llc_requests
        .iter()
        .filter_map(|(_, op, _)| match *op {
            MemOp::Maintain { op, dirty } => Some((op, dirty)),
            _ => None,
        })
        .collect()
}

fn maintain(op: Maintenance) -> ReqKind {
    ReqKind::Maintain { op, dirty: false }
}

#[test]
fn a_flush_invalidates_every_other_holder_then_reaches_the_llc_with_their_dirty_data() {
    let mut bench = Bench::precise();
    bench.request(1, 0x1000, ReqKind::ReadUnique);
    bench.run_until_idle();

    let txn = bench.request(0, 0x1000, maintain(Maintenance::Flush));
    bench.run(6);
    assert_eq!(bench.snoops_to(1).last().map(|s| s.1), Some(SnoopKind::Unique));
    assert!(llc_maintenance(&bench).is_empty(), "the LLC waits for the snoops");
    bench.answer_snoops(1, true, true);
    bench.run_until_idle();

    assert_eq!(llc_maintenance(&bench), [(Maintenance::Flush, true)]);
    assert_eq!(bench.completions_for(txn).len(), 1);
    assert_eq!(bench.holders(0x1000), (None, vec![]));
    assert_eq!(bench.stat("coherence.ha.requests.maintenance"), 1);
}

#[test]
fn a_clean_snoops_only_the_owner_which_keeps_its_line() {
    let mut bench = Bench::precise();
    bench.request(1, 0x1000, ReqKind::ReadUnique);
    bench.run_until_idle();

    let txn = bench.request(0, 0x1000, maintain(Maintenance::Clean));
    bench.run(6);
    assert_eq!(bench.snoops_to(1).last().map(|s| s.1), Some(SnoopKind::Clean));
    bench.answer_snoops(1, true, true);
    bench.run_until_idle();

    assert_eq!(llc_maintenance(&bench), [(Maintenance::Clean, true)]);
    assert_eq!(bench.completions_for(txn).len(), 1);
    assert_eq!(bench.holders(0x1000), (Some(core(1)), vec![core(1)]));
}

#[test]
fn an_invalidate_drops_every_other_copy_and_discards_its_data() {
    let mut bench = Bench::precise();
    bench.request(1, 0x1000, ReqKind::ReadUnique);
    bench.run_until_idle();

    bench.request(0, 0x1000, maintain(Maintenance::Invalidate));
    bench.run(6);
    assert_eq!(bench.snoops_to(1).last().map(|s| s.1), Some(SnoopKind::MakeInvalid));
    bench.answer_snoops(1, true, true);
    bench.run_until_idle();

    assert_eq!(llc_maintenance(&bench), [(Maintenance::Invalidate, false)]);
    assert_eq!(bench.holders(0x1000), (None, vec![]));
}
