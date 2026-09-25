//! The event-driven cache: hits, misses through MSHRs, blocking, fills,
//! writebacks, inclusion and prefetching, driven packet by packet.
//!
//! A test cache is 256 bytes with 64-byte lines and 2 ways: two sets,
//! set = (addr / 64) % 2, tag = addr / 128.

use rvsim_core::common::{LineAddr, PhysAddr};
use rvsim_core::config::{
    CacheConfig, Config, InclusionPolicy, Prefetcher as PrefetcherType,
    ReplacementPolicy as PolicyType,
};
use rvsim_core::core::units::cache::Cache;
use rvsim_core::sim::components::{CacheId, ComponentId, PipelineId, ReqId};
use rvsim_core::sim::events::{Event, EventQueue};
use rvsim_core::sim::handle::{Handle, HandleCtx};
use rvsim_core::sim::packet::{AccessSize, CacheLevel, HitLevel, MemOp, MemRespData, MesiState, Packet, ProbeKind, WriteData};
use rvsim_core::sim::stats::Stats;

const LATENCY: u64 = 2;
const SELF: ComponentId = ComponentId::Cache(CacheId::new(0));
const DOWNSTREAM: ComponentId = ComponentId::Cache(CacheId::new(1));
const UPSTREAM: ComponentId = ComponentId::Cache(CacheId::new(2));
const PIPELINE: ComponentId = ComponentId::Pipeline(PipelineId::new(0));

fn test_config() -> CacheConfig {
    CacheConfig {
        enabled: true,
        size_bytes: 256,
        line_bytes: 64,
        ways: 2,
        policy: PolicyType::Lru,
        latency: LATENCY,
        prefetcher: PrefetcherType::None,
        prefetch_table_size: 64,
        prefetch_degree: 1,
        mshr_count: 4,
        write_buffers: 4,
    }
}

fn cache_with(config: &CacheConfig) -> Cache {
    let mut cache = Cache::new(CacheId::new(0), CacheLevel::L1D, config, "test");
    cache.set_downstream(DOWNSTREAM);
    cache
}

/// A cache plus the bench-side scaffolding its `Handle` impl needs.
struct Bench {
    cache: Cache,
    queue: EventQueue,
    stats: Stats,
    config: Config,
    cycle: u64,
}

impl Bench {
    fn new(cache: Cache) -> Self {
        Self { cache, queue: EventQueue::new(), stats: Stats::new(), config: Config::default(), cycle: 100 }
    }

    fn deliver(&mut self, packet: Packet, source: ComponentId) {
        let mut ctx = HandleCtx {
            scheduler: &mut self.queue,
            stats: &mut self.stats,
            config: &self.config,
            cycle: self.cycle,
            self_id: SELF,
        };
        self.cache.handle(packet, source, &mut ctx);
    }

    fn request(&mut self, req_id: u64, addr: u64, op: MemOp) {
        self.deliver(
            Packet::MemReq { req_id: ReqId::new(req_id), paddr: PhysAddr::new(addr), vaddr: None, size: AccessSize::B8, op },
            PIPELINE,
        );
    }

    fn read(&mut self, req_id: u64, addr: u64) {
        self.request(req_id, addr, MemOp::Read);
    }

    fn write(&mut self, req_id: u64, addr: u64) {
        self.request(req_id, addr, MemOp::Write { data: WriteData::Small(1) });
    }

    /// Everything scheduled so far, in delivery order.
    fn drain(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Some(event) = self.queue.pop_ready(u64::MAX) {
            out.push(event);
        }
        out
    }

    /// Downstream `MemReq`s scheduled so far, as `(req_id, addr, op, fire_at)`.
    fn downstream_requests(&mut self) -> Vec<(ReqId, u64, MemOp, u64)> {
        self.drain()
            .into_iter()
            .filter(|e| e.target == DOWNSTREAM)
            .filter_map(|e| match e.packet {
                Packet::MemReq { req_id, paddr, op, .. } => Some((req_id, paddr.val(), op, e.fire_at)),
                _ => None,
            })
            .collect()
    }

    /// Answers a downstream request as if the next level filled it.
    fn fill(&mut self, req_id: ReqId, addr: u64) {
        self.fill_with(req_id, addr, MesiState::Exclusive);
    }

    fn fill_with(&mut self, req_id: ReqId, addr: u64, state: MesiState) {
        self.deliver(
            Packet::MemResp {
                req_id,
                line_addr: LineAddr::from_phys(PhysAddr::new(addr), 64),
                data: MemRespData::Small(0),
                hit_level: HitLevel::Dram,
                state,
            },
            DOWNSTREAM,
        );
    }

    fn state_of(&self, addr: u64) -> Option<MesiState> {
        let line = LineAddr::from_phys(PhysAddr::new(addr), 64);
        self.cache.held_lines().into_iter().find(|(l, _)| *l == line).map(|(_, s)| s)
    }

    /// Loads `addr` into the cache through a miss and its fill.
    fn install(&mut self, req_id: u64, addr: u64, op: MemOp) {
        self.request(req_id, addr, op);
        let requests = self.downstream_requests();
        let (down_id, _, _, _) = requests.into_iter().next().expect("miss forwarded downstream");
        self.fill(down_id, addr);
        let _ = self.drain();
    }

    fn stat(&self, path: &str) -> u64 {
        self.stats.get(path).unwrap_or(0.0) as u64
    }
}

fn responses_to(events: &[Event], target: ComponentId) -> Vec<(ReqId, u64)> {
    events
        .iter()
        .filter(|e| e.target == target)
        .filter_map(|e| match &e.packet {
            Packet::MemResp { req_id, .. } => Some((*req_id, e.fire_at)),
            _ => None,
        })
        .collect()
}

#[test]
fn miss_sends_one_line_request_and_the_fill_answers_the_requester() {
    let mut bench = Bench::new(cache_with(&test_config()));

    bench.read(7, 0x1008);
    let requests = bench.downstream_requests();
    assert_eq!(requests.len(), 1);
    let (down_id, addr, op, fire_at) = requests[0].clone();
    assert_eq!(addr, 0x1000, "line-aligned fetch");
    assert!(matches!(op, MemOp::Read));
    assert_eq!(fire_at, bench.cycle + LATENCY, "tag lookup precedes the fetch");
    assert_ne!(down_id, ReqId::new(7), "the cache uses its own correlator downstream");

    bench.fill(down_id, 0x1000);
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE), vec![(ReqId::new(7), bench.cycle)]);
    assert!(bench.cache.contains(0x1008));
    assert_eq!(bench.stat("test.misses"), 1);
    assert_eq!(bench.stat("test.fills"), 1);
}

#[test]
fn hit_answers_after_latency_without_downstream_traffic() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Read);

    bench.read(2, 0x1010);
    let events = bench.drain();
    assert!(events.iter().all(|e| e.target != DOWNSTREAM));
    assert_eq!(responses_to(&events, PIPELINE), vec![(ReqId::new(2), bench.cycle + LATENCY)]);
    assert_eq!(bench.stat("test.hits"), 1);
}

#[test]
fn a_second_miss_to_the_same_line_joins_the_mshr() {
    let mut bench = Bench::new(cache_with(&test_config()));

    bench.read(1, 0x1000);
    bench.write(2, 0x1020);
    let requests = bench.downstream_requests();
    assert_eq!(requests.len(), 1, "one fetch for both misses");
    assert_eq!(bench.stat("test.mshr_hits"), 1);

    bench.fill(requests[0].0, 0x1000);
    let events = bench.drain();
    let answered: Vec<ReqId> = responses_to(&events, PIPELINE).into_iter().map(|(id, _)| id).collect();
    assert_eq!(answered, vec![ReqId::new(1), ReqId::new(2)]);
    assert!(bench.cache.duplicate_lines().is_empty());
}

#[test]
fn requests_queue_while_mshrs_are_full_and_retry_after_a_fill() {
    let mut config = test_config();
    config.mshr_count = 1;
    let mut bench = Bench::new(cache_with(&config));

    bench.read(1, 0x1000);
    let first = bench.downstream_requests();
    assert_eq!(first.len(), 1);

    bench.read(2, 0x2000);
    assert!(bench.downstream_requests().is_empty(), "no MSHR free: nothing goes downstream");
    assert_eq!(bench.cache.blocked_requests(), 1);
    assert_eq!(bench.stat("test.blocked_requests"), 1);

    bench.fill(first[0].0, 0x1000);
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE), vec![(ReqId::new(1), bench.cycle)]);
    let retried: Vec<u64> = events
        .iter()
        .filter(|e| e.target == DOWNSTREAM)
        .filter_map(|e| match e.packet { Packet::MemReq { paddr, .. } => Some(paddr.val()), _ => None })
        .collect();
    assert_eq!(retried, vec![0x2000], "the queued miss is fetched once an MSHR frees");
    assert_eq!(bench.cache.blocked_requests(), 0);
}

#[test]
fn a_dirty_victim_is_written_back_and_a_clean_one_is_dropped() {
    let mut bench = Bench::new(cache_with(&test_config()));
    // Set 0 holds tags for 0x0000 and 0x0080; 0x0100 evicts the LRU one.
    bench.install(1, 0x0000, MemOp::Write { data: WriteData::Small(1) });
    bench.install(2, 0x0080, MemOp::Read);

    bench.read(3, 0x0100);
    let fetch = bench.downstream_requests();
    bench.fill(fetch[0].0, 0x0100);
    let events = bench.drain();
    let writebacks: Vec<(u64, bool)> = events
        .iter()
        .filter(|e| e.target == DOWNSTREAM)
        .filter_map(|e| match e.packet {
            Packet::MemReq { paddr, op: MemOp::Writeback { dirty }, size: AccessSize::Line, .. } => Some((paddr.val(), dirty)),
            _ => None,
        })
        .collect();
    assert_eq!(writebacks, vec![(0x0000, true)], "the dirty LRU victim goes down as a dirty writeback");
    assert!(!bench.cache.contains(0x0000));
    assert!(bench.cache.contains(0x0080));
    assert_eq!(bench.stat("test.evictions"), 1);
    assert_eq!(bench.stat("test.writebacks"), 1);
    assert_eq!(bench.cache.writebacks().len(), 1);

    // Evicting the clean line produces no writeback.
    bench.read(4, 0x0180);
    let fetch = bench.downstream_requests();
    bench.fill(fetch[0].0, 0x0180);
    let events = bench.drain();
    assert!(events.iter().all(|e| !matches!(e.packet, Packet::MemReq { op: MemOp::Writeback { .. }, .. })));
    assert_eq!(bench.stat("test.evictions"), 2);
}

#[test]
fn a_full_writeback_buffer_blocks_requests_until_the_next_level_acks() {
    let mut config = test_config();
    config.write_buffers = 1;
    let mut bench = Bench::new(cache_with(&config));
    bench.install(1, 0x0000, MemOp::Write { data: WriteData::Small(1) });
    bench.install(2, 0x0080, MemOp::Read);
    bench.read(3, 0x0100);
    let fetch = bench.downstream_requests();
    bench.fill(fetch[0].0, 0x0100);
    let events = bench.drain();
    let writeback_id = events
        .iter()
        .find_map(|e| match e.packet {
            Packet::MemReq { req_id, op: MemOp::Writeback { .. }, .. } => Some(req_id),
            _ => None,
        })
        .expect("dirty victim written back");

    bench.read(4, 0x0080);
    assert!(bench.drain().is_empty(), "hit is held while the writeback buffer is full");
    assert_eq!(bench.cache.blocked_requests(), 1);

    bench.deliver(
        Packet::MemResp { req_id: writeback_id, line_addr: LineAddr::from_phys(PhysAddr::new(0), 64), data: MemRespData::Small(0), hit_level: HitLevel::Dram, state: MesiState::Exclusive },
        DOWNSTREAM,
    );
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE), vec![(ReqId::new(4), bench.cycle + LATENCY)]);
}

#[test]
fn inclusive_evictions_back_invalidate_upstream_and_nine_ones_do_not() {
    for (policy, expect_inval) in [(InclusionPolicy::Inclusive, true), (InclusionPolicy::Nine, false)] {
        let mut cache = cache_with(&test_config());
        cache.add_upstream(UPSTREAM);
        cache.set_upstream_inclusion(policy);
        let mut bench = Bench::new(cache);
        bench.install(1, 0x0000, MemOp::Read);
        bench.install(2, 0x0080, MemOp::Read);
        bench.read(3, 0x0100);
        let fetch = bench.downstream_requests();
        bench.fill(fetch[0].0, 0x0100);
        let events = bench.drain();
        let invals: Vec<u64> = events
            .iter()
            .filter(|e| e.target == UPSTREAM)
            .filter_map(|e| match e.packet { Packet::CacheInval { line_addr } => Some(line_addr.val()), _ => None })
            .collect();
        assert_eq!(invals, if expect_inval { vec![0x0000] } else { vec![] }, "{policy:?}");
    }
}

#[test]
fn back_invalidation_of_a_dirty_line_writes_it_back_and_propagates() {
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    cache.set_upstream_inclusion(InclusionPolicy::Inclusive);
    let mut bench = Bench::new(cache);
    bench.install(1, 0x1000, MemOp::Write { data: WriteData::Small(1) });

    bench.deliver(Packet::CacheInval { line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64) }, DOWNSTREAM);
    let events = bench.drain();
    assert!(!bench.cache.contains(0x1000));
    assert!(events.iter().any(|e| e.target == DOWNSTREAM && matches!(e.packet, Packet::MemReq { op: MemOp::Writeback { dirty: true }, .. })));
    assert!(events.iter().any(|e| e.target == UPSTREAM && matches!(e.packet, Packet::CacheInval { .. })));
    assert_eq!(bench.stat("test.back_invalidations"), 1);
}

#[test]
fn a_prefetch_is_a_real_fetch_that_a_demand_miss_can_join() {
    let mut config = test_config();
    config.prefetcher = PrefetcherType::NextLine;
    config.size_bytes = 1024;
    let mut bench = Bench::new(cache_with(&config));

    bench.read(1, 0x1000);
    let requests = bench.downstream_requests();
    let addrs: Vec<u64> = requests.iter().map(|r| r.1).collect();
    assert_eq!(addrs, vec![0x1000, 0x1040], "demand line then the next-line prefetch");
    assert_eq!(bench.stat("test.prefetches.issued"), 1);

    bench.read(2, 0x1048);
    let addrs: Vec<u64> = bench.downstream_requests().iter().map(|r| r.1).collect();
    assert_eq!(addrs, vec![0x1080], "the demand miss joins the prefetch MSHR; only its own next line is fetched");
    assert_eq!(bench.stat("test.prefetches.useful"), 1);
    assert_eq!(bench.stat("test.mshr_hits"), 1);
    assert_eq!(bench.stat("test.prefetches.issued"), 2);
}

#[test]
fn prefetches_leave_one_mshr_for_demand_misses() {
    let mut config = test_config();
    config.prefetcher = PrefetcherType::NextLine;
    config.mshr_count = 1;
    let mut bench = Bench::new(cache_with(&config));
    bench.read(1, 0x1000);
    let requests = bench.downstream_requests();
    assert_eq!(requests.len(), 1, "no prefetch when it would take the last MSHR");
}

#[test]
fn a_disabled_cache_forwards_and_routes_the_response_back() {
    let mut config = test_config();
    config.enabled = false;
    let mut bench = Bench::new(cache_with(&config));

    bench.read(9, 0x1000);
    let requests = bench.downstream_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].3, bench.cycle, "pass-through adds no latency");
    bench.fill(requests[0].0, 0x1000);
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE), vec![(ReqId::new(9), bench.cycle)]);
    assert!(!bench.cache.contains(0x1000));
}

#[test]
fn a_writeback_for_an_absent_line_is_forwarded_and_acked() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.deliver(
        Packet::MemReq { req_id: ReqId::new(5), paddr: PhysAddr::new(0x3000), vaddr: None, size: AccessSize::Line, op: MemOp::Writeback { dirty: true } },
        PIPELINE,
    );
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE), vec![(ReqId::new(5), bench.cycle + LATENCY)]);
    assert!(events.iter().any(|e| e.target == DOWNSTREAM && matches!(e.packet, Packet::MemReq { op: MemOp::Writeback { dirty: true }, .. })));
    assert!(!bench.cache.contains(0x3000), "writebacks do not allocate");
}

#[test]
fn a_writeback_for_a_held_line_marks_it_dirty_in_place() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x0000, MemOp::Read);
    bench.deliver(
        Packet::MemReq { req_id: ReqId::new(5), paddr: PhysAddr::new(0x0000), vaddr: None, size: AccessSize::Line, op: MemOp::Writeback { dirty: true } },
        PIPELINE,
    );
    let events = bench.drain();
    assert!(events.iter().all(|e| e.target != DOWNSTREAM), "merged, not forwarded");
    let dirty = bench.cache.flush();
    assert_eq!(dirty.len(), 1);
}

#[test]
fn exclusive_lower_level_gives_up_its_copy_when_it_fills_the_upper_one() {
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    cache.set_upstream_inclusion(InclusionPolicy::Exclusive);
    let mut bench = Bench::new(cache);
    bench.install(1, 0x1000, MemOp::Read);

    bench.deliver(
        Packet::MemReq { req_id: ReqId::new(2), paddr: PhysAddr::new(0x1000), vaddr: None, size: AccessSize::Line, op: MemOp::Read },
        UPSTREAM,
    );
    let events = bench.drain();
    assert_eq!(responses_to(&events, UPSTREAM).len(), 1);
    assert!(!bench.cache.contains(0x1000));
}

#[test]
fn exclusive_upper_level_hands_clean_victims_down() {
    let mut cache = cache_with(&test_config());
    cache.set_clean_victims_to_downstream(true);
    let mut bench = Bench::new(cache);
    bench.install(1, 0x0000, MemOp::Read);
    bench.install(2, 0x0080, MemOp::Read);
    bench.read(3, 0x0100);
    let fetch = bench.downstream_requests();
    bench.fill(fetch[0].0, 0x0100);
    let events = bench.drain();
    assert!(events.iter().any(|e| matches!(e.packet, Packet::MemReq { op: MemOp::Writeback { dirty: false }, .. })));
}

#[test]
fn maintenance_operations_report_dirty_lines_for_the_caller_to_write_back() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Write { data: WriteData::Small(1) });
    bench.install(2, 0x2000, MemOp::Read);

    assert_eq!(bench.cache.clean_line(0x1000).map(|d| d.line.val()), Some(0x1000));
    assert!(bench.cache.contains(0x1000));
    assert!(bench.cache.clean_line(0x1000).is_none(), "already clean");
    assert!(bench.cache.invalidate_line(0x2000).is_none(), "clean line: nothing to write back");
    assert!(!bench.cache.contains(0x2000));

    bench.install(3, 0x3000, MemOp::Write { data: WriteData::Small(1) });
    let dirty: Vec<u64> = bench.cache.flush().into_iter().map(|d| d.line.val()).collect();
    assert_eq!(dirty, vec![0x3000]);
    assert!(bench.cache.contains(0x1000), "clean lines survive a flush");
    assert!(!bench.cache.contains(0x3000));
    assert!(bench.cache.invalidate_all().is_empty());
    assert!(!bench.cache.contains(0x1000));
}

#[test]
fn fills_arriving_out_of_order_never_duplicate_a_tag() {
    let mut bench = Bench::new(cache_with(&test_config()));
    let mut ids = Vec::new();
    for (i, addr) in [0x0000u64, 0x0080, 0x0100, 0x0040].iter().enumerate() {
        bench.read(i as u64, *addr);
        ids.extend(bench.downstream_requests().into_iter().map(|r| (r.0, r.1)));
    }
    for (id, addr) in ids.into_iter().rev() {
        bench.fill(id, addr);
        let _ = bench.drain();
        assert!(bench.cache.duplicate_lines().is_empty());
    }
}

fn granted_states(events: &[Event], target: ComponentId) -> Vec<MesiState> {
    events
        .iter()
        .filter(|e| e.target == target)
        .filter_map(|e| match &e.packet {
            Packet::MemResp { state, .. } => Some(*state),
            _ => None,
        })
        .collect()
}

#[test]
fn a_write_miss_fetches_for_ownership_and_a_read_miss_for_sharing() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.write(1, 0x1000);
    bench.read(2, 0x2000);
    let ops: Vec<MemOp> = bench.downstream_requests().into_iter().map(|r| r.2).collect();
    assert!(matches!(ops[0], MemOp::ReadOwn));
    assert!(matches!(ops[1], MemOp::Read));
}

#[test]
fn fills_install_the_granted_state_and_hits_grant_it_onward() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.read(1, 0x1000);
    let fetch = bench.downstream_requests();
    bench.fill_with(fetch[0].0, 0x1000, MesiState::Shared);
    let events = bench.drain();
    assert_eq!(granted_states(&events, PIPELINE), vec![MesiState::Shared]);
    assert_eq!(bench.state_of(0x1000), Some(MesiState::Shared));

    bench.read(2, 0x2000);
    let fetch = bench.downstream_requests();
    bench.fill_with(fetch[0].0, 0x2000, MesiState::Modified);
    let events = bench.drain();
    assert_eq!(granted_states(&events, PIPELINE), vec![MesiState::Exclusive], "a read never installs dirtier than clean-exclusive");

    bench.read(3, 0x1000);
    let events = bench.drain();
    assert_eq!(granted_states(&events, PIPELINE), vec![MesiState::Shared], "a hit grants the line's own state");
    bench.write(4, 0x2000);
    let events = bench.drain();
    assert_eq!(granted_states(&events, PIPELINE), vec![MesiState::Modified]);
}

#[test]
fn a_write_to_a_shared_line_is_a_permission_miss() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.read(1, 0x1000);
    let fetch = bench.downstream_requests();
    bench.fill_with(fetch[0].0, 0x1000, MesiState::Shared);
    let _ = bench.drain();

    bench.write(2, 0x1008);
    let requests = bench.downstream_requests();
    assert_eq!(requests.len(), 1, "ownership is requested from the next level");
    assert!(matches!(requests[0].2, MemOp::ReadOwn));
    assert_eq!(bench.stat("test.misses"), 2);

    bench.fill_with(requests[0].0, 0x1000, MesiState::Modified);
    let events = bench.drain();
    assert_eq!(granted_states(&events, PIPELINE), vec![MesiState::Modified]);
    assert_eq!(bench.state_of(0x1000), Some(MesiState::Modified));
    assert!(bench.cache.duplicate_lines().is_empty());
}

#[test]
fn an_invalidating_probe_writes_a_dirty_line_back_before_answering() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Write { data: WriteData::Small(1) });

    let txn = ReqId::new(77);
    bench.deliver(Packet::Probe { line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64), kind: ProbeKind::Invalidate, txn }, DOWNSTREAM);
    let events = bench.drain();
    let order: Vec<&str> = events
        .iter()
        .filter(|e| e.target == DOWNSTREAM)
        .map(|e| match e.packet {
            Packet::MemReq { op: MemOp::Writeback { dirty: true }, .. } => "writeback",
            Packet::ProbeResp { dirty: true, .. } => "resp-dirty",
            Packet::ProbeResp { dirty: false, .. } => "resp-clean",
            _ => "other",
        })
        .collect();
    assert_eq!(order, vec!["writeback", "resp-dirty"]);
    assert!(events.iter().any(|e| matches!(e.packet, Packet::ProbeResp { txn: t, .. } if t == txn)));
    assert!(!bench.cache.contains(0x1000));
    assert_eq!(bench.stat("test.probes"), 1);
}

#[test]
fn a_downgrade_probe_leaves_a_shared_copy() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Read);
    bench.deliver(Packet::Probe { line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64), kind: ProbeKind::Downgrade, txn: ReqId::new(1) }, DOWNSTREAM);
    let events = bench.drain();
    assert!(events.iter().any(|e| matches!(e.packet, Packet::ProbeResp { dirty: false, .. })));
    assert!(events.iter().all(|e| !matches!(e.packet, Packet::MemReq { .. })), "clean line: no writeback");
    assert_eq!(bench.state_of(0x1000), Some(MesiState::Shared));
}

#[test]
fn a_probe_that_hits_an_in_flight_fetch_is_applied_after_the_fill() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.read(1, 0x1000);
    let fetch = bench.downstream_requests();
    bench.deliver(Packet::Probe { line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64), kind: ProbeKind::Invalidate, txn: ReqId::new(5) }, DOWNSTREAM);
    let events = bench.drain();
    assert!(events.iter().any(|e| matches!(e.packet, Packet::ProbeResp { dirty: false, .. })), "answered at once: the line is not here yet");

    bench.fill(fetch[0].0, 0x1000);
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE).len(), 1, "the waiting load still gets its data");
    assert!(!bench.cache.contains(0x1000), "then the probe takes the line away");
}

#[test]
fn a_probe_is_forwarded_upstream_and_answered_once_every_copy_replied() {
    let third = ComponentId::Cache(CacheId::new(3));
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    cache.add_upstream(third);
    let mut bench = Bench::new(cache);
    bench.install(1, 0x1000, MemOp::Read);

    bench.deliver(Packet::Probe { line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64), kind: ProbeKind::Invalidate, txn: ReqId::new(9) }, DOWNSTREAM);
    let events = bench.drain();
    let forwarded: Vec<ReqId> = events
        .iter()
        .filter(|e| e.target == UPSTREAM || e.target == third)
        .filter_map(|e| match e.packet { Packet::Probe { txn, .. } => Some(txn), _ => None })
        .collect();
    assert_eq!(forwarded.len(), 2);
    assert!(events.iter().all(|e| !matches!(e.packet, Packet::ProbeResp { .. })), "not answered yet");

    let line = LineAddr::from_phys(PhysAddr::new(0x1000), 64);
    bench.deliver(Packet::ProbeResp { line_addr: line, txn: forwarded[0], dirty: false }, UPSTREAM);
    assert!(bench.drain().is_empty());
    bench.deliver(Packet::ProbeResp { line_addr: line, txn: forwarded[1], dirty: true }, third);
    let events = bench.drain();
    assert!(events.iter().any(|e| e.target == DOWNSTREAM && matches!(e.packet, Packet::ProbeResp { txn, dirty: true, .. } if txn == ReqId::new(9))));
    assert!(!bench.cache.contains(0x1000));
}
