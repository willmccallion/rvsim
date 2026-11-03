//! Which vector instructions are legal under a given `vtype` (RVV 1.0).

use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, Console};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x200;
const DATA: u64 = PROGRAM_BASE + 0x400;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const T0: u32 = 5;
const MCAUSE: u32 = 0x342;
const VSTART: u32 = 0x008;
const ILLEGAL_INSTRUCTION: u64 = 2;
const RAN: u64 = 99;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

/// vtype immediates: `vlmul` in bits 2:0, `vsew` in bits 5:3.
const E8M1: u32 = 0b000;
const E8M2: u32 = 0b001;

/// `vsetvli x0, rs1, zimm`.
const fn vsetvli(rs1: u32, zimm: u32) -> u32 {
    (zimm << 20) | (rs1 << 15) | (0b111 << 12) | 0x57
}

/// `vl<nf>re8.v vd, (rs1)`.
const fn vl_re8(nf: u32, vd: u32, rs1: u32) -> u32 {
    ((nf - 1) << 29) | (1 << 25) | (0b01000 << 20) | (rs1 << 15) | (vd << 7) | 0x07
}

/// `vs1r.v vs3, (rs1)`.
const fn vs1r(vs3: u32, rs1: u32) -> u32 {
    (1 << 25) | (0b01000 << 20) | (rs1 << 15) | (vs3 << 7) | 0x27
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: u32) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1 << 15)
}

/// `vse8.v vs3, (rs1)`.
const fn vse8(vs3: u32, rs1: u32) -> u32 {
    0x0200_0027 | (vs3 << 7) | (rs1 << 15)
}

/// `vlm.v vd, (rs1)`.
const fn vlm(vd: u32, rs1: u32) -> u32 {
    (1 << 25) | (0b01011 << 20) | (rs1 << 15) | (vd << 7) | 0x07
}

/// What a run left behind: `mcause` as the handler recorded it (zero when
/// nothing trapped), whether the marker after the body ran, and `a3`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Outcome {
    mcause: u64,
    ran_on: bool,
    a3: u64,
}

/// Points `a0` at `DATA`, sets `a2 = 16`, runs `body`, then sets the marker.
fn run(backend: BackendKind, body: &[u32]) -> Outcome {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(A2, 0, 16).build(),
    ];
    program.extend_from_slice(body);
    program.extend([i().addi(A1, 0, RAN as i32).build(), i().jal(0, 0).build()]);
    let handler = [i().csrrs(A2, MCAUSE, 0).build(), i().jal(0, 0).build()];

    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    for (n, word) in handler.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + 4 * n as u64), u64::from(*word), 4);
    }
    for byte in 0..64 {
        ctx.sim.probe_mem_store(PhysAddr::new(DATA + byte), byte + 1, 1);
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.sync_arch_regs();

    ctx.run(5_000);

    let ran_on = ctx.get_reg(A1 as usize) == RAN;
    Outcome {
        mcause: if ran_on { 0 } else { ctx.get_reg(A2 as usize) },
        ran_on,
        a3: ctx.get_reg(A3 as usize),
    }
}

fn ran_cleanly(outcome: &Outcome) -> bool {
    outcome.ran_on && outcome.mcause == 0
}

fn trapped_illegal(outcome: &Outcome) -> bool {
    !outcome.ran_on && outcome.mcause == ILLEGAL_INSTRUCTION
}

#[test]
fn vl1re8_into_an_odd_register_is_legal_under_lmul_2() {
    let outcomes = BACKENDS.map(|b| run(b, &[vsetvli(A2, E8M2), vl_re8(1, 1, A0)]));

    assert!(outcomes.iter().all(ran_cleanly), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn vs1r_from_an_odd_register_is_legal_under_lmul_2() {
    let outcomes = BACKENDS.map(|b| run(b, &[vsetvli(A2, E8M2), vs1r(9, A0)]));

    assert!(outcomes.iter().all(ran_cleanly), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn vlm_into_an_odd_register_is_legal_under_lmul_2() {
    let outcomes = BACKENDS.map(|b| run(b, &[vsetvli(A2, E8M2), vlm(1, A0)]));

    assert!(outcomes.iter().all(ran_cleanly), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn vl2re8_into_an_odd_register_is_illegal_under_lmul_1() {
    let outcomes = BACKENDS.map(|b| run(b, &[vsetvli(A2, E8M1), vl_re8(2, 1, A0)]));

    assert!(outcomes.iter().all(trapped_illegal), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn vl2re8_into_an_even_register_is_legal_under_lmul_1() {
    let outcomes = BACKENDS.map(|b| run(b, &[vsetvli(A2, E8M1), vl_re8(2, 2, A0)]));

    assert!(outcomes.iter().all(ran_cleanly), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn vstart_reads_zero_after_a_vector_store() {
    let i = InstructionBuilder::new;
    let body = [
        vsetvli(A2, E8M1),
        vle8(3, A0),
        i().addi(T0, 0, 3).build(),
        i().csrrw(0, VSTART, T0).build(),
        vse8(3, A0),
        i().csrrs(A3, VSTART, 0).build(),
    ];

    let vstarts = BACKENDS.map(|b| run(b, &body).a3);

    assert_eq!(vstarts, [0, 0], "[InOrder, OutOfOrder]");
}
