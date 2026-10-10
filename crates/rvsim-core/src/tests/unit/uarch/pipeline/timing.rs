//! Cycle-exact checks of the backend timing model on both backends.
//!
//! A functional unit's latency is what its dependents wait for, and a
//! resolved misprediction redirects fetch `redirect_latency` cycles later.

use crate::config::BackendKind;
use crate::config::{BranchPredictorKind, Config, CsrSquash, MemDepPredictorKind};
use crate::tests::support::builder::instruction::{FENCE_IORW, InstructionBuilder};
use crate::tests::support::harness::TestContext;

const BASE_ADDR: u64 = 0x8000_0000;
const MEM_SIZE: usize = 0x1000;
const CHAIN_LEN: u32 = 20;
const DONE_REG: usize = 2;
const DONE_VALUE: u64 = 7;

/// A single-issue core whose writeback ports never hold a result back, so
/// only the unit latency shows.
fn config(backend: BackendKind, int_mul_latency: u64) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.fu_config.int_mul_latency = int_mul_latency;
    config.pipeline.writeback_width = Some(8);
    config
}

/// Cycle at which `program` writes `DONE_VALUE` into `DONE_REG`.
fn cycles_to_finish(config: &Config, program: &[u32]) -> u64 {
    let mut tc = TestContext::new_with_config(config)
        .with_memory(MEM_SIZE, BASE_ADDR)
        .load_program(BASE_ADDR, program);
    tc.run_until(5_000, |tc| tc.get_reg(DONE_REG) == DONE_VALUE).expect("program did not finish")
}

fn done_marker() -> Vec<u32> {
    let nop = InstructionBuilder::new().nop().build();
    let mut tail =
        vec![InstructionBuilder::new().addi(DONE_REG as u32, 0, DONE_VALUE as i32).build()];
    tail.extend(std::iter::repeat_n(nop, 4));
    tail
}

/// `x1 = 1`, then `CHAIN_LEN` multiplies each reading the previous result.
fn dependent_multiply_chain() -> Vec<u32> {
    let mut program = vec![InstructionBuilder::new().addi(1, 0, 1).build()];
    program.extend((0..CHAIN_LEN).map(|_| InstructionBuilder::new().mul(1, 1, 1).build()));
    program.extend(done_marker());
    program
}

/// `x1 = 1`, then `CHAIN_LEN` multiplies that only read `x1`.
fn independent_multiplies() -> Vec<u32> {
    let mut program = vec![InstructionBuilder::new().addi(1, 0, 1).build()];
    program.extend((0..CHAIN_LEN).map(|i| InstructionBuilder::new().mul(3 + i, 1, 1).build()));
    program.extend(done_marker());
    program
}

fn assert_chain_pays_latency_per_link(backend: BackendKind) {
    let fast = cycles_to_finish(&config(backend, 3), &dependent_multiply_chain());
    let slow = cycles_to_finish(&config(backend, 9), &dependent_multiply_chain());

    assert_eq!(
        slow - fast,
        u64::from(CHAIN_LEN) * 6,
        "{backend:?}: each link should wait the unit latency"
    );
}

fn assert_independent_ops_pay_latency_once(backend: BackendKind) {
    let fast = cycles_to_finish(&config(backend, 3), &independent_multiplies());
    let slow = cycles_to_finish(&config(backend, 9), &independent_multiplies());

    assert_eq!(slow - fast, 6, "{backend:?}: a pipelined unit overlaps independent ops");
}

#[test]
fn o3_dependent_chain_pays_the_unit_latency_per_link() {
    assert_chain_pays_latency_per_link(BackendKind::OutOfOrder);
}

#[test]
fn inorder_dependent_chain_pays_the_unit_latency_per_link() {
    assert_chain_pays_latency_per_link(BackendKind::InOrder);
}

#[test]
fn o3_independent_ops_on_a_pipelined_unit_pay_the_latency_once() {
    assert_independent_ops_pay_latency_once(BackendKind::OutOfOrder);
}

#[test]
fn inorder_independent_ops_on_a_pipelined_unit_pay_the_latency_once() {
    assert_independent_ops_pay_latency_once(BackendKind::InOrder);
}

const TARGET_REG: usize = 3;
const TARGET_VALUE: u64 = 42;
const WRONG_PATH_REG: usize = 4;

fn redirect_config(backend: BackendKind, width: usize, redirect_latency: u64) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.pipeline.branch_predictor = BranchPredictorKind::Static;
    config.pipeline.redirect_latency = Some(redirect_latency);
    // The wrong path runs into the next line; with a real I-cache the
    // refetch of the target is a hit instead of a second trip to memory.
    config.cache.l1_i.enabled = true;
    config
}

/// A branch the static predictor gets wrong, two wrong-path writes to
/// `WRONG_PATH_REG`, then the target's write to `TARGET_REG`.
fn mispredicted_branch() -> Vec<u32> {
    let nop = InstructionBuilder::new().nop().build();
    let mut program = vec![
        InstructionBuilder::new().addi(1, 0, 1).build(),
        InstructionBuilder::new().beq(0, 0, 12).build(),
        InstructionBuilder::new().addi(WRONG_PATH_REG as u32, 0, 99).build(),
        InstructionBuilder::new().addi(WRONG_PATH_REG as u32, 0, 98).build(),
        InstructionBuilder::new().addi(TARGET_REG as u32, 0, TARGET_VALUE as i32).build(),
    ];
    program.extend(std::iter::repeat_n(nop, 8));
    program
}

fn cycles_to_reach_target(config: &Config) -> u64 {
    let mut tc = TestContext::new_with_config(config)
        .with_memory(MEM_SIZE, BASE_ADDR)
        .load_program(BASE_ADDR, &mispredicted_branch());
    let cycles = tc
        .run_until(5_000, |tc| tc.get_reg(TARGET_REG) == TARGET_VALUE)
        .expect("branch target never reached");
    assert_eq!(tc.get_reg(WRONG_PATH_REG), 0, "a wrong-path instruction retired");
    cycles
}

#[test]
fn inorder_redirect_lands_exactly_redirect_latency_after_the_branch_resolves() {
    let one = cycles_to_reach_target(&redirect_config(BackendKind::InOrder, 1, 1));
    let five = cycles_to_reach_target(&redirect_config(BackendKind::InOrder, 1, 5));

    assert_eq!(five - one, 4);
}

#[test]
fn o3_redirect_lands_exactly_redirect_latency_after_the_branch_resolves() {
    let one = cycles_to_reach_target(&redirect_config(BackendKind::OutOfOrder, 4, 1));
    let five = cycles_to_reach_target(&redirect_config(BackendKind::OutOfOrder, 4, 5));

    assert_eq!(five - one, 4);
}

#[test]
fn nothing_on_the_wrong_path_retires_while_the_redirect_is_pending() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let _ = cycles_to_reach_target(&redirect_config(backend, 4, 12));
    }
}

/// Runs the mispredicted branch on a four-wide O3 core squashing
/// `squash_width` ROB entries per cycle; returns the squashed entries,
/// the cycles rename was held, and the cycles to reach the target.
fn squash_recovery(squash_width: usize) -> (f64, f64, u64) {
    let mut config = redirect_config(BackendKind::OutOfOrder, 4, 2);
    config.pipeline.squash_width = squash_width;
    let mut tc = TestContext::new_with_config(&config)
        .with_memory(MEM_SIZE, BASE_ADDR)
        .load_program(BASE_ADDR, &mispredicted_branch());
    let cycles = tc
        .run_until(5_000, |tc| tc.get_reg(TARGET_REG) == TARGET_VALUE)
        .expect("branch target never reached");
    let paths = &tc.sim.state.cores[0].units.stat_paths.pipeline;
    let stats = &tc.sim.state.stats;
    assert_eq!(stats.get(paths.flushes_total), Some(1.0), "one squash");
    let squashed = stats.get(paths.flushes_squashed_insns).unwrap_or(0.0);
    (squashed, stats.get(paths.stalls_squash).unwrap_or(0.0), cycles)
}

/// Commit squashes `squash_width` ROB entries per cycle, as gem5's
/// `squashWidth`, and rename resumes the cycle after it finishes.
#[test]
fn o3_rename_waits_the_squashed_entries_over_the_squash_width_plus_a_cycle() {
    for squash_width in [1, 3, 8] {
        let (squashed, stalled, _) = squash_recovery(squash_width);
        assert!(squashed > 1.0, "the wrong path reached the ROB");
        assert_eq!(
            stalled,
            (squashed / squash_width as f64).ceil() + 1.0,
            "squash width {squash_width}: {squashed} entries"
        );
    }
}

#[test]
fn o3_a_wider_squash_recovers_sooner() {
    let (_, _, one) = squash_recovery(1);
    let (_, _, eight) = squash_recovery(8);
    assert!(eight < one, "one per cycle: {one}, eight per cycle: {eight}");
}

/// A store to one address, a load from another that the blind
/// memory-dependence predictor makes wait for the store's address, and
/// the load's dependent. `with_store = false` puts a `nop` in the store's
/// place.
fn load_behind_store(with_store: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let first = if with_store { i().sd(10, 0, 0).build() } else { i().nop().build() };
    let mut program = vec![
        i().auipc(10, 0).build(),
        i().addi(10, 10, 0x400).build(),
        first,
        i().ld(6, 10, 8).build(),
        i().addi(7, 6, 1).build(),
    ];
    program.extend(done_marker());
    program
}

/// Cycles a load waits because an older store's address is unknown.
fn store_visibility_delay(backend: BackendKind) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.mem_dep_predictor = MemDepPredictorKind::Blind;
    cycles_to_finish(&config, &load_behind_store(true))
        - cycles_to_finish(&config, &load_behind_store(false))
}

#[test]
fn inorder_load_behind_a_store_is_not_delayed_by_it() {
    assert_eq!(store_visibility_delay(BackendKind::InOrder), 0);
}

#[test]
fn o3_load_behind_a_store_is_not_delayed_by_it() {
    assert_eq!(store_visibility_delay(BackendKind::OutOfOrder), 0);
}

/// `links` loads that each read the address the previous one loaded.
fn pointer_chase(links: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(6, 0).build(),
        i().addi(6, 6, 0x400).build(),
        i().sd(6, 6, 0).build(),
        i().nop().build(),
    ];
    program.extend((0..links).map(|_| i().ld(6, 6, 0).build()));
    program.extend(done_marker());
    program
}

/// Cycles each dependent load adds with an L1D of `l1d_latency`.
fn load_to_use(backend: BackendKind, l1d_latency: u64) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.writeback_width = Some(8);
    config.cache.l1_d.enabled = true;
    config.cache.l1_d.latency = l1d_latency;
    let short = cycles_to_finish(&config, &pointer_chase(10));
    let long = cycles_to_finish(&config, &pointer_chase(30));
    (long - short) / 20
}

/// `links` store/load/multiply links through one address: each load reads
/// back the store just before it, a 3-cycle multiply by one passes the
/// value on, and the next store writes it. The multiply keeps each link
/// longer than the front end takes to fetch it, so the links measure
/// latency.
fn store_load_chain(links: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(10, 0).build(),
        i().addi(10, 10, 0x400).build(),
        i().addi(6, 0, 1).build(),
        i().addi(7, 0, 1).build(),
    ];
    for _ in 0..links {
        program.push(i().sd(10, 6, 0).build());
        program.push(i().ld(6, 10, 0).build());
        program.push(i().mul(6, 6, 7).build());
    }
    program.extend(done_marker());
    program
}

/// Cycles each store/load/multiply link adds with an L1D of `l1d_latency`
/// and the given store-forward latency. Every load waits for its store's
/// address, so the loads never speculate.
fn forwarded_link(backend: BackendKind, l1d_latency: u64, forward_latency: Option<u64>) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 2;
    config.pipeline.writeback_width = Some(8);
    config.pipeline.mem_dep_predictor = MemDepPredictorKind::Blind;
    config.pipeline.fu_config.int_mul_latency = 3;
    config.pipeline.store_forward_latency = forward_latency;
    config.cache.l1_d.enabled = true;
    config.cache.l1_d.latency = l1d_latency;
    let short = cycles_to_finish(&config, &store_load_chain(10));
    let long = cycles_to_finish(&config, &store_load_chain(30));
    (long - short) / 20
}

fn assert_forwarded_load_takes_an_l1d_hit_latency_by_default(backend: BackendKind) {
    for l1d_latency in [1, 4, 9] {
        assert_eq!(
            forwarded_link(backend, l1d_latency, None),
            forwarded_link(backend, l1d_latency, Some(l1d_latency)),
            "{backend:?} l1d latency {l1d_latency}: unset forwards at the L1D hit latency"
        );
    }
}

#[test]
fn inorder_forwarded_load_takes_an_l1d_hit_latency_by_default() {
    assert_forwarded_load_takes_an_l1d_hit_latency_by_default(BackendKind::InOrder);
}

#[test]
fn o3_forwarded_load_takes_an_l1d_hit_latency_by_default() {
    assert_forwarded_load_takes_an_l1d_hit_latency_by_default(BackendKind::OutOfOrder);
}

fn assert_forwarded_load_takes_the_configured_latency(backend: BackendKind) {
    let zero = forwarded_link(backend, 4, Some(0));
    for forward_latency in [1, 3, 9] {
        assert_eq!(
            forwarded_link(backend, 4, Some(forward_latency)) - zero,
            forward_latency,
            "{backend:?} forward latency {forward_latency}: each cycle of it lengthens a link"
        );
    }
}

#[test]
fn inorder_forwarded_load_takes_the_configured_latency() {
    assert_forwarded_load_takes_the_configured_latency(BackendKind::InOrder);
}

#[test]
fn o3_forwarded_load_takes_the_configured_latency() {
    assert_forwarded_load_takes_the_configured_latency(BackendKind::OutOfOrder);
}

/// With the store's address resolved ahead of its data, a link is the
/// data half's cycle to the waiting load's replay, the forward and the
/// multiply.
#[test]
fn o3_zero_latency_forwarded_link_is_the_replay_cycle_and_the_multiply() {
    assert_eq!(forwarded_link(BackendKind::OutOfOrder, 1, Some(0)), 1 + 3);
}

/// A divide, a store of its result (or a `nop` in the store's place), then
/// a load from another address and a dependent chain longer than the
/// divide.
fn load_past_a_store_of_slow_data(with_store: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let store = if with_store { i().sd(10, 13, 0).build() } else { i().nop().build() };
    let mut program = vec![
        i().auipc(10, 0).build(),
        i().addi(10, 10, 0x400).build(),
        i().addi(11, 0, 100).build(),
        i().addi(12, 0, 7).build(),
        i().div(13, 11, 12).build(),
        store,
        i().ld(6, 10, 8).build(),
    ];
    program.extend((0..40).map(|_| i().addi(6, 6, 1).build()));
    program.extend(done_marker());
    program
}

/// A store whose data waits on a divide resolves its address at once, so a
/// younger load to another address, which waits for every older store's
/// address, runs alongside the divide.
#[test]
fn o3_a_load_does_not_wait_for_an_older_stores_data() {
    let mut config = Config::default();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.width = 4;
    config.pipeline.mem_dep_predictor = MemDepPredictorKind::Blind;
    config.pipeline.fu_config.int_div_latency = 20;

    let with_store = cycles_to_finish(&config, &load_past_a_store_of_slow_data(true));
    let without = cycles_to_finish(&config, &load_past_a_store_of_slow_data(false));

    assert_eq!(with_store, without);
}

/// Address generation, then the L1D; the value is bypassed from memory2
/// into the dependent's issue, as Rocket bypasses its D-cache response
/// from MEM into the next instruction's EX (`RocketCore.scala`).
#[test]
fn inorder_dependent_load_waits_the_l1d_latency_plus_one_cycle() {
    for l1d_latency in [1, 4] {
        assert_eq!(
            load_to_use(BackendKind::InOrder, l1d_latency),
            l1d_latency + 1,
            "l1d latency {l1d_latency}"
        );
    }
}

/// Address generation, then the L1D; the load writes back as its data
/// returns, as gem5's O3 LSQ does.
#[test]
fn o3_dependent_load_waits_the_l1d_latency_plus_one_cycle() {
    for l1d_latency in [1, 4] {
        assert_eq!(
            load_to_use(BackendKind::OutOfOrder, l1d_latency),
            l1d_latency + 1,
            "l1d latency {l1d_latency}"
        );
    }
}

/// Runs `program` until `done` holds; returns its control stall cycles.
fn control_stalls(config: &Config, program: &[u32], done: impl Fn(&TestContext) -> bool) -> f64 {
    let mut tc = TestContext::new_with_config(config)
        .with_memory(MEM_SIZE, BASE_ADDR)
        .load_program(BASE_ADDR, program);
    tc.run_until(5_000, done).expect("program never finished");
    let paths = &tc.sim.state.cores[0].units.stat_paths.pipeline;
    tc.sim.state.stats.get(paths.stalls_control).unwrap_or(0.0)
}

fn mispredict_control_stalls(config: &Config) -> f64 {
    control_stalls(config, &mispredicted_branch(), |tc| tc.get_reg(TARGET_REG) == TARGET_VALUE)
}

#[test]
fn a_program_without_redirects_has_no_control_stalls() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stalls = control_stalls(&config(backend, 3), &dependent_multiply_chain(), |tc| {
            tc.get_reg(DONE_REG) == DONE_VALUE
        });

        assert_eq!(stalls, 0.0, "{backend:?}");
    }
}

/// A squash's control stalls run from the redirect until rename hands on
/// the target: the front-end refill, not the cycles the redirect pends.
#[test]
fn control_stalls_count_the_refill_after_the_redirect_is_taken() {
    for (backend, refill) in [(BackendKind::InOrder, 3.0), (BackendKind::OutOfOrder, 4.0)] {
        let quick = mispredict_control_stalls(&redirect_config(backend, 4, 1));
        let slow = mispredict_control_stalls(&redirect_config(backend, 4, 5));

        assert_eq!((quick, slow), (refill, refill), "{backend:?}");
    }
}

#[test]
fn o3_control_stalls_include_rename_waiting_out_the_rob_squash() {
    let (_, held, _) = squash_recovery(1);
    let mut config = redirect_config(BackendKind::OutOfOrder, 4, 2);
    config.pipeline.squash_width = 1;

    let stalls = mispredict_control_stalls(&config);

    assert_eq!(stalls, held);
}

const MSCRATCH: u32 = 0x340;
const MTVEC: u32 = 0x305;

/// `inst`, then the done marker: the cycles a squash after `inst` adds.
fn system_then_done(inst: u32) -> Vec<u32> {
    let mut program = vec![inst];
    program.extend(done_marker());
    program
}

fn config_with_csr_squash(backend: BackendKind, policy: CsrSquash) -> Config {
    let mut config = config(backend, 3);
    config.pipeline.csr_squash = Some(policy);
    config
}

/// Cycles `inst` and the marker take under `policy`.
fn cycles_under(backend: BackendKind, policy: CsrSquash, inst: u32) -> u64 {
    cycles_to_finish(&config_with_csr_squash(backend, policy), &system_then_done(inst))
}

/// Cycles `inst` and the marker take when nothing squashes after it: under
/// `Never` on the out-of-order backend; on the in-order one, which has no
/// `Never`, the cycles of an ALU instruction, which a system instruction
/// that does not squash flows through the pipeline like.
fn cycles_unsquashed(backend: BackendKind, inst: u32) -> u64 {
    match backend {
        BackendKind::InOrder => {
            let addi = InstructionBuilder::new().addi(3, 0, 1).build();
            cycles_to_finish(&config(backend, 3), &system_then_done(addi))
        }
        BackendKind::OutOfOrder => cycles_under(backend, CsrSquash::Never, inst),
    }
}

/// `(every, affecting, unsquashed)`: `inst`'s cycles under `EveryAccess`,
/// under `AffectingWrites`, and with no squash.
fn cycles_by_policy(backend: BackendKind, inst: u32) -> (u64, u64, u64) {
    (
        cycles_under(backend, CsrSquash::EveryAccess, inst),
        cycles_under(backend, CsrSquash::AffectingWrites, inst),
        cycles_unsquashed(backend, inst),
    )
}

#[test]
fn a_csr_read_squashes_only_when_every_access_squashes() {
    let csrr_mscratch = InstructionBuilder::new().csrrs(3, MSCRATCH, 0).build();
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let (every, affecting, unsquashed) = cycles_by_policy(backend, csrr_mscratch);

        assert!(every > unsquashed, "{backend:?}: every access {every}, unsquashed {unsquashed}");
        assert_eq!(affecting, unsquashed, "{backend:?}: a read does not steer execution");
    }
}

#[test]
fn a_scratch_csr_write_squashes_only_when_every_access_squashes() {
    let csrw_mscratch = InstructionBuilder::new().csrrw(0, MSCRATCH, 1).build();
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let (every, affecting, unsquashed) = cycles_by_policy(backend, csrw_mscratch);

        assert!(every > unsquashed, "{backend:?}: every access {every}, unsquashed {unsquashed}");
        assert_eq!(affecting, unsquashed, "{backend:?}: Rocket exempts mscratch from its flush");
    }
}

#[test]
fn a_write_to_a_csr_that_steers_execution_squashes_under_both_policies() {
    let csrw_mtvec = InstructionBuilder::new().csrrw(0, MTVEC, 1).build();
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let (every, affecting, unsquashed) = cycles_by_policy(backend, csrw_mtvec);

        assert_eq!(every, affecting, "{backend:?}: an mtvec write flushes under both");
        assert!(
            affecting > unsquashed,
            "{backend:?}: affecting {affecting}, unsquashed {unsquashed}"
        );
    }
}

/// The squash after a CSR access is taken when the access commits on both
/// backends, so a write that steers execution costs what any squashing
/// access costs.
#[test]
fn a_csr_squash_is_taken_from_commit() {
    let csrw_mtvec = InstructionBuilder::new().csrrw(0, MTVEC, 1).build();
    let csrw_mscratch = InstructionBuilder::new().csrrw(0, MSCRATCH, 1).build();
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let steering = cycles_under(backend, CsrSquash::AffectingWrites, csrw_mtvec);
        let every = cycles_under(backend, CsrSquash::EveryAccess, csrw_mscratch);

        assert_eq!(steering, every, "{backend:?}");
    }
}

/// `first`, `second`, then the done marker.
fn pair_then_done(first: u32, second: u32) -> Vec<u32> {
    let mut program = vec![first, second];
    program.extend(done_marker());
    program
}

/// The in-order backend issues a system instruction behind a unit's long
/// operation without waiting for the operation to retire: the instruction
/// takes effect when it retires, in order, like any other.
#[test]
fn an_inorder_system_instruction_does_not_wait_to_be_the_oldest() {
    let mul = InstructionBuilder::new().mul(4, 1, 2).build();
    let csrw_mscratch = InstructionBuilder::new().csrrw(0, MSCRATCH, 1).build();
    let addi = InstructionBuilder::new().addi(3, 0, 1).build();
    let mut config = config_with_csr_squash(BackendKind::InOrder, CsrSquash::AffectingWrites);
    config.pipeline.fu_config.int_mul_latency = 20;

    let behind_csr = cycles_to_finish(&config, &pair_then_done(mul, csrw_mscratch));
    let behind_alu = cycles_to_finish(&config, &pair_then_done(mul, addi));

    assert_eq!(behind_csr, behind_alu);
}

/// A CSR read's value exists when it retires, so its dependent issues the
/// cycle after, while a unit's result is forwarded the cycle it is
/// produced: the difference is the stages from execute to commit
/// (memory1, memory2, writeback, commit).
#[test]
fn an_inorder_csr_read_reaches_its_dependent_when_it_retires() {
    let csrr_mscratch = InstructionBuilder::new().csrrs(3, MSCRATCH, 0).build();
    let dependent = InstructionBuilder::new().addi(4, 3, 1).build();
    let independent = InstructionBuilder::new().addi(4, 5, 1).build();
    let config = config_with_csr_squash(BackendKind::InOrder, CsrSquash::AffectingWrites);

    let chained = cycles_to_finish(&config, &pair_then_done(csrr_mscratch, dependent));
    let apart = cycles_to_finish(&config, &pair_then_done(csrr_mscratch, independent));

    assert_eq!(chained - apart, 4);
}

#[test]
fn a_fence_squashes_at_commit_only_when_configured() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut squashing = config(backend, 3);
        squashing.pipeline.fence_squash = true;

        let plain = cycles_to_finish(&config(backend, 3), &system_then_done(FENCE_IORW));
        let squashed = cycles_to_finish(&squashing, &system_then_done(FENCE_IORW));

        assert!(squashed > plain, "{backend:?}: squashing {squashed}, plain {plain}");
    }
}

/// `redirect_config` with the front end's three latencies set.
fn depth_config(
    backend: BackendKind,
    fetch_decode: u64,
    decode_rename: u64,
    rename_issue: u64,
) -> Config {
    let mut config = redirect_config(backend, 1, 1);
    config.pipeline.fetch_decode_latency = fetch_decode;
    config.pipeline.decode_rename_latency = decode_rename;
    config.pipeline.rename_issue_latency = rename_issue;
    config
}

/// The refill after a redirect runs from fetch to rename once, so the
/// control stalls it costs move by exactly the front end's depth.
#[test]
fn each_cycle_from_fetch_to_decode_lengthens_the_refill_a_cycle() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let one = mispredict_control_stalls(&depth_config(backend, 1, 1, 2));
        let three = mispredict_control_stalls(&depth_config(backend, 3, 1, 2));

        assert_eq!(three - one, 2.0, "{backend:?}");
    }
}

#[test]
fn decode_and_rename_in_one_stage_shorten_the_refill_a_cycle() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let shared = mispredict_control_stalls(&depth_config(backend, 1, 0, 2));
        let latched = mispredict_control_stalls(&depth_config(backend, 1, 1, 2));

        assert_eq!(latched - shared, 1.0, "{backend:?}");
    }
}

/// The target is reached after two passes from rename to issue: the first
/// fill and the refill after the redirect.
#[test]
fn each_cycle_from_rename_to_issue_delays_the_refetched_target_twice() {
    for (backend, least) in [(BackendKind::InOrder, 1), (BackendKind::OutOfOrder, 2)] {
        let short = cycles_to_reach_target(&depth_config(backend, 1, 1, least));
        let long = cycles_to_reach_target(&depth_config(backend, 1, 1, least + 2));

        assert_eq!(long - short, 4, "{backend:?}");
    }
}
