//! The event-driven cache: hits, misses through MSHRs, blocking, fills,
//! writebacks, inclusion and prefetching, driven packet by packet.
//!
//! A test cache is 256 bytes with 64-byte lines and 2 ways: two sets,
//! set = (addr / 64) % 2, tag = addr / 128.

use crate::common::{HartId, LineAddr, PhysAddr};
use crate::config::{CacheConfig, Config, InclusionPolicy, PrefetcherKind, ReplacementPolicyKind};
use crate::sim::components::{CacheId, ComponentId, PipelineId, ReqId};
use crate::sim::events::{Event, EventQueue};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::memory::{GlobalMemory, Ram};
use crate::sim::packet::WriteOrigin;
use crate::sim::packet::{
    AccessSize, CacheLevel, HitLevel, Maintenance, MemOp, MemRespData, MesiState, Packet,
    ProbeKind, WriteData,
};
use crate::sim::stats::{StatSource, Stats};
use crate::soc::cache::Cache;

const LATENCY: u64 = 2;
const RESPONSE_LATENCY: u64 = 1;
const HART: WriteOrigin = WriteOrigin::Hart(HartId::new(0));
const RAM_BYTES: usize = 0x1_0000;
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
        policy: ReplacementPolicyKind::Lru,
        latency: LATENCY,
        response_latency: RESPONSE_LATENCY,
        prefetcher: PrefetcherKind::None,
        prefetch_table_size: 64,
        prefetch_degree: 1,
        mshr_count: 4,
        write_buffers: 4,
        targets_per_mshr: 8,
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
    memory: GlobalMemory,
    config: Config,
    cycle: u64,
}

impl Bench {
    fn new(cache: Cache) -> Self {
        let mut stats = Stats::new();
        cache.stat_paths.register(&mut stats);
        Self {
            cache,
            queue: EventQueue::new(),
            stats,
            memory: GlobalMemory::new(Some(Ram::new(0, RAM_BYTES)), 1, 64),
            config: Config::default(),
            cycle: 100,
        }
    }

    fn deliver(&mut self, packet: Packet, source: ComponentId) {
        let mut ctx = HandleCtx {
            scheduler: &mut self.queue,
            stats: &mut self.stats,
            memory: &mut self.memory,
            config: &self.config,
            cycle: self.cycle,
            self_id: SELF,
        };
        self.cache.handle(packet, source, &mut ctx);
    }

    fn request(&mut self, req_id: u64, addr: u64, op: MemOp) {
        self.request_from(PIPELINE, req_id, addr, op);
    }

    fn request_from(&mut self, source: ComponentId, req_id: u64, addr: u64, op: MemOp) {
        self.deliver(
            Packet::MemReq {
                req_id: ReqId::new(req_id),
                paddr: PhysAddr::new(addr),
                vaddr: None,
                pc: None,
                size: AccessSize::B8,
                op,
            },
            source,
        );
    }

    fn read(&mut self, req_id: u64, addr: u64) {
        self.request(req_id, addr, MemOp::Read);
    }

    fn write(&mut self, req_id: u64, addr: u64) {
        self.request(
            req_id,
            addr,
            MemOp::Write { data: WriteData::Small(1), origin: WriteOrigin::Hart(HartId::new(0)) },
        );
    }

    /// The 8 bytes of RAM at `addr`.
    fn ram_value(&self, addr: u64) -> u64 {
        self.memory.read(PhysAddr::new(addr), 8).expect("inside RAM")
    }

    /// Puts `value` in RAM at `addr`, behind the cache's back.
    fn set_ram(&mut self, addr: u64, value: u64) {
        self.memory.load(PhysAddr::new(addr), &value.to_le_bytes());
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
                Packet::MemReq { req_id, paddr, op, .. } => {
                    Some((req_id, paddr.val(), op, e.fire_at))
                }
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
        self.install_from(PIPELINE, req_id, addr, op);
    }

    /// Installs a line on behalf of `source`, which then holds a copy.
    fn install_from(&mut self, source: ComponentId, req_id: u64, addr: u64, op: MemOp) {
        self.request_from(source, req_id, addr, op);
        let requests = self.downstream_requests();
        let (down_id, _, _, _) = requests.into_iter().next().expect("miss forwarded downstream");
        self.fill(down_id, addr);
        let _ = self.drain();
    }

    /// The probes forwarded to caches above, as `(target, txn)`.
    fn forwarded_probes(&mut self) -> Vec<(ComponentId, ReqId)> {
        self.drain()
            .into_iter()
            .filter_map(|e| match e.packet {
                Packet::Probe { txn, .. } if e.target != DOWNSTREAM => Some((e.target, txn)),
                _ => None,
            })
            .collect()
    }

    fn probe_from_below(&mut self, addr: u64, txn: u64) {
        self.deliver(
            Packet::Probe {
                line_addr: LineAddr::from_phys(PhysAddr::new(addr), 64),
                kind: ProbeKind::Invalidate,
                txn: ReqId::new(txn),
            },
            DOWNSTREAM,
        );
    }

    fn stat(&self, path: &str) -> u64 {
        self.stats.get(path).unwrap_or(0.0) as u64
    }
}

/// The values the responses to `target` carry, as `(req_id, value)`.
fn values_read_by(events: &[Event], target: ComponentId) -> Vec<(ReqId, u64)> {
    events
        .iter()
        .filter(|e| e.target == target)
        .filter_map(|e| match &e.packet {
            Packet::MemResp { req_id, data: MemRespData::Performed { value, .. }, .. } => {
                Some((*req_id, *value))
            }
            _ => None,
        })
        .collect()
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
    assert_eq!(
        responses_to(&events, PIPELINE),
        vec![(ReqId::new(7), bench.cycle + RESPONSE_LATENCY)],
        "the fill is forwarded to its request as it is written"
    );
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
    let answered: Vec<ReqId> =
        responses_to(&events, PIPELINE).into_iter().map(|(id, _)| id).collect();
    assert_eq!(answered, vec![ReqId::new(1), ReqId::new(2)]);
    assert!(bench.cache.duplicate_lines().is_empty());
}

#[test]
fn a_hit_reads_memory_when_it_is_served_not_when_it_is_answered() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Read);
    bench.set_ram(0x1000, 0xAA);

    bench.read(2, 0x1000);
    bench.set_ram(0x1000, 0xBB);

    assert_eq!(values_read_by(&bench.drain(), PIPELINE), vec![(ReqId::new(2), 0xAA)]);
}

#[test]
fn a_miss_reads_memory_when_its_fill_arrives() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.set_ram(0x1000, 0xAA);
    bench.read(1, 0x1000);
    let fetch = bench.downstream_requests();

    bench.set_ram(0x1000, 0xBB);
    bench.fill(fetch[0].0, 0x1000);

    assert_eq!(values_read_by(&bench.drain(), PIPELINE), vec![(ReqId::new(1), 0xBB)]);
}

#[test]
fn a_store_that_hits_writes_memory_as_it_is_served() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Write { data: WriteData::Small(0), origin: HART });

    bench.request(2, 0x1000, MemOp::Write { data: WriteData::Small(0xAB), origin: HART });

    assert_eq!(bench.ram_value(0x1000), 0xAB);
}

#[test]
fn a_store_that_misses_writes_memory_only_when_its_line_arrives() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.request(1, 0x1000, MemOp::Write { data: WriteData::Small(0xAB), origin: HART });
    let fetch = bench.downstream_requests();
    assert_eq!(bench.ram_value(0x1000), 0, "nothing written while the line is fetched");

    bench.fill(fetch[0].0, 0x1000);

    assert_eq!(bench.ram_value(0x1000), 0xAB);
}

#[test]
fn a_write_whose_bytes_are_already_placed_does_not_write_memory() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Read);
    bench.set_ram(0x1000, 0x11);

    bench.request(
        2,
        0x1000,
        MemOp::Write { data: WriteData::Small(0xAB), origin: WriteOrigin::Placed },
    );

    assert_eq!(bench.ram_value(0x1000), 0x11);
}

/// The maintenance requests `events` sends the next level, as
/// `(req_id, op, dirty)`.
fn maintenance_sent(events: &[Event]) -> Vec<(ReqId, Maintenance, bool)> {
    events
        .iter()
        .filter(|e| e.target == DOWNSTREAM)
        .filter_map(|e| match e.packet {
            Packet::MemReq { req_id, op: MemOp::Maintain { op, dirty }, .. } => {
                Some((req_id, op, dirty))
            }
            _ => None,
        })
        .collect()
}

fn maintain(bench: &mut Bench, req_id: u64, addr: u64, op: Maintenance) {
    bench.request(req_id, addr, MemOp::Maintain { op, dirty: false });
}

fn line_state(bench: &Bench, addr: u64) -> Option<MesiState> {
    let line = LineAddr::from_phys(PhysAddr::new(addr), 64);
    bench.cache.held_lines().into_iter().find(|(held, _)| *held == line).map(|(_, state)| state)
}

#[test]
fn a_clean_keeps_the_line_and_carries_its_dirty_data_down() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Write { data: WriteData::Small(1), origin: HART });

    maintain(&mut bench, 2, 0x1008, Maintenance::Clean);

    let sent = maintenance_sent(&bench.drain());
    assert_eq!(
        sent.iter().map(|&(_, op, dirty)| (op, dirty)).collect::<Vec<_>>(),
        [(Maintenance::Clean, true)]
    );
    assert_eq!(line_state(&bench, 0x1000), Some(MesiState::Exclusive));
}

#[test]
fn a_flush_drops_the_line_and_carries_its_dirty_data_down() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Write { data: WriteData::Small(1), origin: HART });

    maintain(&mut bench, 2, 0x1000, Maintenance::Flush);

    let sent = maintenance_sent(&bench.drain());
    assert_eq!(
        sent.iter().map(|&(_, op, dirty)| (op, dirty)).collect::<Vec<_>>(),
        [(Maintenance::Flush, true)]
    );
    assert_eq!(line_state(&bench, 0x1000), None);
}

#[test]
fn an_invalidate_drops_the_line_and_discards_its_dirty_data() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Write { data: WriteData::Small(1), origin: HART });

    maintain(&mut bench, 2, 0x1000, Maintenance::Invalidate);

    let sent = maintenance_sent(&bench.drain());
    assert_eq!(
        sent.iter().map(|&(_, op, dirty)| (op, dirty)).collect::<Vec<_>>(),
        [(Maintenance::Invalidate, false)]
    );
    assert_eq!(line_state(&bench, 0x1000), None);
}

#[test]
fn a_maintenance_operation_is_answered_once_the_next_level_answers_it() {
    let mut bench = Bench::new(cache_with(&test_config()));
    maintain(&mut bench, 7, 0x1000, Maintenance::Flush);
    let sent = maintenance_sent(&bench.drain());
    assert!(responses_to(&bench.drain(), PIPELINE).is_empty());

    bench.fill(sent[0].0, 0x1000);

    assert_eq!(
        responses_to(&bench.drain(), PIPELINE).iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [ReqId::new(7)]
    );
}

#[test]
fn a_maintenance_operation_waits_for_its_lines_fetch_to_fill() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.write(1, 0x1000);
    let fetch = bench.downstream_requests();

    maintain(&mut bench, 2, 0x1000, Maintenance::Flush);
    assert!(maintenance_sent(&bench.drain()).is_empty(), "held while the line is fetched");
    bench.fill_with(fetch[0].0, 0x1000, MesiState::Modified);

    let sent = maintenance_sent(&bench.drain());
    assert_eq!(
        sent.iter().map(|&(_, op, dirty)| (op, dirty)).collect::<Vec<_>>(),
        [(Maintenance::Flush, true)]
    );
    assert_eq!(line_state(&bench, 0x1000), None);
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
    assert_eq!(
        responses_to(&events, PIPELINE),
        vec![(ReqId::new(1), bench.cycle + RESPONSE_LATENCY)]
    );
    let retried: Vec<u64> = events
        .iter()
        .filter(|e| e.target == DOWNSTREAM)
        .filter_map(|e| match e.packet {
            Packet::MemReq { paddr, .. } => Some(paddr.val()),
            _ => None,
        })
        .collect();
    assert_eq!(retried, vec![0x2000], "the queued miss is fetched once an MSHR frees");
    assert_eq!(bench.cache.blocked_requests(), 0);
}

#[test]
fn an_mshr_holding_its_target_limit_blocks_the_cache_until_its_fill() {
    let mut config = test_config();
    config.targets_per_mshr = 2;
    let mut bench = Bench::new(cache_with(&config));
    bench.read(1, 0x1000);
    let first = bench.downstream_requests();

    bench.read(2, 0x1008);
    bench.read(3, 0x2000);

    assert!(bench.downstream_requests().is_empty(), "a free MSHR does not unblock the cache");
    assert_eq!(bench.cache.blocked_requests(), 1);
    bench.fill(first[0].0, 0x1000);
    let events = bench.drain();
    let answered_at = bench.cycle + RESPONSE_LATENCY;
    assert_eq!(
        responses_to(&events, PIPELINE),
        vec![(ReqId::new(1), answered_at), (ReqId::new(2), answered_at)]
    );
    assert!(
        events.iter().any(|e| e.target == DOWNSTREAM
            && matches!(e.packet, Packet::MemReq { paddr, .. } if paddr.val() == 0x2000)),
        "the queued miss is fetched once the full MSHR's fill returns"
    );
}

#[test]
fn a_dirty_victim_is_written_back_and_a_clean_one_is_dropped() {
    let mut bench = Bench::new(cache_with(&test_config()));
    // Set 0 holds tags for 0x0000 and 0x0080; 0x0100 evicts the LRU one.
    bench.install(
        1,
        0x0000,
        MemOp::Write { data: WriteData::Small(1), origin: WriteOrigin::Hart(HartId::new(0)) },
    );
    bench.install(2, 0x0080, MemOp::Read);

    bench.read(3, 0x0100);
    let fetch = bench.downstream_requests();
    bench.fill(fetch[0].0, 0x0100);
    let events = bench.drain();
    let writebacks: Vec<(u64, bool)> = events
        .iter()
        .filter(|e| e.target == DOWNSTREAM)
        .filter_map(|e| match e.packet {
            Packet::MemReq {
                paddr,
                op: MemOp::Writeback { dirty },
                size: AccessSize::Line,
                ..
            } => Some((paddr.val(), dirty)),
            _ => None,
        })
        .collect();
    assert_eq!(
        writebacks,
        vec![(0x0000, true)],
        "the dirty LRU victim goes down as a dirty writeback"
    );
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
    assert!(
        events
            .iter()
            .all(|e| !matches!(e.packet, Packet::MemReq { op: MemOp::Writeback { .. }, .. }))
    );
    assert_eq!(bench.stat("test.evictions"), 2);
}

#[test]
fn a_full_writeback_buffer_blocks_requests_until_the_next_level_acks() {
    let mut config = test_config();
    config.write_buffers = 1;
    let mut bench = Bench::new(cache_with(&config));
    bench.install(
        1,
        0x0000,
        MemOp::Write { data: WriteData::Small(1), origin: WriteOrigin::Hart(HartId::new(0)) },
    );
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
        Packet::MemResp {
            req_id: writeback_id,
            line_addr: LineAddr::from_phys(PhysAddr::new(0), 64),
            data: MemRespData::Small(0),
            hit_level: HitLevel::Dram,
            state: MesiState::Exclusive,
        },
        DOWNSTREAM,
    );
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE), vec![(ReqId::new(4), bench.cycle + LATENCY)]);
}

#[test]
fn inclusive_evictions_back_invalidate_upstream_and_nine_ones_do_not() {
    for (policy, expect_inval) in
        [(InclusionPolicy::Inclusive, true), (InclusionPolicy::Nine, false)]
    {
        let mut cache = cache_with(&test_config());
        cache.add_upstream(UPSTREAM);
        cache.set_upstream_inclusion(policy);
        let mut bench = Bench::new(cache);
        bench.install_from(UPSTREAM, 1, 0x0000, MemOp::Read);
        bench.install_from(UPSTREAM, 2, 0x0080, MemOp::Read);
        bench.read(3, 0x0100);
        let fetch = bench.downstream_requests();
        bench.fill(fetch[0].0, 0x0100);
        let events = bench.drain();
        let invals: Vec<u64> = events
            .iter()
            .filter(|e| e.target == UPSTREAM)
            .filter_map(|e| match e.packet {
                Packet::CacheInval { line_addr } => Some(line_addr.val()),
                _ => None,
            })
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
    bench.install_from(
        UPSTREAM,
        1,
        0x1000,
        MemOp::Write { data: WriteData::Small(1), origin: WriteOrigin::Hart(HartId::new(0)) },
    );

    bench.deliver(
        Packet::CacheInval { line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64) },
        DOWNSTREAM,
    );
    let events = bench.drain();
    assert!(!bench.cache.contains(0x1000));
    assert!(events.iter().any(|e| e.target == DOWNSTREAM
        && matches!(e.packet, Packet::MemReq { op: MemOp::Writeback { dirty: true }, .. })));
    assert!(
        events
            .iter()
            .any(|e| e.target == UPSTREAM && matches!(e.packet, Packet::CacheInval { .. }))
    );
    assert_eq!(bench.stat("test.back_invalidations"), 1);
}

#[test]
fn a_prefetch_is_a_real_fetch_that_a_demand_miss_can_join() {
    let mut config = test_config();
    config.prefetcher = PrefetcherKind::NextLine;
    config.size_bytes = 1024;
    let mut bench = Bench::new(cache_with(&config));

    bench.read(1, 0x1000);
    let requests = bench.downstream_requests();
    let addrs: Vec<u64> = requests.iter().map(|r| r.1).collect();
    assert_eq!(addrs, vec![0x1000, 0x1040], "demand line then the next-line prefetch");
    assert_eq!(bench.stat("test.prefetches.issued"), 1);

    bench.read(2, 0x1048);
    let addrs: Vec<u64> = bench.downstream_requests().iter().map(|r| r.1).collect();
    assert_eq!(
        addrs,
        vec![0x1080],
        "the demand miss joins the prefetch MSHR; only its own next line is fetched"
    );
    assert_eq!(bench.stat("test.prefetches.late"), 1);
    assert_eq!(bench.stat("test.mshr_hits"), 1);
    assert_eq!(bench.stat("test.prefetches.issued"), 2);
}

#[test]
fn a_prefetch_into_the_next_4k_page_is_dropped() {
    let mut config = test_config();
    config.prefetcher = PrefetcherKind::NextLine;
    let mut bench = Bench::new(cache_with(&config));

    bench.read(1, 0x1FC0);

    let addrs: Vec<u64> = bench.downstream_requests().iter().map(|r| r.1).collect();
    assert_eq!(addrs, vec![0x1FC0], "the next line is in the next page");
    assert_eq!(bench.stat("test.prefetches.page_crossing"), 1);
    assert_eq!(bench.stat("test.prefetches.issued"), 0);
}

const PREFETCH_L1D: MemOp = MemOp::Prefetch { into: CacheLevel::L1D, exclusive: false };

#[test]
fn a_prefetch_request_fetches_its_line_and_answers_no_one() {
    let mut bench = Bench::new(cache_with(&test_config()));

    bench.request(1, 0x1000, PREFETCH_L1D);
    let requests = bench.downstream_requests();
    let (down_id, addr, op, _) = requests[0].clone();
    bench.fill(down_id, 0x1000);
    let events = bench.drain();

    assert_eq!((requests.len(), addr), (1, 0x1000));
    assert!(matches!(op, MemOp::Read));
    assert_eq!(bench.state_of(0x1000), Some(MesiState::Exclusive));
    assert!(responses_to(&events, PIPELINE).is_empty());
    assert_eq!(bench.stat("test.prefetches.issued"), 1);
}

/// Prefetches `addr` into the cache and fills it, with no request
/// joining the fetch.
fn prefetch_and_fill(bench: &mut Bench, req_id: u64, addr: u64) {
    bench.request(req_id, addr, PREFETCH_L1D);
    let (down_id, _, _, _) = bench.downstream_requests()[0].clone();
    bench.fill(down_id, addr);
    let _ = bench.drain();
}

#[test]
fn the_first_request_to_find_a_prefetched_line_makes_the_prefetch_useful() {
    let mut bench = Bench::new(cache_with(&test_config()));
    prefetch_and_fill(&mut bench, 1, 0x1000);

    bench.read(2, 0x1000);
    bench.read(3, 0x1008);

    assert_eq!(bench.stat("test.prefetches.useful"), 1);
    assert_eq!(bench.stat("test.prefetches.late"), 0);
    assert_eq!(bench.stats.get("test.prefetches.accuracy"), Some(1.0));
}

#[test]
fn a_late_prefetch_is_not_counted_useful_when_its_line_is_found_again() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.request(1, 0x1000, PREFETCH_L1D);
    let (down_id, _, _, _) = bench.downstream_requests()[0].clone();
    bench.read(2, 0x1000);
    bench.fill(down_id, 0x1000);
    let _ = bench.drain();

    bench.read(3, 0x1000);

    assert_eq!(bench.stat("test.prefetches.late"), 1);
    assert_eq!(bench.stat("test.prefetches.useful"), 0);
    assert_eq!(bench.stats.get("test.prefetches.used"), Some(1.0));
}

#[test]
fn a_prefetched_line_evicted_before_any_request_is_unused() {
    let mut bench = Bench::new(cache_with(&test_config()));
    prefetch_and_fill(&mut bench, 1, 0x1000);

    bench.install(2, 0x1080, MemOp::Read);
    bench.install(3, 0x1100, MemOp::Read);

    assert_eq!(bench.state_of(0x1000), None, "the set's two ways were refilled");
    assert_eq!(bench.stat("test.prefetches.unused"), 1);
    assert_eq!(bench.stat("test.prefetches.useful"), 0);
    assert_eq!(bench.stats.get("test.prefetches.accuracy"), Some(0.0));
}

#[test]
fn a_prefetched_line_a_probe_takes_before_any_request_is_unused() {
    let mut bench = Bench::new(cache_with(&test_config()));
    prefetch_and_fill(&mut bench, 1, 0x1000);

    bench.probe_from_below(0x1000, 9);
    let _ = bench.drain();

    assert_eq!(bench.state_of(0x1000), None);
    assert_eq!(bench.stat("test.prefetches.unused"), 1);
}

#[test]
fn an_evicted_demand_line_is_not_an_unused_prefetch() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Read);

    bench.install(2, 0x1080, MemOp::Read);
    bench.install(3, 0x1100, MemOp::Read);

    assert_eq!(bench.state_of(0x1000), None);
    assert_eq!(bench.stat("test.prefetches.unused"), 0);
}

#[test]
fn an_exclusive_prefetch_fetches_with_write_permission() {
    let mut bench = Bench::new(cache_with(&test_config()));

    bench.request(1, 0x1000, MemOp::Prefetch { into: CacheLevel::L1D, exclusive: true });

    let requests = bench.downstream_requests();
    assert_eq!(requests.len(), 1);
    assert!(matches!(requests[0].2, MemOp::ReadOwn));
}

#[test]
fn a_prefetch_of_a_held_or_inflight_line_is_dropped() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Read);
    bench.read(2, 0x2000);
    let _ = bench.drain();

    bench.request(3, 0x1000, PREFETCH_L1D);
    bench.request(4, 0x2000, PREFETCH_L1D);

    assert!(bench.downstream_requests().is_empty());
    assert_eq!(bench.stat("test.prefetches.issued"), 0);
}

#[test]
fn a_prefetch_for_a_lower_level_is_passed_down_untouched() {
    let mut bench = Bench::new(cache_with(&test_config()));
    let into_l2 = MemOp::Prefetch { into: CacheLevel::L2, exclusive: true };

    bench.request(1, 0x1000, into_l2);

    let requests = bench.downstream_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1, 0x1000);
    assert!(matches!(requests[0].2, MemOp::Prefetch { into: CacheLevel::L2, exclusive: true }));
    assert_eq!(bench.state_of(0x1000), None);
    assert_eq!(bench.stat("test.prefetches.issued"), 0);
}

#[test]
fn a_disabled_level_passes_a_prefetch_for_a_lower_level_down() {
    let mut config = test_config();
    config.enabled = false;
    let mut bench = Bench::new(cache_with(&config));

    bench.request(1, 0x1000, MemOp::Prefetch { into: CacheLevel::L2, exclusive: false });
    bench.request(2, 0x2000, PREFETCH_L1D);

    let addrs: Vec<u64> = bench.downstream_requests().iter().map(|r| r.1).collect();
    assert_eq!(addrs, vec![0x1000], "only the L2's prefetch goes on");
}

#[test]
fn a_run_of_store_misses_sends_exclusive_prefetches_to_the_l2() {
    let mut config = test_config();
    config.mshr_count = 8;
    let mut cache = cache_with(&config);
    cache.set_store_prefetcher(crate::soc::cache::prefetch::StoreStreamPrefetcher::new(64, 4, 2));
    let mut bench = Bench::new(cache);

    for (id, addr) in [(1, 0x1000), (2, 0x1040), (3, 0x1080)] {
        bench.write(id, addr);
    }

    let prefetches: Vec<u64> = bench
        .downstream_requests()
        .into_iter()
        .filter(|r| matches!(r.2, MemOp::Prefetch { into: CacheLevel::L2, exclusive: true }))
        .map(|r| r.1)
        .collect();
    assert_eq!(prefetches, vec![0x10C0, 0x1100]);
    assert_eq!(bench.stat("test.prefetches.store_stream"), 2);
}

#[test]
fn a_prefetch_request_is_dropped_rather_than_take_the_last_mshr() {
    let mut config = test_config();
    config.mshr_count = 2;
    let mut bench = Bench::new(cache_with(&config));
    bench.read(1, 0x1000);
    let _ = bench.drain();

    bench.request(2, 0x2000, PREFETCH_L1D);

    assert!(bench.downstream_requests().is_empty());
    assert_eq!(bench.stat("test.prefetches.dropped"), 1);
}

#[test]
fn prefetches_leave_one_mshr_for_demand_misses() {
    let mut config = test_config();
    config.prefetcher = PrefetcherKind::NextLine;
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
        Packet::MemReq {
            req_id: ReqId::new(5),
            paddr: PhysAddr::new(0x3000),
            vaddr: None,
            pc: None,
            size: AccessSize::Line,
            op: MemOp::Writeback { dirty: true },
        },
        PIPELINE,
    );
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE), vec![(ReqId::new(5), bench.cycle + LATENCY)]);
    assert!(events.iter().any(|e| e.target == DOWNSTREAM
        && matches!(e.packet, Packet::MemReq { op: MemOp::Writeback { dirty: true }, .. })));
    assert!(!bench.cache.contains(0x3000), "writebacks do not allocate");
}

#[test]
fn a_writeback_for_a_held_line_marks_it_dirty_in_place() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x0000, MemOp::Read);
    bench.deliver(
        Packet::MemReq {
            req_id: ReqId::new(5),
            paddr: PhysAddr::new(0x0000),
            vaddr: None,
            pc: None,
            size: AccessSize::Line,
            op: MemOp::Writeback { dirty: true },
        },
        PIPELINE,
    );
    let events = bench.drain();
    assert!(events.iter().all(|e| e.target != DOWNSTREAM), "merged, not forwarded");
    assert_eq!(
        bench.cache.held_lines(),
        vec![(LineAddr::from_phys(PhysAddr::new(0x0000), 64), MesiState::Modified)]
    );
}

/// A probe downgrades an upper cache's dirty copy and answers dirty; its
/// writeback can reach us after the probe completed, while we hold the line
/// Shared. The data was in the probe's answer, and a writeback carries no
/// permission, so the line stays Shared.
#[test]
fn a_late_dirty_writeback_does_not_upgrade_a_shared_line() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.read(1, 0x0000);
    let (down_id, _, _, _) = bench.downstream_requests().into_iter().next().expect("miss");
    bench.fill_with(down_id, 0x0000, MesiState::Shared);
    let _ = bench.drain();

    bench.request_from(UPSTREAM, 2, 0x0000, MemOp::Writeback { dirty: true });
    let _ = bench.drain();

    assert_eq!(bench.state_of(0x0000), Some(MesiState::Shared));
}

#[test]
fn exclusive_lower_level_gives_up_its_copy_when_it_fills_the_upper_one() {
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    cache.set_upstream_inclusion(InclusionPolicy::Exclusive);
    let mut bench = Bench::new(cache);
    bench.install(1, 0x1000, MemOp::Read);

    bench.deliver(
        Packet::MemReq {
            req_id: ReqId::new(2),
            paddr: PhysAddr::new(0x1000),
            vaddr: None,
            pc: None,
            size: AccessSize::Line,
            op: MemOp::Read,
        },
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
    assert!(
        events.iter().any(|e| matches!(
            e.packet,
            Packet::MemReq { op: MemOp::Writeback { dirty: false }, .. }
        ))
    );
}

/// An exclusive cache with one cache above it.
fn exclusive_bench() -> Bench {
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    cache.set_upstream_inclusion(InclusionPolicy::Exclusive);
    Bench::new(cache)
}

fn line_read(bench: &mut Bench, req_id: u64, addr: u64) {
    bench.deliver(
        Packet::MemReq {
            req_id: ReqId::new(req_id),
            paddr: PhysAddr::new(addr),
            vaddr: None,
            pc: None,
            size: AccessSize::Line,
            op: MemOp::Read,
        },
        UPSTREAM,
    );
}

fn victim(bench: &mut Bench, req_id: u64, addr: u64, dirty: bool) {
    bench.deliver(
        Packet::MemReq {
            req_id: ReqId::new(req_id),
            paddr: PhysAddr::new(addr),
            vaddr: None,
            pc: None,
            size: AccessSize::Line,
            op: MemOp::Writeback { dirty },
        },
        UPSTREAM,
    );
}

#[test]
fn an_exclusive_level_passes_a_line_fetched_for_the_level_above_without_keeping_it() {
    let mut bench = exclusive_bench();
    line_read(&mut bench, 1, 0x1000);
    let (down_id, _, _, _) = bench.downstream_requests()[0].clone();

    bench.fill(down_id, 0x1000);
    let events = bench.drain();

    assert_eq!(responses_to(&events, UPSTREAM).len(), 1);
    assert_eq!(bench.state_of(0x1000), None);
    assert_eq!(bench.stat("test.fills"), 0);
}

#[test]
fn an_exclusive_level_keeps_the_victim_the_level_above_hands_back() {
    let mut bench = exclusive_bench();
    line_read(&mut bench, 1, 0x1000);
    let (down_id, _, _, _) = bench.downstream_requests()[0].clone();
    bench.fill(down_id, 0x1000);
    let _ = bench.drain();

    victim(&mut bench, 2, 0x1000, true);
    let events = bench.drain();

    assert_eq!(bench.state_of(0x1000), Some(MesiState::Modified));
    assert!(!events.iter().any(|e| e.target == DOWNSTREAM), "the victim stays here, not passed on");
}

#[test]
fn an_exclusive_level_does_not_prefetch_a_line_held_above() {
    let mut bench = exclusive_bench();
    line_read(&mut bench, 1, 0x1000);
    let (down_id, _, _, _) = bench.downstream_requests()[0].clone();
    bench.fill(down_id, 0x1000);
    let _ = bench.drain();

    bench.request(2, 0x1000, PREFETCH_L1D);

    assert!(bench.downstream_requests().is_empty());
    assert_eq!(bench.stat("test.prefetches.issued"), 0);
}

#[test]
fn a_flush_from_above_lets_an_exclusive_level_prefetch_the_line_again() {
    let mut bench = exclusive_bench();
    line_read(&mut bench, 1, 0x1000);
    let (down_id, _, _, _) = bench.downstream_requests()[0].clone();
    bench.fill(down_id, 0x1000);
    let _ = bench.drain();
    bench.request_from(
        UPSTREAM,
        2,
        0x1000,
        MemOp::Maintain { op: Maintenance::Flush, dirty: false },
    );
    let _ = bench.drain();

    bench.request(3, 0x1000, PREFETCH_L1D);

    assert_eq!(bench.stat("test.prefetches.issued"), 1);
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
    assert_eq!(
        granted_states(&events, PIPELINE),
        vec![MesiState::Exclusive],
        "a read never installs dirtier than clean-exclusive"
    );

    bench.read(3, 0x1000);
    let events = bench.drain();
    assert_eq!(
        granted_states(&events, PIPELINE),
        vec![MesiState::Shared],
        "a hit grants the line's own state"
    );
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
fn a_write_joining_a_read_miss_waits_for_ownership_when_the_fill_is_shared() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.read(1, 0x1000);
    bench.write(2, 0x1008);
    let fetch = bench.downstream_requests();

    bench.fill_with(fetch[0].0, 0x1000, MesiState::Shared);
    let events = bench.drain();
    let answered: Vec<ReqId> =
        responses_to(&events, PIPELINE).into_iter().map(|(id, _)| id).collect();
    let upgrades: Vec<MemOp> = events
        .iter()
        .filter(|e| e.target == DOWNSTREAM)
        .filter_map(|e| match &e.packet {
            Packet::MemReq { op, .. } => Some(op.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(answered, vec![ReqId::new(1)], "only the read is served by a shared fill");
    assert_eq!(bench.state_of(0x1000), Some(MesiState::Shared));
    assert!(matches!(upgrades.as_slice(), [MemOp::ReadOwn]), "ownership is requested next");
}

#[test]
fn a_write_joining_a_read_miss_is_served_once_ownership_arrives() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.read(1, 0x1000);
    bench.write(2, 0x1008);
    let fetch = bench.downstream_requests();
    bench.fill_with(fetch[0].0, 0x1000, MesiState::Shared);
    let upgrade = bench.downstream_requests();

    bench.fill_with(upgrade[0].0, 0x1000, MesiState::Modified);
    let events = bench.drain();

    assert_eq!(responses_to(&events, PIPELINE).len(), 1);
    assert_eq!(granted_states(&events, PIPELINE), vec![MesiState::Modified]);
    assert_eq!(bench.state_of(0x1000), Some(MesiState::Modified));
}

#[test]
fn a_write_joining_a_read_miss_is_served_by_an_exclusive_fill() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.read(1, 0x1000);
    bench.write(2, 0x1008);
    let fetch = bench.downstream_requests();

    bench.fill_with(fetch[0].0, 0x1000, MesiState::Exclusive);
    let events = bench.drain();

    assert!(events.iter().all(|e| e.target != DOWNSTREAM), "no second fetch");
    assert_eq!(responses_to(&events, PIPELINE).len(), 2);
    assert_eq!(bench.state_of(0x1000), Some(MesiState::Modified));
}

#[test]
fn an_invalidating_probe_writes_a_dirty_line_back_before_answering() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(
        1,
        0x1000,
        MemOp::Write { data: WriteData::Small(1), origin: WriteOrigin::Hart(HartId::new(0)) },
    );

    let txn = ReqId::new(77);
    bench.deliver(
        Packet::Probe {
            line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64),
            kind: ProbeKind::Invalidate,
            txn,
        },
        DOWNSTREAM,
    );
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
    assert!(
        events.iter().any(|e| matches!(e.packet, Packet::ProbeResp { txn: t, .. } if t == txn))
    );
    assert!(!bench.cache.contains(0x1000));
    assert_eq!(bench.stat("test.probes"), 1);
}

#[test]
fn a_downgrade_probe_leaves_a_shared_copy() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.install(1, 0x1000, MemOp::Read);
    bench.deliver(
        Packet::Probe {
            line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64),
            kind: ProbeKind::Downgrade,
            txn: ReqId::new(1),
        },
        DOWNSTREAM,
    );
    let events = bench.drain();
    assert!(events.iter().any(|e| matches!(e.packet, Packet::ProbeResp { dirty: false, .. })));
    assert!(
        events.iter().all(|e| !matches!(e.packet, Packet::MemReq { .. })),
        "clean line: no writeback"
    );
    assert_eq!(bench.state_of(0x1000), Some(MesiState::Shared));
}

#[test]
fn a_probe_that_hits_an_in_flight_fetch_leaves_the_fill_alone() {
    let mut bench = Bench::new(cache_with(&test_config()));
    bench.read(1, 0x1000);
    let fetch = bench.downstream_requests();
    bench.deliver(
        Packet::Probe {
            line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64),
            kind: ProbeKind::Invalidate,
            txn: ReqId::new(5),
        },
        DOWNSTREAM,
    );
    let events = bench.drain();
    assert!(
        events
            .iter()
            .any(|e| matches!(e.packet, Packet::ProbeResp { had_copy: false, dirty: false, .. })),
        "answered at once: the line is not here yet"
    );

    bench.fill(fetch[0].0, 0x1000);
    let events = bench.drain();
    assert_eq!(responses_to(&events, PIPELINE).len(), 1, "the waiting load still gets its data");
    assert!(
        bench.cache.contains(0x1000),
        "the fill was ordered after the probe, so the line stays"
    );
}

#[test]
fn a_probe_is_forwarded_upstream_and_answered_once_every_copy_replied() {
    let third = ComponentId::Cache(CacheId::new(3));
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    cache.add_upstream(third);
    let mut bench = Bench::new(cache);
    bench.install_from(UPSTREAM, 1, 0x1000, MemOp::Read);
    bench.request_from(third, 2, 0x1000, MemOp::Read);
    let _ = bench.drain();

    bench.deliver(
        Packet::Probe {
            line_addr: LineAddr::from_phys(PhysAddr::new(0x1000), 64),
            kind: ProbeKind::Invalidate,
            txn: ReqId::new(9),
        },
        DOWNSTREAM,
    );
    let events = bench.drain();
    let forwarded: Vec<ReqId> = events
        .iter()
        .filter(|e| e.target == UPSTREAM || e.target == third)
        .filter_map(|e| match e.packet {
            Packet::Probe { txn, .. } => Some(txn),
            _ => None,
        })
        .collect();
    assert_eq!(forwarded.len(), 2);
    assert!(
        events.iter().all(|e| !matches!(e.packet, Packet::ProbeResp { .. })),
        "not answered yet"
    );

    bench.deliver(Packet::ProbeResp { txn: forwarded[0], had_copy: true, dirty: false }, UPSTREAM);
    assert!(bench.drain().is_empty());
    bench.deliver(Packet::ProbeResp { txn: forwarded[1], had_copy: true, dirty: true }, third);
    let events = bench.drain();
    assert!(events.iter().any(|e| e.target == DOWNSTREAM
        && matches!(e.packet, Packet::ProbeResp { txn, dirty: true, .. } if txn == ReqId::new(9))));
    assert!(!bench.cache.contains(0x1000));
}

#[test]
fn a_probe_goes_only_to_the_caches_above_that_were_given_the_line() {
    let third = ComponentId::Cache(CacheId::new(3));
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    cache.add_upstream(third);
    let mut bench = Bench::new(cache);
    bench.install_from(UPSTREAM, 1, 0x1000, MemOp::Read);

    bench.probe_from_below(0x1000, 9);

    let forwarded = bench.forwarded_probes();
    assert_eq!(forwarded.len(), 1, "only the cache given the line is probed");
    assert_eq!(forwarded[0].0, UPSTREAM);
}

#[test]
fn a_probe_for_a_line_no_cache_above_holds_is_answered_at_once() {
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    let mut bench = Bench::new(cache);
    bench.install(1, 0x1000, MemOp::Read);

    bench.probe_from_below(0x1000, 9);

    let events = bench.drain();
    assert!(events.iter().all(|e| !matches!(e.packet, Packet::Probe { .. })), "nothing probed");
    assert!(events.iter().any(|e| e.target == DOWNSTREAM
        && matches!(e.packet, Packet::ProbeResp { txn, had_copy: true, .. } if txn == ReqId::new(9))));
    assert!(!bench.cache.contains(0x1000));
}

#[test]
fn a_cache_above_that_evicted_its_copy_is_no_longer_probed() {
    let mut cache = cache_with(&test_config());
    cache.add_upstream(UPSTREAM);
    let mut bench = Bench::new(cache);
    bench.install_from(UPSTREAM, 1, 0x1000, MemOp::Read);
    bench.request_from(UPSTREAM, 2, 0x1000, MemOp::Writeback { dirty: false });
    let _ = bench.drain();

    bench.probe_from_below(0x1000, 9);

    assert!(bench.forwarded_probes().is_empty(), "the copy above is gone");
}

mod coherent {
    //! The L2 as a core's requesting agent: misses, upgrades, writebacks
    //! and evictions become coherence messages to the home (the bench
    //! plays the fabric at `DOWNSTREAM`), snoops turn into probes of the
    //! L1s, and completions are acknowledged.

    use super::*;
    use crate::common::CoreId;
    use crate::sim::packet::coherence::{CoherenceMsg, ReqKind, SnoopKind};

    const CORE: CoreId = CoreId::new(0);

    fn coherent_l2(enabled: bool) -> Bench {
        let config = CacheConfig { enabled, ..test_config() };
        let mut cache = Cache::new(CacheId::new(0), CacheLevel::L2, &config, "test");
        cache.set_downstream(DOWNSTREAM);
        cache.add_upstream(UPSTREAM);
        cache.set_coherent(CORE);
        Bench::new(cache)
    }

    fn line(addr: u64) -> LineAddr {
        LineAddr::from_phys(PhysAddr::new(addr), 64)
    }

    fn requests(events: &[Event]) -> Vec<(ReqId, ReqKind, u64, u64)> {
        events
            .iter()
            .filter(|e| e.target == DOWNSTREAM)
            .filter_map(|e| match e.packet {
                Packet::Coh(CoherenceMsg::Req { txn, line, kind, .. }) => {
                    Some((txn, kind, line.val(), e.fire_at))
                }
                _ => None,
            })
            .collect()
    }

    fn acks(events: &[Event]) -> Vec<ReqId> {
        events
            .iter()
            .filter_map(|e| match e.packet {
                Packet::Coh(CoherenceMsg::CompAck { txn, .. }) if e.target == DOWNSTREAM => {
                    Some(txn)
                }
                _ => None,
            })
            .collect()
    }

    fn snoop_responses(events: &[Event]) -> Vec<(bool, bool)> {
        events
            .iter()
            .filter_map(|e| match e.packet {
                Packet::Coh(CoherenceMsg::SnoopResp { had_copy, dirty, .. })
                    if e.target == DOWNSTREAM =>
                {
                    Some((had_copy, dirty))
                }
                _ => None,
            })
            .collect()
    }

    fn probes(events: &[Event]) -> Vec<(ProbeKind, ReqId, u64)> {
        events
            .iter()
            .filter_map(|e| match e.packet {
                Packet::Probe { kind, txn, .. } if e.target == UPSTREAM => {
                    Some((kind, txn, e.fire_at))
                }
                _ => None,
            })
            .collect()
    }

    impl Bench {
        fn complete(&mut self, txn: ReqId, addr: u64, state: MesiState, with_data: bool) {
            let msg = if with_data {
                CoherenceMsg::CompData { txn, line: line(addr), to: CORE, state }
            } else {
                CoherenceMsg::Comp { txn, line: line(addr), to: CORE, state }
            };
            self.deliver(Packet::Coh(msg), DOWNSTREAM);
        }

        fn snoop(&mut self, addr: u64, kind: SnoopKind) -> ReqId {
            let txn = ReqId::for_fabric(0x55);
            self.deliver(
                Packet::Coh(CoherenceMsg::Snoop { txn, line: line(addr), kind, target: CORE }),
                DOWNSTREAM,
            );
            txn
        }

        fn line_request(&mut self, req_id: u64, addr: u64, op: MemOp) {
            self.deliver(
                Packet::MemReq {
                    req_id: ReqId::new(req_id),
                    paddr: PhysAddr::new(addr),
                    vaddr: None,
                    pc: None,
                    size: AccessSize::Line,
                    op,
                },
                UPSTREAM,
            );
        }

        /// Installs `addr` through a coherence request completed in `state`.
        /// Installs a line on behalf of the L1 above, which then holds a copy.
        fn install_coherent(&mut self, req_id: u64, addr: u64, op: MemOp, state: MesiState) {
            self.request_from(UPSTREAM, req_id, addr, op);
            let reqs = requests(&self.drain());
            assert_eq!(reqs.len(), 1, "one request for the line");
            self.complete(reqs[0].0, addr, state, true);
            let _ = self.drain();
        }
    }

    #[test]
    fn a_read_miss_asks_the_home_for_a_shared_copy_and_acknowledges_the_data() {
        let mut bench = coherent_l2(true);

        bench.read(1, 0x1000);
        let events = bench.drain();
        let reqs = requests(&events);
        assert_eq!(reqs.len(), 1);
        assert_eq!(
            (reqs[0].1, reqs[0].2, reqs[0].3),
            (ReqKind::ReadShared, 0x1000, bench.cycle + LATENCY)
        );
        assert!(
            events.iter().all(|e| !matches!(e.packet, Packet::MemReq { .. })),
            "no plain memory request"
        );

        bench.complete(reqs[0].0, 0x1000, MesiState::Exclusive, true);
        let events = bench.drain();
        assert_eq!(acks(&events), vec![reqs[0].0]);
        assert_eq!(responses_to(&events, PIPELINE).len(), 1);
        assert_eq!(bench.state_of(0x1000), Some(MesiState::Exclusive));
    }

    #[test]
    fn a_write_miss_asks_for_a_unique_copy() {
        let mut bench = coherent_l2(true);
        bench.write(1, 0x1000);
        let reqs = requests(&bench.drain());
        assert_eq!(reqs[0].1, ReqKind::ReadUnique);
        bench.complete(reqs[0].0, 0x1000, MesiState::Modified, true);
        let _ = bench.drain();
        assert_eq!(bench.state_of(0x1000), Some(MesiState::Modified));
    }

    #[test]
    fn a_write_to_a_shared_line_asks_for_permission_only() {
        let mut bench = coherent_l2(true);
        bench.install_coherent(1, 0x1000, MemOp::Read, MesiState::Shared);

        bench.write(2, 0x1008);
        let reqs = requests(&bench.drain());
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].1, ReqKind::CleanUnique);
        assert_eq!(bench.stat("test.coherence.upgrades"), 1);

        bench.complete(reqs[0].0, 0x1000, MesiState::Modified, false);
        let events = bench.drain();
        assert_eq!(responses_to(&events, PIPELINE).len(), 1);
        assert_eq!(bench.state_of(0x1000), Some(MesiState::Modified));
        assert_eq!(bench.stat("test.fills"), 2, "the grant fills the line in place");
    }

    #[test]
    fn a_snoop_probes_the_l1s_after_the_lookup_and_answers_with_their_verdict() {
        let mut bench = coherent_l2(true);
        bench.install_coherent(
            1,
            0x1000,
            MemOp::Write { data: WriteData::Small(1), origin: WriteOrigin::Hart(HartId::new(0)) },
            MesiState::Modified,
        );

        let txn = bench.snoop(0x1000, SnoopKind::Unique);
        let events = bench.drain();
        let sent = probes(&events);
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].0, sent[0].2), (ProbeKind::Invalidate, bench.cycle + LATENCY));
        assert!(snoop_responses(&events).is_empty(), "the L1s have not answered yet");
        assert_eq!(bench.state_of(0x1000), None, "our own copy is given up at once");
        assert!(
            events.iter().all(|e| !matches!(e.packet, Packet::Coh(CoherenceMsg::Req { .. }))),
            "no writeback: the data goes with the snoop answer"
        );

        bench.deliver(Packet::ProbeResp { txn: sent[0].1, had_copy: true, dirty: true }, UPSTREAM);
        let events = bench.drain();
        assert_eq!(snoop_responses(&events), vec![(true, true)]);
        assert!(events.iter().any(
            |e| matches!(e.packet, Packet::Coh(CoherenceMsg::SnoopResp { txn: t, .. }) if t == txn)
        ));
        assert_eq!(bench.stat("test.coherence.snoops"), 1);
        assert_eq!(bench.stat("test.coherence.invalidations"), 1);
    }

    #[test]
    fn a_shared_snoop_keeps_a_shared_copy() {
        let mut bench = coherent_l2(true);
        bench.install_coherent(1, 0x1000, MemOp::Read, MesiState::Exclusive);

        bench.snoop(0x1000, SnoopKind::Shared);
        let events = bench.drain();
        let sent = probes(&events);
        assert_eq!(sent[0].0, ProbeKind::Downgrade);
        bench
            .deliver(Packet::ProbeResp { txn: sent[0].1, had_copy: false, dirty: false }, UPSTREAM);
        let events = bench.drain();
        assert_eq!(snoop_responses(&events), vec![(true, false)]);
        assert_eq!(bench.state_of(0x1000), Some(MesiState::Shared));
        assert_eq!(bench.stat("test.coherence.downgrades"), 1);
    }

    #[test]
    fn a_snoop_for_an_absent_line_says_so_without_touching_the_fetch() {
        let mut bench = coherent_l2(true);
        bench.read(1, 0x1000);
        let reqs = requests(&bench.drain());

        bench.snoop(0x1000, SnoopKind::Unique);
        let events = bench.drain();
        let sent = probes(&events);
        bench
            .deliver(Packet::ProbeResp { txn: sent[0].1, had_copy: false, dirty: false }, UPSTREAM);
        assert_eq!(snoop_responses(&bench.drain()), vec![(false, false)]);

        bench.complete(reqs[0].0, 0x1000, MesiState::Exclusive, true);
        let _ = bench.drain();
        assert_eq!(
            bench.state_of(0x1000),
            Some(MesiState::Exclusive),
            "the fill was ordered after the snoop"
        );
    }

    #[test]
    fn a_permission_grant_for_a_line_a_snoop_took_is_reissued_as_a_data_request() {
        let mut bench = coherent_l2(true);
        bench.install_coherent(1, 0x1000, MemOp::Read, MesiState::Shared);
        bench.write(2, 0x1000);
        let upgrade = requests(&bench.drain());
        assert_eq!(upgrade[0].1, ReqKind::CleanUnique);

        bench.snoop(0x1000, SnoopKind::Unique);
        let sent = probes(&bench.drain());
        bench
            .deliver(Packet::ProbeResp { txn: sent[0].1, had_copy: false, dirty: false }, UPSTREAM);
        let _ = bench.drain();
        assert_eq!(bench.state_of(0x1000), None);

        bench.complete(upgrade[0].0, 0x1000, MesiState::Modified, false);
        let events = bench.drain();
        assert_eq!(acks(&events), vec![upgrade[0].0], "the useless grant is still acknowledged");
        let retry = requests(&events);
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].1, ReqKind::ReadUnique);
        assert_ne!(retry[0].0, upgrade[0].0, "a fresh correlator");
        assert!(responses_to(&events, PIPELINE).is_empty(), "the writer still waits");
        assert_eq!(bench.stat("test.coherence.upgrade_retries"), 1);

        bench.complete(retry[0].0, 0x1000, MesiState::Modified, true);
        let events = bench.drain();
        assert_eq!(responses_to(&events, PIPELINE).len(), 1);
        assert_eq!(bench.state_of(0x1000), Some(MesiState::Modified));
    }

    #[test]
    fn a_dirty_victim_is_written_back_and_a_clean_one_reported() {
        let mut bench = coherent_l2(true);
        // Set 0 has two ways: 0x1000, 0x1080 and 0x1100 all map to it.
        bench.install_coherent(
            1,
            0x1000,
            MemOp::Write { data: WriteData::Small(1), origin: WriteOrigin::Hart(HartId::new(0)) },
            MesiState::Modified,
        );
        bench.install_coherent(2, 0x1080, MemOp::Read, MesiState::Exclusive);

        bench.read(3, 0x1100);
        let fetch = requests(&bench.drain());
        assert_eq!(fetch.len(), 1, "the victim is chosen when the line arrives");
        bench.complete(fetch[0].0, 0x1100, MesiState::Exclusive, true);
        let reqs = requests(&bench.drain());
        let writeback = reqs
            .iter()
            .find(|r| matches!(r.1, ReqKind::WriteBack { dirty: true }))
            .expect("dirty victim written back");
        assert_eq!(writeback.2, 0x1000);
        assert!(bench.cache.writebacks().holds(line(0x1000)));
        bench.complete(writeback.0, 0x1000, MesiState::Invalid, false);
        let _ = bench.drain();
        assert!(!bench.cache.writebacks().holds(line(0x1000)), "the ack retires the writeback");

        bench.read(4, 0x1180);
        let fetch = requests(&bench.drain());
        bench.complete(fetch[0].0, 0x1180, MesiState::Exclusive, true);
        let reqs = requests(&bench.drain());
        let evict = reqs.iter().find(|r| r.1 == ReqKind::Evict).expect("clean victim reported");
        assert_eq!(evict.2, 0x1080);
    }

    #[test]
    fn a_disabled_l2_still_requests_and_probes_for_its_l1s() {
        let mut bench = coherent_l2(false);

        bench.line_request(1, 0x1000, MemOp::ReadOwn);
        let reqs = requests(&bench.drain());
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].1, ReqKind::ReadUnique);
        bench.complete(reqs[0].0, 0x1000, MesiState::Modified, true);
        let events = bench.drain();
        assert_eq!(acks(&events).len(), 1);
        let answered: Vec<_> = events
            .iter()
            .filter_map(|e| match e.packet {
                Packet::MemResp { req_id, state, .. } if e.target == UPSTREAM => {
                    Some((req_id, state))
                }
                _ => None,
            })
            .collect();
        assert_eq!(answered, vec![(ReqId::new(1), MesiState::Modified)]);
        assert!(!bench.cache.contains(0x1000), "nothing is kept here");

        bench.snoop(0x1000, SnoopKind::Unique);
        let sent = probes(&bench.drain());
        assert_eq!(sent.len(), 1, "the L1 is asked");
        bench.deliver(Packet::ProbeResp { txn: sent[0].1, had_copy: true, dirty: true }, UPSTREAM);
        assert_eq!(snoop_responses(&bench.drain()), vec![(true, true)]);

        bench.line_request(2, 0x1000, MemOp::Writeback { dirty: true });
        let reqs = requests(&bench.drain());
        assert_eq!(
            reqs.iter().map(|r| r.1).collect::<Vec<_>>(),
            vec![ReqKind::WriteBack { dirty: true }]
        );
        bench.line_request(3, 0x1040, MemOp::Writeback { dirty: false });
        let reqs = requests(&bench.drain());
        assert_eq!(reqs.iter().map(|r| r.1).collect::<Vec<_>>(), vec![ReqKind::Evict]);
    }

    #[test]
    fn a_sub_line_access_through_a_disabled_l2_stays_a_memory_request() {
        let mut bench = coherent_l2(false);
        bench.read(1, 0x80000400);
        let events = bench.drain();
        assert!(requests(&events).is_empty());
        assert!(
            events
                .iter()
                .any(|e| e.target == DOWNSTREAM && matches!(e.packet, Packet::MemReq { .. }))
        );
    }
}
