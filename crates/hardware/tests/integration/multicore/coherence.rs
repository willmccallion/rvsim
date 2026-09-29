//! Several harts with private caches: every program must still be exact
//! and the coherence invariants must hold at every audit, on every home
//! agent and interconnect.

use crate::common::builder::instruction::{ECALL, FENCE_IORW, InstructionBuilder};
use crate::common::multihart::{DATA_BASE, MultiHart};
use crate::integration::multicore::{amo_counter, spinlock};
use rvsim_core::coherence::audit::audit;
use rvsim_core::common::{LineAddr, PhysAddr};
use rvsim_core::config::{Config, HomeAgentConfig, InterconnectConfig};
use rvsim_core::core::pipeline::engine::BackendType;
use rvsim_core::core::units::cache::Cache;
use rvsim_core::isa::encoding::rv64i::{funct3 as i_f3, opcodes as i_op};
use rvsim_core::isa::encoding::zicboz::{CBO_CLEAN_IMM, CBO_FLUSH_IMM};
use rvsim_core::sim::packet::MesiState;

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const T3: u32 = 28;
const T4: u32 = 29;
const T5: u32 = 30;
const T6: u32 = 31;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A7: u32 = 17;
const MHARTID: u32 = 0xF14;
const SYS_EXIT: i32 = 93;
const AUDIT_EVERY: u64 = 32;

fn cached(harts: usize, backend: BackendType) -> Config {
    let mut config = Config::default();
    config.system.hart_count = harts;
    config.system.console = rvsim_core::config::Console::Quiet;
    config.pipeline.backend = backend;
    for cache in [&mut config.cache.l1_i, &mut config.cache.l1_d] {
        cache.enabled = true;
        cache.size_bytes = 4096;
        cache.ways = 2;
        cache.latency = 1;
    }
    config.cache.l2.enabled = true;
    config.cache.l2.size_bytes = 16384;
    config.cache.l2.ways = 4;
    config.cache.l2.latency = 4;
    config
}

/// Every hart bumps its own word of one shared line with plain loads and
/// stores; the line ping-pongs between owners and the total must be exact.
fn shared_line_stores(harts: i32, iterations: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let idle = 25 + 2 * harts;
    let mut code = vec![
        i().addi(T0, 0, 31).build(),
        i().addi(T2, 0, 1).build(),
        i().sll(T2, T2, T0).build(),
        i().addi(T2, T2, 0x400).build(),
        i().csrrs(T5, MHARTID, 0).build(),
        i().addi(T6, 0, 3).build(),
        i().sll(T5, T5, T6).build(),
        i().add(T4, T2, T5).build(),
        i().addi(T1, 0, iterations).build(),
        i().ld(T6, T4, 0).build(),
        i().addi(T6, T6, 1).build(),
        i().sd(T4, T6, 0).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -16).build(),
        i().addi(T3, T2, 0x80).build(),
        i().addi(A1, 0, 1).build(),
        i().amoadd_d(0, T3, A1).build(),
        i().csrrs(A1, MHARTID, 0).build(),
        i().bne(A1, 0, (idle - 18) * 4).build(),
        i().ld(A2, T3, 0).build(),
        i().addi(A3, 0, harts).build(),
        i().bne(A2, A3, -8).build(),
        i().addi(A0, 0, 0).build(),
    ];
    for hart in 0..harts {
        code.push(i().ld(A1, T2, hart * 8).build());
        code.push(i().add(A0, A0, A1).build());
    }
    code.push(i().addi(A7, 0, SYS_EXIT).build());
    code.push(ECALL);
    code.push(i().jal(0, 0).build());
    assert_eq!(code.len() as i32, idle + 1);
    code
}

/// Hart 0 fills a payload line then raises a flag on another; every other
/// hart waits for the flag, sums the payload and adds it to a total.
fn producer_consumer(harts: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut code = vec![
        i().addi(T0, 0, 31).build(),
        i().addi(T2, 0, 1).build(),
        i().sll(T2, T2, T0).build(),
        i().addi(T2, T2, 0x400).build(),
        i().csrrs(T5, MHARTID, 0).build(),
        i().bne(T5, 0, (26 - 5) * 4).build(),
    ];
    for k in 0..8 {
        code.push(i().addi(T6, 0, k + 1).build());
        code.push(i().sd(T2, T6, 0x40 + 8 * k).build());
    }
    code.push(FENCE_IORW);
    code.push(i().addi(T6, 0, 1).build());
    code.push(i().sd(T2, T6, 0).build());
    code.push(i().jal(0, (48 - 25) * 4).build());
    assert_eq!(code.len(), 26);
    code.push(i().ld(T6, T2, 0).build());
    code.push(i().beq(T6, 0, -4).build());
    code.push(FENCE_IORW);
    code.push(i().addi(A0, 0, 0).build());
    for k in 0..8 {
        code.push(i().ld(T6, T2, 0x40 + 8 * k).build());
        code.push(i().add(A0, A0, T6).build());
    }
    code.push(i().addi(T3, T2, 0x80).build());
    code.push(i().amoadd_d(0, T3, A0).build());
    assert_eq!(code.len(), 48);
    code.extend([
        i().addi(T3, T2, 0x88).build(),
        i().addi(T6, 0, 1).build(),
        i().amoadd_d(0, T3, T6).build(),
        i().bne(T5, 0, (59 - 51) * 4).build(),
        i().ld(A2, T3, 0).build(),
        i().addi(A3, 0, harts).build(),
        i().bne(A2, A3, -8).build(),
        i().addi(T3, T2, 0x80).build(),
        i().ld(A0, T3, 0).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
        i().jal(0, 0).build(),
    ]);
    assert_eq!(code.len(), 60);
    code
}

/// Runs to exit, auditing the coherence invariants along the way.
fn run_audited(system: &mut MultiHart, max_cycles: u64) -> Option<u64> {
    for cycle in 0..max_cycles {
        system.sim.tick().expect("tick");
        if cycle % AUDIT_EVERY == 0 {
            let violations = audit(&system.sim.state);
            assert!(violations.is_empty(), "cycle {cycle}: {violations:?}");
        }
        if let Some(code) = system.sim.state.check_exit() {
            let violations = audit(&system.sim.state);
            assert!(violations.is_empty(), "at exit: {violations:?}");
            return Some(code);
        }
    }
    None
}

fn fabric_stat(system: &MultiHart, path: &str) -> u64 {
    system.sim.state.shared.stats.get(path).unwrap_or(0.0) as u64
}

fn check_all_programs(config: &Config, label: &str) {
    let harts = config.system.hart_count;
    let iterations = 60;

    let mut system =
        MultiHart::with_config(config, &amo_counter::program(harts as i32, iterations));
    let exit = run_audited(&mut system, 6_000_000);
    assert_eq!(exit, Some(harts as u64 * iterations as u64), "{label}: amo counter");

    let mut system = MultiHart::with_config(config, &spinlock::program(harts as i32, iterations));
    let exit = run_audited(&mut system, 6_000_000);
    assert_eq!(exit, Some(harts as u64 * iterations as u64), "{label}: spinlock");

    let mut system = MultiHart::with_config(config, &shared_line_stores(harts as i32, iterations));
    let exit = run_audited(&mut system, 6_000_000);
    assert_eq!(exit, Some(harts as u64 * iterations as u64), "{label}: shared line stores");
    assert!(
        fabric_stat(&system, "coherence.ha.snoops_sent") > 0,
        "{label}: the shared line was snooped"
    );

    let mut system = MultiHart::with_config(config, &producer_consumer(harts as i32));
    let exit = run_audited(&mut system, 6_000_000);
    assert_eq!(exit, Some((harts as u64 - 1) * 36), "{label}: producer/consumer");
    assert!(
        fabric_stat(&system, "coherence.ha.c2c_transfers") > 0,
        "{label}: the payload came from the producer's cache"
    );
}

#[test]
fn two_cached_inorder_harts_stay_coherent() {
    check_all_programs(&cached(2, BackendType::InOrder), "2 inorder");
}

#[test]
fn two_cached_o3_harts_stay_coherent() {
    check_all_programs(&cached(2, BackendType::OutOfOrder), "2 o3");
}

#[test]
fn four_cached_o3_harts_stay_coherent() {
    check_all_programs(&cached(4, BackendType::OutOfOrder), "4 o3");
}

#[test]
fn a_broadcast_home_keeps_four_harts_coherent() {
    let mut config = cached(4, BackendType::InOrder);
    config.coherence.home_agent = HomeAgentConfig::Broadcast;
    check_all_programs(&config, "broadcast");
}

#[test]
fn a_tiny_snoop_filter_recalls_lines_and_stays_exact() {
    let mut config = cached(4, BackendType::InOrder);
    config.coherence.home_agent = HomeAgentConfig::SnoopFilter { capacity_factor: 0.001, ways: 2 };
    check_all_programs(&config, "tiny filter");
    let mut system = MultiHart::with_config(&config, &shared_line_stores(4, 60));
    run_audited(&mut system, 6_000_000).expect("exit");
    assert!(fabric_stat(&system, "coherence.ha.recalls") > 0, "the filter had to recall lines");
}

#[test]
fn every_interconnect_keeps_four_harts_coherent() {
    let fabrics = [
        ("ring", InterconnectConfig::Ring { hop_latency: 2, bytes_per_cycle: 32 }),
        ("mesh", InterconnectConfig::Mesh { hop_latency: 2, bytes_per_cycle: 32 }),
        ("torus", InterconnectConfig::Torus { hop_latency: 2, bytes_per_cycle: 32 }),
        ("hypercube", InterconnectConfig::Hypercube { hop_latency: 2, bytes_per_cycle: 32 }),
    ];
    for (label, interconnect) in fabrics {
        let mut config = cached(4, BackendType::InOrder);
        config.coherence.interconnect = interconnect;
        check_all_programs(&config, label);
    }
}

#[test]
fn l1_only_cores_stay_coherent_through_their_agent() {
    let mut config = cached(4, BackendType::OutOfOrder);
    config.cache.l2.enabled = false;
    check_all_programs(&config, "no l2");
}

#[test]
fn a_shared_llc_serves_the_home_agent() {
    let mut config = cached(4, BackendType::OutOfOrder);
    config.cache.l3.enabled = true;
    config.cache.l3.size_bytes = 65536;
    config.cache.l3.ways = 8;
    config.cache.l3.latency = 12;
    check_all_programs(&config, "with llc");
}

#[test]
fn cores_without_caches_take_no_part_in_coherence() {
    let mut config = Config::default();
    config.system.hart_count = 2;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut system = MultiHart::with_config(&config, &shared_line_stores(2, 20));
    let exit = run_audited(&mut system, 2_000_000);
    assert_eq!(exit, Some(40));
    assert_eq!(fabric_stat(&system, "coherence.ha.snoops_sent"), 0);
    assert!(fabric_stat(&system, "coherence.ha.requests.non_coherent") > 0);
}

/// Hart 0 bumps a word that shares a line with the code every hart runs, so
/// its L1D holds the line Shared, and reloads it at once; the exit code is
/// the reloaded value minus the stored one.
fn reload_of_a_word_in_the_code_line() -> Vec<u32> {
    const WORD_INDEX: i32 = 14;
    const OFFSET_FROM_T0: i32 = (WORD_INDEX - 2) * 4;
    let i = InstructionBuilder::new;
    let mut code = vec![
        i().csrrs(T5, MHARTID, 0).build(),
        i().bne(T5, 0, (10 - 1) * 4).build(),
        i().auipc(T0, 0).build(),
        i().lw(A1, T0, OFFSET_FROM_T0).build(),
        i().addi(A1, A1, 1).build(),
        i().sw(T0, A1, OFFSET_FROM_T0).build(),
        i().lw(A2, T0, OFFSET_FROM_T0).build(),
        i().sub(A0, A2, A1).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
        i().jal(0, 0).build(),
    ];
    code.resize(WORD_INDEX as usize, i().nop().build());
    code.push(0x41);
    code
}

#[test]
fn a_reload_sees_its_write_combined_store_while_the_line_is_shared() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let mut config = cached(2, backend);
        config.cache.wcb_entries = 4;
        let mut system = MultiHart::with_config(&config, &reload_of_a_word_in_the_code_line());

        let exit = system.run_until_exit(20_000);

        assert_eq!(exit, Some(0), "{backend:?}");
    }
}

/// Hart 1 writes a word of a line and raises a flag on another; hart 0
/// waits for the flag, then applies the CBO `cbo_imm` to the line and a
/// fence that waits for it, and exits with 0.
fn cbo_after_another_hart_writes(cbo_imm: i64) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let cbo = ((cbo_imm as u32 & 0xFFF) << 20) | (T2 << 15) | (i_f3::CBO << 12) | i_op::OP_MISC_MEM;
    let code = vec![
        i().addi(T0, 0, 31).build(),
        i().addi(T2, 0, 1).build(),
        i().sll(T2, T2, T0).build(),
        i().addi(T2, T2, 0x400).build(),
        i().csrrs(T5, MHARTID, 0).build(),
        i().bne(T5, 0, (13 - 5) * 4).build(),
        i().ld(T6, T2, 0x80).build(),
        i().beq(T6, 0, -4).build(),
        cbo,
        FENCE_IORW,
        i().addi(A0, 0, 0).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
        i().addi(T6, 0, 5).build(),
        i().sd(T2, T6, 0).build(),
        FENCE_IORW,
        i().addi(T6, 0, 1).build(),
        i().sd(T2, T6, 0x80).build(),
        i().jal(0, 0).build(),
    ];
    assert_eq!(code.len(), 19);
    code
}

/// The state each hart's L1D and L2 hold the written line in.
fn states_of_the_written_line(system: &MultiHart) -> Vec<(Option<MesiState>, Option<MesiState>)> {
    let line = LineAddr::from_phys(PhysAddr::new(DATA_BASE), 64);
    let state_in = |cache: &Cache| {
        cache.held_lines().into_iter().find(|(held, _)| *held == line).map(|(_, state)| state)
    };
    system
        .sim
        .state
        .cores
        .iter()
        .map(|core| (state_in(&core.units.l1_d_cache), state_in(&core.units.l2_cache)))
        .collect()
}

#[test]
fn a_flush_takes_a_line_out_of_every_harts_caches() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let config = cached(2, backend);
        let mut system =
            MultiHart::with_config(&config, &cbo_after_another_hart_writes(CBO_FLUSH_IMM));

        let exit = system.run_until_exit(50_000);

        assert_eq!(exit, Some(0), "{backend:?}");
        assert_eq!(
            states_of_the_written_line(&system),
            [(None, None), (None, None)],
            "{backend:?}"
        );
        assert_eq!(system.read_u64(DATA_BASE), 5, "{backend:?}");
    }
}

#[test]
fn a_clean_leaves_the_writer_holding_its_line_clean() {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let config = cached(2, backend);
        let mut system =
            MultiHart::with_config(&config, &cbo_after_another_hart_writes(CBO_CLEAN_IMM));

        let exit = system.run_until_exit(50_000);

        assert_eq!(exit, Some(0), "{backend:?}");
        let writer = states_of_the_written_line(&system)[1];
        assert!(
            !matches!(writer, (Some(MesiState::Modified), _) | (_, Some(MesiState::Modified))),
            "{backend:?}: {writer:?}"
        );
        assert!(writer.1.is_some(), "{backend:?}: the writer keeps its copy");
    }
}
