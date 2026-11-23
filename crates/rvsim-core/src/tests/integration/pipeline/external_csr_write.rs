//! A CSR written from outside the hart, as a debugger or the Python API
//! writes one, holds for every instruction after the write, including those
//! the pipeline has already taken in.

use crate::config::{BackendKind, Config};
use crate::isa::csr;
use crate::tests::support::builder::instruction::{InstructionBuilder, JAL_SELF};
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, T1, T2, X0};

/// `vsetivli zero, 1, e8, m1, ta, ma`.
const VSETIVLI_ONE_BYTE: u32 = 0xCC00_F057;
/// `vmv.v.i v1, 0`.
const VMV_V1_ZERO: u32 = 0x5E00_30D7;
/// `vaadd.vx v2, v1, t2`: the average of 0 and `t2`, rounded by `vxrm`.
const VAADD_V2_V1_T2: u32 = 0x2613_E157;
/// `vmv.x.s a0, v2`.
const VMV_A0_V2: u32 = 0x4220_2557;
const DIVIDEND: i32 = 7;
/// Divides that hold the average back, so it waits renamed in the issue
/// queue when the rounding mode changes.
const DIVIDES: usize = 4;
/// `vxrm` round-down: the average of 0 and 7 is 3 (round-to-nearest-up,
/// the reset value, gives 4).
const ROUND_DOWN: u64 = 2;
const AVERAGE_ROUNDED_DOWN: u64 = 3;
/// Cycles in which the out-of-order core renames the average but has yet to
/// execute it.
const CYCLES_BEFORE_THE_WRITE: u64 = 20;
const CYCLES_TO_FINISH: u64 = 2_000;

fn average_behind_divides() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().addi(T2, X0, DIVIDEND).build(), i().addi(T1, X0, 1).build()];
    program.extend((0..DIVIDES).map(|_| i().div(T2, T2, T1).build()));
    program.extend([VSETIVLI_ONE_BYTE, VMV_V1_ZERO, VAADD_V2_V1_T2, VMV_A0_V2, JAL_SELF]);
    program
}

#[test]
fn o3_a_vxrm_write_reaches_an_average_already_renamed() {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.width = 4;
    let mut ctx = TestContext::with_fast_memory(&config)
        .load_program(PROGRAM_BASE, &average_behind_divides());
    ctx.run(CYCLES_BEFORE_THE_WRITE);

    assert!(ctx.sim.write_csr(0, csr::VXRM, ROUND_DOWN), "vxrm is writable");
    ctx.run(CYCLES_TO_FINISH);

    assert_eq!(ctx.get_reg(A0), AVERAGE_ROUNDED_DOWN);
}
