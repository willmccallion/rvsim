//! The home agent's and the interconnect's counts, the private caches'
//! coherence counts, and the stats every hart and core adds to the
//! system's, on two harts sharing lines.

use super::{Recorder, accounting_checks};
use crate::config::{BackendKind, Config, HomeAgentConfig, InterconnectConfig};
use crate::system::simulator::StopAt;
use crate::tests::support::builder::instruction::{FENCE_IORW, InstructionBuilder};
use crate::tests::support::multihart::{MultiHart, PROGRAM_BASE};

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T5: u32 = 30;
const T6: u32 = 31;
const MHARTID: u32 = 0xF14;
/// Hart 1 sets this word once its body is done; a line of its own.
const DONE_FLAG: i32 = 0x300;
/// Cycles run after hart 0 reaches its end for every transaction to finish.
const SETTLE: u64 = 3_000;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

/// Two harts with private L1Ds and L2s, coherent through the home agent.
fn two_cached_harts(backend: BackendKind) -> Config {
    let mut config = Config::default();
    config.system.hart_count = 2;
    config.system.console = crate::config::Console::Quiet;
    config.pipeline.backend = backend;
    for cache in [&mut config.cache.l1_i, &mut config.cache.l1_d] {
        cache.enabled = true;
        cache.size_bytes = 4096;
        cache.ways = 2;
    }
    config.cache.l2.enabled = true;
    config.cache.l2.size_bytes = 16384;
    config.cache.l2.ways = 4;
    config
}

/// A program in which `t2` holds `DATA_BASE` and each hart runs its own
/// body. Hart 1 then raises the done flag and spins; hart 0 waits for the
/// flag and spins at the returned address.
fn per_hart(hart0: &[u32], hart1: &[u32]) -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    let mut code = vec![
        i().addi(T0, 0, 31).build(),
        i().addi(T2, 0, 1).build(),
        i().sll(T2, T2, T0).build(),
        i().addi(T2, T2, 0x400).build(),
        i().csrrs(T5, MHARTID, 0).build(),
    ];
    let hart0_tail = 4;
    let hart1_at = code.len() + 1 + hart0.len() + hart0_tail;
    code.push(i().bne(T5, 0, ((hart1_at - code.len()) * 4) as i32).build());
    code.extend_from_slice(hart0);
    code.extend([FENCE_IORW, i().ld(T6, T2, DONE_FLAG).build(), i().beq(T6, 0, -4).build()]);
    let end = PROGRAM_BASE + 4 * code.len() as u64;
    code.push(i().jal(0, 0).build());
    assert_eq!(code.len(), hart1_at);
    code.extend_from_slice(hart1);
    code.extend([
        FENCE_IORW,
        i().addi(T6, 0, 1).build(),
        i().sd(T2, T6, DONE_FLAG).build(),
        i().jal(0, 0).build(),
    ]);
    (code, end)
}

/// Runs until hart 0 reaches `end`, then until every transaction is done.
fn run_settled(config: &Config, (program, end): (Vec<u32>, u64), context: &str) -> MultiHart {
    let mut system = MultiHart::with_config(config, &program);
    let stop = StopAt { pcs: vec![end], cycles: Some(2_000_000), ..StopAt::default() };
    let reason = system.sim.run_to(&stop).expect("the run ticks");
    assert_eq!(reason, crate::system::simulator::StopReason::Pc { hart: 0 }, "{context}");
    for _ in 0..SETTLE {
        system.sim.tick().expect("tick");
    }
    system
}

/// `rounds` times: store to this hart's word of the shared line, then load
/// the other hart's word.
fn ping_pong(rounds: i32, mine: i32, theirs: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T1, 0, rounds).build(),
        i().sd(T2, T1, mine).build(),
        i().ld(T3, T2, theirs).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -12).build(),
    ]
}

fn sum_over_cores(rec: &mut Recorder, system: &MultiHart, tail: &str) -> u64 {
    (0..2).map(|core| rec.read(&system.sim, &format!("core{core}.{tail}"))).sum()
}

fn every_snoop_and_upgrade_the_home_counts_reaches_a_cache(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let program = per_hart(&ping_pong(20, 0, 8), &ping_pong(20, 8, 0));

        let system = run_settled(&two_cached_harts(backend), program, &context);

        let sent = rec.read(&system.sim, "coherence.ha.snoops_sent");
        let received = sum_over_cores(rec, &system, "cache.l2.coherence.snoops");
        assert!(sent > 0, "{context}");
        assert_eq!(received, sent, "{context}: every snoop reaches one L2");
        let lost = sum_over_cores(rec, &system, "cache.l2.coherence.invalidations")
            + sum_over_cores(rec, &system, "cache.l2.coherence.downgrades");
        assert!((1..=received).contains(&lost), "{context}: {lost} of {received} found a copy");
        let upgrades = sum_over_cores(rec, &system, "cache.l2.coherence.upgrades");
        rec.expect(&system.sim, "coherence.ha.requests.clean_unique", upgrades, &context);
        let probes = sum_over_cores(rec, &system, "cache.l1d.probes");
        assert!((1..=received).contains(&probes), "{context}: {probes} probes for {received}");
        for cache in ["l1d", "l1i"] {
            let snooped = sum_over_cores(rec, &system, &format!("cache.{cache}.coherence.snoops"));
            assert_eq!(snooped, 0, "{context}: the home snoops only the L2s");
        }
        assert!(rec.read(&system.sim, "coherence.ha.c2c_transfers") > 0, "{context}");
    }
}

const T4: u32 = 29;

/// `lines` lines from 4 KiB above the data, each stored to when `store`,
/// otherwise loaded.
fn sweep(lines: i32, store: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let access = if store { i().sd(T4, T1, 0).build() } else { i().ld(T3, T4, 0).build() };
    vec![
        i().lui(T4, 1).build(),
        i().add(T4, T2, T4).build(),
        i().addi(T1, 0, lines).build(),
        access,
        i().addi(T4, T4, 64).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -12).build(),
    ]
}

/// L2 fetches that went to the home: misses that did not join one, the
/// L2's own prefetches, and permission grants re-sent for data.
fn l2_fetches(rec: &mut Recorder, system: &MultiHart) -> u64 {
    sum_over_cores(rec, system, "cache.l2.misses")
        - sum_over_cores(rec, system, "cache.l2.mshr_hits")
        + sum_over_cores(rec, system, "cache.l2.prefetches.issued")
        + sum_over_cores(rec, system, "cache.l2.coherence.upgrade_retries")
}

fn a_snoop_of_a_cache_holding_nothing_takes_nothing(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut broadcast = two_cached_harts(backend);
        broadcast.coherence.home_agent = HomeAgentConfig::Broadcast;
        let program = per_hart(&sweep(64, true), &[]);

        let system = run_settled(&broadcast, program, &context);

        // Hart 0's stores to the 64 lines snoop hart 1 for each; hart 1
        // holds none of them, only the code and its done flag.
        let snooped = rec.read(&system.sim, "core1.cache.l2.coherence.snoops");
        assert!(snooped >= 64, "{context}: the home snoops every core");
        rec.expect(&system.sim, "core1.cache.l2.coherence.invalidations", 0, &context);
    }
}

fn the_home_starts_one_request_per_l2_fetch_eviction_and_writeback(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut hart1 = ping_pong(20, 8, 0);
        hart1.extend(sweep(320, true));
        hart1.extend(sweep(320, false));
        let program = per_hart(&ping_pong(20, 0, 8), &hart1);

        let system = run_settled(&two_cached_harts(backend), program, &context);

        let home = |rec: &mut Recorder, kind: &str| {
            rec.read(&system.sim, &format!("coherence.ha.requests.{kind}"))
        };
        let reads = home(rec, "read_shared") + home(rec, "read_unique") + home(rec, "clean_unique");
        assert_eq!(l2_fetches(rec, &system), reads, "{context}: each L2 fetch is a request");
        let (evicts, writebacks) = (home(rec, "evicts"), home(rec, "writebacks"));
        assert!(evicts > 0 && writebacks > 0, "{context}: clean and dirty lines left the L2s");
        let evicted = sum_over_cores(rec, &system, "cache.l2.evictions");
        assert_eq!(evicted, evicts + writebacks, "{context}: each eviction tells the home");
        assert!(home(rec, "stale_writebacks") <= writebacks, "{context}");
        let started = rec.histogram(&system.sim, "coherence.ha.txn_latency").count();
        assert_eq!(started, reads + writebacks, "{context}: a transaction for each");
        // A writeback reads the tracking state without looking a line up
        // for snoops; reads, maintenance and recall victims do.
        let looked_up = rec.read(&system.sim, "coherence.ha.filter.hits")
            + rec.read(&system.sim, "coherence.ha.filter.misses");
        let looked_for =
            reads + home(rec, "maintenance") + rec.read(&system.sim, "coherence.ha.recalls");
        assert_eq!(looked_up, looked_for, "{context}: one tracking lookup each");
    }
}

fn the_interconnect_moves_a_header_or_a_header_and_a_line(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut hart1 = ping_pong(20, 8, 0);
        hart1.extend(sweep(320, true));
        let program = per_hart(&ping_pong(20, 0, 8), &hart1);
        let per_cycle = 16;
        let mut config = two_cached_harts(backend);
        config.coherence.interconnect =
            InterconnectConfig::Crossbar { hop_latency: 1, bytes_per_cycle: per_cycle };

        let system = run_settled(&config, program, &context);

        let messages = rec.read(&system.sim, "coherence.interconnect.messages");
        let bytes = rec.read(&system.sim, "coherence.interconnect.bytes");
        let (header, line) = (8, config.cache.l2.line_bytes as u64);
        let with_line = (bytes - header * messages) / line;
        assert_eq!(bytes, header * messages + line * with_line, "{context}: whole messages");
        let reads = rec.read(&system.sim, "coherence.ha.requests.read_shared")
            + rec.read(&system.sim, "coherence.ha.requests.read_unique");
        let writebacks = rec.read(&system.sim, "coherence.ha.requests.writebacks");
        assert!(with_line >= reads + writebacks, "{context}: a line for each fill and writeback");
        let per_cycle = per_cycle as u64;
        let extra_cycles = (header + line).div_ceil(per_cycle) - header.div_ceil(per_cycle);
        let busy = rec.read(&system.sim, "coherence.interconnect.busy_cycles");
        assert_eq!(busy, with_line * extra_cycles, "{context}: a line holds its port longer");
        let blocked = rec.read(&system.sim, "coherence.interconnect.blocked_cycles");
        assert!(blocked > 0, "{context}: messages queued behind lines");
    }
}

fn the_home_counts_maintenance_recalls_and_waits(rec: &mut Recorder) {
    use crate::isa::encoding::rv64i::{funct3 as i_f3, opcodes as i_op};
    use crate::isa::encoding::zicboz::CBO_FLUSH_IMM;
    let i = InstructionBuilder::new;
    let flush =
        ((CBO_FLUSH_IMM as u32 & 0xFFF) << 20) | (T2 << 15) | (i_f3::CBO << 12) | i_op::OP_MISC_MEM;
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let hart0 = [i().addi(T1, 0, 5).build(), i().sd(T2, T1, 0).build(), flush];
        let program = per_hart(&hart0, &ping_pong(20, 8, 0));
        let mut tiny = two_cached_harts(backend);
        tiny.coherence.home_agent =
            HomeAgentConfig::SnoopFilter { capacity_factor: 0.001, ways: 2 };
        let mut one_txn = two_cached_harts(backend);
        one_txn.coherence.txn_entries = 1;

        let plain = run_settled(&two_cached_harts(backend), program.clone(), &context);
        let tiny = run_settled(&tiny, program.clone(), &context);
        let one_txn = run_settled(&one_txn, program, &context);

        rec.expect(&plain.sim, "coherence.ha.requests.maintenance", 1, &context);
        rec.expect(&plain.sim, "coherence.ha.recalls", 0, &context);
        rec.expect(&plain.sim, "coherence.ha.txn_full_stalls", 0, &context);
        assert!(rec.read(&tiny.sim, "coherence.ha.recalls") > 0, "{context}: a tiny filter");
        assert!(rec.read(&one_txn.sim, "coherence.ha.txn_full_stalls") > 0, "{context}");
        assert!(rec.read(&one_txn.sim, "coherence.ha.serialised") > 0, "{context}");
    }
}

fn cores_without_caches_go_to_memory_without_snooping(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut cacheless = two_cached_harts(backend);
        for cache in [&mut cacheless.cache.l1_i, &mut cacheless.cache.l1_d, &mut cacheless.cache.l2]
        {
            cache.enabled = false;
        }
        let program = || per_hart(&ping_pong(5, 0, 8), &ping_pong(5, 8, 0));

        let cached = run_settled(&two_cached_harts(backend), program(), &context);
        let uncached = run_settled(&cacheless, program(), &context);

        rec.expect(&cached.sim, "coherence.ha.requests.non_coherent", 0, &context);
        assert!(rec.read(&uncached.sim, "coherence.ha.requests.non_coherent") > 0, "{context}");
        rec.expect(&uncached.sim, "coherence.ha.snoops_sent", 0, &context);
    }
}

fn only_the_l2s_take_part_in_coherence(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut config = two_cached_harts(backend);
        config.cache.l3.enabled = true;
        config.cache.l3.size_bytes = 65536;
        config.cache.l3.ways = 8;
        let program = per_hart(&ping_pong(20, 0, 8), &ping_pong(20, 8, 0));

        let system = run_settled(&config, program, &context);

        for core in 0..2 {
            for cache in ["l1i", "l1d"] {
                for stat in ["snoops", "invalidations", "downgrades", "upgrades", "upgrade_retries"]
                {
                    let path = format!("core{core}.cache.{cache}.coherence.{stat}");
                    rec.expect(&system.sim, &path, 0, &context);
                }
            }
        }
        for stat in ["snoops", "invalidations", "downgrades", "upgrades", "upgrade_retries"] {
            rec.expect(&system.sim, &format!("llc.coherence.{stat}"), 0, &context);
        }
        rec.expect(&system.sim, "llc.probes", 0, &context);
        for core in 0..2 {
            // The home snoops the L2s; only the levels above an L2 are probed.
            rec.expect(&system.sim, &format!("core{core}.cache.l2.probes"), 0, &context);
        }
        let fetched = sum_over_cores(rec, &system, "cache.l1i.probes");
        assert!(fetched <= rec.read(&system.sim, "coherence.ha.snoops_sent"), "{context}");
    }
}

/// `mtime` as the CLINT's base plus this.
const MTIME: u64 = 0xBFF8;
/// Machine time both harts wait for before storing, long after both loads.
const STORE_AT: i32 = 400;

/// Loads the shared word, waits for machine time `STORE_AT`, then stores to
/// it: both harts hold the line Shared and ask to upgrade within a few
/// cycles of each other. The wait reads the CLINT, which coherence does
/// not slow down for one hart more than the other.
fn read_meet_write(clint_base: u64) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mtime = clint_base + MTIME;
    vec![
        i().ld(T3, T2, 0).build(),
        i().lui(T4, ((mtime + 8) >> 12) as i32).build(),
        i().addi(T1, 0, STORE_AT).build(),
        i().ld(T0, T4, -8).build(),
        i().blt(T0, T1, -4).build(),
        i().sd(T2, T0, 0).build(),
    ]
}

fn an_upgrade_a_snoop_overtook_is_retried_for_data(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        // Hops long enough that the first upgrade's snoop is still on its
        // way when the second hart asks.
        let mut config = two_cached_harts(backend);
        config.coherence.interconnect =
            InterconnectConfig::Crossbar { hop_latency: 40, bytes_per_cycle: 16 };
        let clint = config.system.clint_base;
        let program = per_hart(&read_meet_write(clint), &read_meet_write(clint));

        let system = run_settled(&config, program, &context);

        let retries = sum_over_cores(rec, &system, "cache.l2.coherence.upgrade_retries");
        let upgrades = sum_over_cores(rec, &system, "cache.l2.coherence.upgrades");
        assert!(retries <= upgrades, "{context}: {retries} of {upgrades}");
        // The in-order harts store within a few cycles of each other, so
        // the race happens there every time.
        if backend == BackendKind::InOrder {
            assert!(retries > 0, "{context}: {retries} of {upgrades}");
        }
    }
}

/// `rounds` times: an LR/SC increment of the shared word whose LR hits:
/// a load brings the line in and the divide waits for it, so the LR reads
/// as the divide starts and then waits for it to retire.
fn lr_sc_increments_on_a_held_line(rounds: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T1, 0, rounds).build(),
        i().ld(T3, T2, 0).build(),
        i().add(T0, T3, T1).build(),
        i().div(T0, T0, T0).build(),
        i().lr_d(T3, T2).build(),
        i().addi(T3, T3, 1).build(),
        i().sc_d(T0, T2, T3).build(),
        i().bne(T0, 0, -24).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -32).build(),
    ]
}

/// `rounds` times: store to the shared word, a multiply chain apart, so
/// some store lands while the other hart's LR waits to retire.
fn paced_stores_to_the_shared_word(rounds: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T1, 0, rounds).build(),
        i().mul(T3, T1, T1).build(),
        i().mul(T3, T3, T1).build(),
        i().mul(T3, T3, T1).build(),
        i().mul(T3, T3, T1).build(),
        i().sd(T2, T3, 0).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -24).build(),
    ]
}

/// The line `racing_loads` reads and `racing_stores` writes.
const RACED: i32 = 0x80;

/// `rounds` times: two loads from one line, the older waiting on a divide
/// for its address, so the younger reads the line first.
fn racing_loads(rounds: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T1, 0, rounds).build(),
        i().div(T0, T1, T1).build(),
        i().add(T4, T2, T0).build(),
        i().ld(T3, T4, RACED - 1).build(),
        i().ld(T5, T2, RACED + 8).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -20).build(),
    ]
}

/// `rounds` times: store to the line `racing_loads` reads.
fn racing_stores(rounds: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T1, 0, rounds).build(),
        i().sd(T2, T1, RACED).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -8).build(),
    ]
}

/// The coherence replays and violations `program` produced, after checking
/// that each is a coherence flush and every coherence flush is one of them.
fn coherence_squashes(
    rec: &mut Recorder,
    backend: BackendKind,
    program: (Vec<u32>, u64),
    context: &str,
) -> (u64, u64) {
    let system = run_settled(&two_cached_harts(backend), program, context);
    let replays = sum_over_cores(rec, &system, "lsq.coherence_replays");
    let violations = sum_over_cores(rec, &system, "lsq.coherence_violations");
    let flushes = sum_over_cores(rec, &system, "pipeline.flushes.coherence");
    assert_eq!(flushes, replays + violations, "{context}: coherence flushes");
    (replays, violations)
}

fn remote_writes_replay_lrs_and_squash_loads_that_read_too_early(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        // Hart 1 writes the word throughout hart 0's LR/SC loop, so some
        // write lands while an LR waits behind its divide to retire.
        let lrs =
            per_hart(&lr_sc_increments_on_a_held_line(60), &paced_stores_to_the_shared_word(600));
        let races = per_hart(&racing_loads(200), &racing_stores(200));

        let (replays, _) = coherence_squashes(rec, backend, lrs, &format!("{context} LR/SC"));
        let (_, violations) = coherence_squashes(rec, backend, races, &format!("{context} loads"));

        assert!(replays > 0, "{context}: an LR read a line the other hart then wrote");
        if backend == BackendKind::OutOfOrder {
            assert!(violations > 0, "{context}: a load read past a remote write");
        } else {
            assert_eq!(violations, 0, "{context}: in-order loads read in order");
        }
    }
}

fn the_system_sums_what_every_hart_counts(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let program = per_hart(&ping_pong(10, 0, 8), &ping_pong(30, 8, 0));

        let system = run_settled(&two_cached_harts(backend), program, &context);

        for stat in ["retired_insts", "traps"] {
            let harts: u64 =
                (0..2).map(|hart| rec.read(&system.sim, &format!("hart{hart}.{stat}"))).sum();
            rec.expect(&system.sim, &format!("system.{stat}"), harts, &context);
        }
        let retired =
            [0, 1].map(|hart| rec.read(&system.sim, &format!("hart{hart}.retired_insts")));
        assert!(retired[1] > retired[0], "{context}: hart 1 ran more rounds");
    }
}

accounting_checks!(
    every_snoop_and_upgrade_the_home_counts_reaches_a_cache,
    a_snoop_of_a_cache_holding_nothing_takes_nothing,
    the_home_starts_one_request_per_l2_fetch_eviction_and_writeback,
    the_interconnect_moves_a_header_or_a_header_and_a_line,
    the_home_counts_maintenance_recalls_and_waits,
    cores_without_caches_go_to_memory_without_snooping,
    only_the_l2s_take_part_in_coherence,
    an_upgrade_a_snoop_overtook_is_retried_for_data,
    remote_writes_replay_lrs_and_squash_loads_that_read_too_early,
    the_system_sums_what_every_hart_counts,
);
