//! An instruction waits only for the registers it reads, not for whatever
//! register the bits in an unused rs1 or rs2 field happen to name.

use crate::config::{BackendKind, Config};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{X0, X5, X6, X16, X17, X18};

/// Dependent multiplies after the instruction under test, so a false wait
/// on the divide shows up as cycles after it completes.
const DEPENDENT_MULTIPLIES: usize = 4;

/// `addi x5, x0, imm`'s immediate bits [4:0] sit in the rs2 field, so 16
/// names x16, which the divide writes; 17 names x17, which is long ready.
const IMM_NAMING_X16: i32 = 16;
const IMM_NAMING_X17: i32 = 17;

/// `lui`'s immediate bits [7:3] sit in the rs1 field.
const LUI_RS1_SHIFT: u32 = 3;

/// Cycles until the program retires on `backend`: x16 is written by a
/// divide, then `reader` writes x5, then multiplies depend on x5.
fn cycles_with(backend: BackendKind, reader: u32) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(X17, X0, 100).build(),
        i().addi(X18, X0, 7).build(),
        i().div(X16, X17, X18).build(),
        reader,
    ];
    program.extend((0..DEPENDENT_MULTIPLIES).map(|_| i().mul(X6, X5, X5).build()));
    program.push(i().jal(X0, 0).build());
    let retired = program.len() as u64 - 1;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);

    ctx.run_until(2_000, |ctx| ctx.state().harts[0].instructions_retired >= retired)
        .expect("the program retired")
}

fn addi_x5(imm: i32) -> u32 {
    InstructionBuilder::new().addi(X5, X0, imm).build()
}

fn lui_x5_with_rs1_field(rs1: u32) -> u32 {
    let imm = i32::try_from(rs1 << LUI_RS1_SHIFT).expect("a five-bit field fits");
    InstructionBuilder::new().lui(X5, imm).build()
}

#[test]
fn an_immediate_naming_a_busy_register_does_not_delay_an_out_of_order_addi() {
    let busy = cycles_with(BackendKind::OutOfOrder, addi_x5(IMM_NAMING_X16));
    let ready = cycles_with(BackendKind::OutOfOrder, addi_x5(IMM_NAMING_X17));

    assert_eq!(busy, ready);
}

#[test]
fn an_immediate_naming_a_busy_register_does_not_delay_an_in_order_addi() {
    let busy = cycles_with(BackendKind::InOrder, addi_x5(IMM_NAMING_X16));
    let ready = cycles_with(BackendKind::InOrder, addi_x5(IMM_NAMING_X17));

    assert_eq!(busy, ready);
}

#[test]
fn a_lui_does_not_wait_on_the_register_its_rs1_bits_name() {
    for backend in [BackendKind::OutOfOrder, BackendKind::InOrder] {
        let busy = cycles_with(backend, lui_x5_with_rs1_field(16));
        let ready = cycles_with(backend, lui_x5_with_rs1_field(17));

        assert_eq!(busy, ready, "{backend:?}");
    }
}
