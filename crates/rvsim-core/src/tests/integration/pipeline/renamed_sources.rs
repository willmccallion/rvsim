//! The out-of-order backend reads every source operand through the rename
//! map: an FMA's third operand and a vector op's vs1 group come from the
//! physical registers their producers were renamed to, not from whatever
//! a later instruction has renamed the architectural register to since.

use crate::config::{BackendKind, Config};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{T0, X0, X1, X2, X3};

/// `fcvt.d.l ft1, ra`.
const FCVT_D_L_FT1: u32 = 0xd220_f0d3;
/// `fcvt.d.l ft2, sp`.
const FCVT_D_L_FT2: u32 = 0xd221_7153;
/// `fcvt.d.l ft3, gp`.
const FCVT_D_L_FT3: u32 = 0xd221_f1d3;
/// `fmadd.d ft4, ft1, ft2, ft3`.
const FMADD_D: u32 = 0x1a20_f243;
/// `fcvt.l.d t0, ft4, rtz`.
const FCVT_L_D_T0: u32 = 0xc222_12d3;
/// `vsetivli zero, 2, e64, m1, ta, ma`.
const VSETIVLI: u32 = 0xcd81_7057;
/// `vmv.v.x v1, ra`.
const VMV_V1: u32 = 0x5e00_c0d7;
/// `vmv.v.x v2, sp`.
const VMV_V2: u32 = 0x5e01_4157;
/// `vadd.vv v3, v2, v1`.
const VADD_VV: u32 = 0x0220_81d7;
/// `vmv.v.x v1, gp`: a later write to v1, renamed while the add waits.
const VMV_V1_AGAIN: u32 = 0x5e01_c0d7;
/// `vmv.x.s t0, v3`.
const VMV_X_S_T0: u32 = 0x4230_22d7;
/// `jal x0, 0`: where the program spins once it is done.
const SPIN: u32 = 0x0000_006f;

const A: u16 = 2;
const B: u16 = 3;
const C: u16 = 5;

/// Sets x1, x2 and x3 to `A`, `B` and `C`, runs `body` on the out-of-order
/// backend, and returns t0 once every instruction but the closing spin has
/// retired.
fn t0_after(body: &[u32]) -> u64 {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> = [(X1, A), (X2, B), (X3, C)]
        .iter()
        .map(|&(reg, value)| i().addi(reg, X0, i32::from(value)).build())
        .collect();
    program.extend(body);
    program.push(SPIN);
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);
    let instructions = program.len() as u64 - 1;

    ctx.run_until(2_000, |ctx| ctx.state().harts[0].instructions_retired >= instructions)
        .expect("the program retired");

    ctx.get_reg(T0)
}

#[test]
fn an_fma_reads_its_renamed_third_operand() {
    let body = [FCVT_D_L_FT1, FCVT_D_L_FT2, FCVT_D_L_FT3, FMADD_D, FCVT_L_D_T0];

    assert_eq!(t0_after(&body), u64::from(A * B + C));
}

#[test]
fn a_vector_op_reads_its_renamed_vs1_group() {
    let body = [VSETIVLI, VMV_V1, VMV_V2, VADD_VV, VMV_V1_AGAIN, VMV_X_S_T0];

    assert_eq!(t0_after(&body), u64::from(A + B));
}
