//! A checkpoint carries a system's whole architectural state, and restores
//! into another configuration of the same system: a run split across a
//! checkpoint ends where an uninterrupted one does.

use crate::common::{HartId, PhysAddr};
use crate::config::BackendKind;
use crate::config::{Config, HomeAgentConfig};
use crate::isa::privileged::PrivilegeMode;
use crate::soc::coherence::CoherenceFabric;
use crate::system::checkpoint::CheckpointError;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::multihart::{DATA_BASE, MultiHart};

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x1000;
const SQUARES: u64 = 200;
const X5: u32 = 5;
const X6: u32 = 6;
const X7: u32 = 7;
const X8: u32 = 8;
const X9: u32 = 9;
const DONE: u32 = 31;

/// Stores the running sum of `i * i` for each `i` below [`SQUARES`] to
/// consecutive doublewords at [`DATA`], then sets x31.
fn running_squares() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(X5, 1).build(),
        i().addi(X6, 0, 0).build(),
        i().addi(X7, 0, SQUARES as i32).build(),
        i().addi(X8, 0, 0).build(),
        i().mul(X9, X6, X6).build(),
        i().add(X8, X8, X9).build(),
        i().sd(X5, X8, 0).build(),
        i().addi(X5, X5, 8).build(),
        i().addi(X6, X6, 1).build(),
        i().bne(X6, X7, -20).build(),
        i().addi(DONE, 0, 1).build(),
        i().jal(0, 0).build(),
    ]
}

fn config(backend: BackendKind) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    config
}

/// The final sum, every stored sum, and the instructions retired.
fn outcome(ctx: &mut TestContext) -> (u64, Vec<u64>, u64) {
    let stored =
        (0..SQUARES).map(|i| ctx.sim.probe_mem_load(PhysAddr::new(DATA + 8 * i), 8)).collect();
    (ctx.get_reg(X8 as usize), stored, ctx.sim.state.harts[0].instructions_retired)
}

fn run_to_done(ctx: &mut TestContext) {
    let done = ctx.run_until(200_000, |ctx| ctx.get_reg(DONE as usize) == 1);
    assert!(done.is_some(), "the program finishes");
}

fn saved(ctx: &mut TestContext) -> Vec<u8> {
    let mut bytes = Vec::new();
    ctx.sim.save_checkpoint(&mut bytes).expect("save");
    bytes
}

#[test]
fn a_run_checkpointed_on_the_in_order_core_finishes_on_the_o3_core_as_one_run_would() {
    let mut whole = TestContext::new_with_config(&config(BackendKind::OutOfOrder))
        .load_program(PROGRAM_BASE, &running_squares());
    run_to_done(&mut whole);
    let mut first_half = TestContext::new_with_config(&config(BackendKind::InOrder))
        .load_program(PROGRAM_BASE, &running_squares());
    first_half.run(600);
    assert_eq!(first_half.get_reg(DONE as usize), 0, "the checkpoint is taken mid-run");
    let checkpoint = saved(&mut first_half);

    let mut second_half = TestContext::new_with_config(&config(BackendKind::OutOfOrder));
    second_half.sim.restore_checkpoint(&mut checkpoint.as_slice()).expect("restore");
    run_to_done(&mut second_half);

    assert_eq!(outcome(&mut second_half), outcome(&mut whole));
}

#[test]
fn every_csr_pmp_entry_vector_register_and_reservation_survives_a_checkpoint() {
    let mut source = TestContext::new_with_config(&config(BackendKind::InOrder));
    let hart = &mut source.sim.state.harts[0];
    let fields: [&mut u64; 12] = [
        &mut hart.csrs.mstatus,
        &mut hart.csrs.satp,
        &mut hart.csrs.stimecmp,
        &mut hart.csrs.menvcfg,
        &mut hart.csrs.senvcfg,
        &mut hart.csrs.vstart,
        &mut hart.csrs.vxrm,
        &mut hart.csrs.vl,
        &mut hart.csrs.vtype,
        &mut hart.csrs.tselect,
        &mut hart.csrs.tdata1[1],
        &mut hart.csrs.scounteren,
    ];
    for (value, field) in (0x11u64..).zip(fields) {
        *field = value;
    }
    hart.pmp.set_addr(0, 0x2000_0000 >> 2);
    hart.pmp.set_cfg(0, 0x9f);
    hart.privilege = PrivilegeMode::Supervisor;
    let vector: Vec<u8> = (0..hart.regs.vpr().bytes().len()).map(|i| i as u8).collect();
    hart.regs.vpr_mut().set_bytes(&vector);
    source
        .sim
        .state
        .uncore
        .memory
        .reservations_mut()
        .set(HartId::new(0), PhysAddr::new(0x8000_2040));
    let before = source.sim.state.harts[0].csrs.clone();
    let checkpoint = saved(&mut source);

    let mut restored = TestContext::new_with_config(&config(BackendKind::OutOfOrder));
    restored.sim.restore_checkpoint(&mut checkpoint.as_slice()).expect("restore");

    let hart = &restored.sim.state.harts[0];
    assert_eq!(hart.csrs, before);
    assert_eq!(hart.pmp.entries(), source.sim.state.harts[0].pmp.entries());
    assert_eq!(hart.privilege, PrivilegeMode::Supervisor);
    assert_eq!(hart.regs.vpr().bytes(), vector.as_slice());
    let reservations = restored.sim.state.uncore.memory.reservations();
    assert_eq!(reservations.reserved(HartId::new(0)), Some(PhysAddr::new(0x8000_2040)));
}

#[test]
fn a_checkpoint_does_not_restore_into_a_system_with_other_harts() {
    let mut one = TestContext::new_with_config(&config(BackendKind::InOrder));
    let checkpoint = saved(&mut one);
    let mut two_harts = config(BackendKind::InOrder);
    two_harts.system.hart_count = 2;
    let mut two = TestContext::new_with_config(&two_harts);

    let result = two.sim.restore_checkpoint(&mut checkpoint.as_slice());

    assert!(matches!(
        result,
        Err(CheckpointError::Mismatch { what: "harts", saved: 1, current: 2 })
    ));
}

#[test]
fn a_restore_leaves_the_snoop_filter_tracking_nothing() {
    let i = InstructionBuilder::new;
    let program = [
        i().auipc(X5, 0).build(),
        i().addi(X5, X5, (DATA_BASE - PROGRAM_BASE) as i32).build(),
        i().ld(X6, X5, 0).build(),
        i().sd(X5, X6, 64).build(),
        i().jal(0, 0).build(),
    ];
    let mut two_harts = config(BackendKind::OutOfOrder);
    two_harts.system.hart_count = 2;
    two_harts.coherence.home_agent = HomeAgentConfig::SnoopFilter { capacity_factor: 1.5, ways: 8 };
    two_harts.cache.l1_d.enabled = true;
    two_harts.cache.l2.enabled = true;
    let mut system = MultiHart::with_config(&two_harts, &program);
    let _ = system.run_until_exit(2000);
    let tracked = |system: &MultiHart| {
        system
            .sim
            .state
            .uncore
            .coherence
            .as_ref()
            .and_then(CoherenceFabric::tracked_lines)
            .map(|lines| lines.len())
    };
    assert!(tracked(&system).is_some_and(|lines| lines > 0), "the harts cached lines");
    let mut checkpoint = Vec::new();
    system.sim.save_checkpoint(&mut checkpoint).expect("save");

    system.sim.restore_checkpoint(&mut checkpoint.as_slice()).expect("restore");

    assert_eq!(tracked(&system), Some(0));
}

#[test]
fn ram_that_was_zero_at_the_save_is_zero_after_a_restore_and_costs_no_space() {
    let mut system = TestContext::new_with_config(&config(BackendKind::InOrder))
        .load_program(PROGRAM_BASE, &running_squares());
    let far = PROGRAM_BASE + 0x40_0000;
    let checkpoint = saved(&mut system);
    let mut restored = TestContext::new_with_config(&config(BackendKind::InOrder));
    restored.sim.probe_mem_store(PhysAddr::new(far), 0x1234, 8);

    restored.sim.restore_checkpoint(&mut checkpoint.as_slice()).expect("restore");

    assert_eq!(restored.sim.probe_mem_load(PhysAddr::new(far), 8), 0);
    assert_eq!(restored.sim.probe_mem_load(PhysAddr::new(PROGRAM_BASE), 4), 0x0000_1297);
    let ram = restored.sim.state.memory.ram().expect("the system has RAM").size() as usize;
    assert!(
        checkpoint.len() < ram / 100,
        "a {}-byte checkpoint of {ram} bytes of RAM",
        checkpoint.len()
    );
}
