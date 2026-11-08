//! Which vector instructions are legal under a given `vtype` (RVV 1.0).

use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, Console};
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext, store_words};
use crate::tests::support::regs::{A0, A1, A2, A3, A4, T0, X0};

const HANDLER: u64 = PROGRAM_BASE + 0x200;
const DATA: u64 = PROGRAM_BASE + 0x400;
const MCAUSE: u32 = 0x342;
const VSTART: u32 = 0x008;
const VTYPE: u32 = 0xC21;
const VILL: u64 = 1 << 63;
const ILLEGAL_INSTRUCTION: u64 = 2;
const RAN: u64 = 99;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

/// vtype immediates: `vlmul` in bits 2:0, `vsew` in bits 5:3.
const E8M1: u32 = 0b000;
const E8M2: u32 = 0b001;
const RESERVED_LMUL: u32 = 0b100;

/// `vsetvli x0, rs1, zimm`.
const fn vsetvli(rs1: RegIdx, zimm: u32) -> u32 {
    (zimm << 20) | (rs1.field() << 15) | (0b111 << 12) | 0x57
}

/// `vsetvl rd, rs1, rs2`.
const fn vsetvl(rd: RegIdx, rs1: RegIdx, rs2: RegIdx) -> u32 {
    (0b100_0000 << 25)
        | (rs2.field() << 20)
        | (rs1.field() << 15)
        | (0b111 << 12)
        | (rd.field() << 7)
        | 0x57
}

/// `vsetivli x0, uimm, zimm`.
const fn vsetivli(uimm: u32, zimm: u32) -> u32 {
    (0b11 << 30) | (zimm << 20) | (uimm << 15) | (0b111 << 12) | 0x57
}

/// `vl<nf>re8.v vd, (rs1)`.
const fn vl_re8(nf: u32, vd: u32, rs1: RegIdx) -> u32 {
    ((nf - 1) << 29) | (1 << 25) | (0b01000 << 20) | (rs1.field() << 15) | (vd << 7) | 0x07
}

/// `vs1r.v vs3, (rs1)`.
const fn vs1r(vs3: u32, rs1: RegIdx) -> u32 {
    (1 << 25) | (0b01000 << 20) | (rs1.field() << 15) | (vs3 << 7) | 0x27
}

/// `vle8.v vd, (rs1)`.
const fn vle8(vd: u32, rs1: RegIdx) -> u32 {
    0x0200_0007 | (vd << 7) | (rs1.field() << 15)
}

/// `vse8.v vs3, (rs1)`.
const fn vse8(vs3: u32, rs1: RegIdx) -> u32 {
    0x0200_0027 | (vs3 << 7) | (rs1.field() << 15)
}

/// `vmv1r.v vd, vs2`.
const fn vmv1r(vd: u32, vs2: u32) -> u32 {
    (0b10_0111 << 26) | (1 << 25) | (vs2 << 20) | (0b011 << 12) | (vd << 7) | 0x57
}

/// `vfadd.vv vd, vs2, vs1`.
const fn vfadd_vv(vd: u32, vs2: u32, vs1: u32) -> u32 {
    (1 << 25) | (vs2 << 20) | (vs1 << 15) | (0b001 << 12) | (vd << 7) | 0x57
}

/// `vlm.v vd, (rs1)`.
const fn vlm(vd: u32, rs1: RegIdx) -> u32 {
    (1 << 25) | (0b01011 << 20) | (rs1.field() << 15) | (vd << 7) | 0x07
}

/// What a run left behind: `mcause` as the handler recorded it (zero when
/// nothing trapped), whether the marker after the body ran, and `a3`/`a4`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Outcome {
    mcause: u64,
    ran_on: bool,
    a3: u64,
    a4: u64,
}

/// Points `a0` at `DATA`, sets `a2 = 16`, runs `body`, then sets the marker.
fn run(backend: BackendKind, body: &[u32]) -> Outcome {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(A0, 0).build(),
        i().addi(A0, A0, (DATA - PROGRAM_BASE) as i32).build(),
        i().addi(A2, X0, 16).build(),
    ];
    program.extend_from_slice(body);
    program.extend([i().addi(A1, X0, RAN as i32).build(), i().jal(X0, 0).build()]);
    let handler = [i().csrrs(A2, MCAUSE, X0).build(), i().jal(X0, 0).build()];

    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);
    store_words(&mut ctx.sim, HANDLER, &handler);
    for byte in 0..64 {
        ctx.sim.probe_mem_store(PhysAddr::new(DATA + byte), byte + 1, ProbeWidth::B1);
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.sync_arch_regs();

    ctx.run(5_000);

    let ran_on = ctx.get_reg(A1) == RAN;
    Outcome {
        mcause: if ran_on { 0 } else { ctx.get_reg(A2) },
        ran_on,
        a3: ctx.get_reg(A3),
        a4: ctx.get_reg(A4),
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
        i().addi(T0, X0, 3).build(),
        i().csrrw(X0, VSTART, T0).build(),
        vse8(3, A0),
        i().csrrs(A3, VSTART, X0).build(),
    ];

    let vstarts = BACKENDS.map(|b| run(b, &body).a3);

    assert_eq!(vstarts, [0, 0], "[InOrder, OutOfOrder]");
}

#[test]
fn vsetvl_with_a_reserved_vtype_bit_sets_vill() {
    let i = InstructionBuilder::new;
    let body =
        [i().addi(T0, X0, 1 << 8).build(), vsetvl(A4, A2, T0), i().csrrs(A3, VTYPE, X0).build()];

    let results = BACKENDS.map(|b| {
        let outcome = run(b, &body);
        (outcome.a3, outcome.a4)
    });

    assert_eq!(results, [(VILL, 0); 2], "[InOrder, OutOfOrder]: (vtype, vl)");
}

#[test]
fn vsetivli_sets_the_vtype_it_names() {
    let i = InstructionBuilder::new;
    let body = [vsetivli(4, E8M2), i().csrrs(A3, VTYPE, X0).build()];

    let vtypes = BACKENDS.map(|b| run(b, &body).a3);

    assert_eq!(vtypes, [u64::from(E8M2); 2], "[InOrder, OutOfOrder]");
}

#[test]
fn unit_stride_load_under_vill_is_an_illegal_instruction() {
    let outcomes = BACKENDS.map(|b| run(b, &[vsetvli(A2, RESERVED_LMUL), vle8(1, A0)]));

    assert!(outcomes.iter().all(trapped_illegal), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn whole_register_move_under_vill_executes() {
    let outcomes = BACKENDS.map(|b| run(b, &[vsetvli(A2, RESERVED_LMUL), vmv1r(2, 1)]));

    assert!(outcomes.iter().all(ran_cleanly), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn whole_register_load_and_store_under_vill_move_the_data() {
    let i = InstructionBuilder::new;
    let body = [
        vsetvli(A2, RESERVED_LMUL),
        vl_re8(1, 1, A0),
        i().addi(A3, A0, 64).build(),
        vs1r(1, A3),
        i().ld(A4, A3, 0).build(),
    ];

    let copied = BACKENDS.map(|b| run(b, &body).a4);

    assert_eq!(copied, [0x0807_0605_0403_0201; 2], "[InOrder, OutOfOrder]");
}

#[test]
fn floating_point_add_at_sew_8_is_an_illegal_instruction() {
    let outcomes = BACKENDS.map(|b| run(b, &[vsetvli(A2, E8M1), vfadd_vv(4, 2, 3)]));

    assert!(outcomes.iter().all(trapped_illegal), "[InOrder, OutOfOrder]: {outcomes:?}");
}
