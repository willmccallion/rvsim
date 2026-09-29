//! How `vsetvl` reaches the vector CSRs: at commit, so a fault older than
//! it leaves them alone, and without a pipeline flush, so a strip-mined
//! loop pays only decode's wait for the new configuration.

use crate::support::builder::instruction::InstructionBuilder;
use crate::support::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::BackendKind;
use rvsim_core::config::Config;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const A0: u32 = 10;
const A2: u32 = 12;
const A3: u32 = 13;
const VL: u32 = 0xC20;
const VTYPE: u32 = 0xC21;
/// `vtype` for e32, m1, tu, mu.
const E32_M1: u32 = 0b010_000;
/// `vtype` for e64, m1, tu, mu.
const E64_M1: u32 = 0b011_000;

/// `vsetivli rd, uimm, zimm`.
const fn vsetivli(rd: u32, uimm: u32, zimm: u32) -> u32 {
    0xC000_0057 | (zimm << 20) | (uimm << 15) | (0b111 << 12) | (rd << 7)
}

/// `lr.d t0, (a0)`: an atomic on a misaligned address always traps.
const LR_D_T0_A0: u32 = 0x1005_32AF;

/// Sets vl=3/e32, faults on a misaligned reservation, and, younger than
/// the fault, sets vl=2/e64; then spins.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        vsetivli(0, 3, E32_M1),
        i().addi(A0, 0, 1).build(),
        LR_D_T0_A0,
        vsetivli(0, 2, E64_M1),
        i().jal(0, 0).build(),
    ]
}

/// Records `vl` and `vtype`, then spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().csrrs(A2, VL, 0).build(), i().csrrs(A3, VTYPE, 0).build(), i().jal(0, 0).build()]
}

fn run(backend: BackendKind) -> (u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    for (i, word) in handler().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + (i as u64) * 4), u64::from(*word), 4);
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(500);
    assert_eq!(ctx.sim.state.harts[0].csrs.mcause, 4, "{backend:?}: the misaligned lr.d trapped");
    (ctx.get_reg(A2 as usize), ctx.get_reg(A3 as usize))
}

#[test]
fn a_younger_vsetvl_does_not_reach_the_csrs_before_an_older_trap_o3() {
    assert_eq!(run(BackendKind::OutOfOrder), (3, u64::from(E32_M1)));
}

#[test]
fn a_younger_vsetvl_does_not_reach_the_csrs_before_an_older_trap_inorder() {
    assert_eq!(run(BackendKind::InOrder), (3, u64::from(E32_M1)));
}

/// Eight `vsetivli`s, each followed by an independent `addi`.
fn vsetivli_sequence() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = Vec::new();
    for n in 0..8 {
        program.push(vsetivli(0, 1 + (n % 4), E32_M1));
        program.push(i().addi(A0, A0, 1).build());
    }
    program.push(i().jal(0, 0).build());
    program
}

fn cycles_to_retire(backend: BackendKind, program: &[u32]) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = rvsim_core::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, program);
    let target = program.len() as u64 - 1;
    let mut cycles = 0;
    while ctx.sim.state.harts[0].instructions_retired < target {
        ctx.run(1);
        cycles += 1;
        assert!(cycles < 2000, "{backend:?}: the sequence did not retire");
    }
    assert_eq!(ctx.get_reg(A0 as usize), 8);
    cycles
}

#[test]
fn a_vsetvl_stalls_decode_instead_of_flushing_the_pipeline() {
    let o3 = cycles_to_retire(BackendKind::OutOfOrder, &vsetivli_sequence());
    let inorder = cycles_to_retire(BackendKind::InOrder, &vsetivli_sequence());
    println!("o3={o3} inorder={inorder}");
    assert!(o3 < 60, "out-of-order: {o3} cycles");
    assert!(inorder < 60, "in-order: {inorder} cycles");
}
