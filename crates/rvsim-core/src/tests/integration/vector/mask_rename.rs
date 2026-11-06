//! A masked vector instruction uses the v0 it was renamed with, even when
//! v0 still holds its value from reset and a younger instruction writes
//! v0 before the masked one executes.

use crate::config::BackendKind;
use crate::config::Config;
use crate::isa::rvv::VRegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const DONE_REG: usize = 31;
const DONE: u64 = 7;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

/// `vsetivli zero, 4, e32, m1, tu, mu`.
const VSETIVLI_4_E32_M1: u32 = 0xC102_7057;
/// `vdiv.vv v2, v4, v5`: slow, so the masked add after it waits.
const VDIV_V2_V4_V5: u32 = 0x8642_A157;
/// `vadd.vv v1, v2, v3, v0.t`.
const VADD_V1_V2_V3_MASKED: u32 = 0x0021_80D7;
/// `vmv.v.i v0, -1`: a younger write of v0 that sets every mask bit.
const VMV_V0_ALL_ONES: u32 = 0x5E0F_B057;

/// v0's mask from reset: elements 0 and 2 active.
const MASK: u32 = 0b0101;

fn elements(values: [u32; 4]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

/// v1 after the masked add on `backend`.
fn masked_add_result(backend: BackendKind) -> Vec<u8> {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    let i = InstructionBuilder::new;
    let program = [
        VSETIVLI_4_E32_M1,
        VDIV_V2_V4_V5,
        VADD_V1_V2_V3_MASKED,
        VMV_V0_ALL_ONES,
        i().addi(DONE_REG as u32, 0, DONE as i32).build(),
        i().jal(0, 0).build(),
    ];
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    let vpr = ctx.sim.state.harts[0].regs.vpr_mut();
    vpr.write_bytes(VRegIdx::new(0), &elements([MASK, 0, 0, 0]));
    vpr.write_bytes(VRegIdx::new(1), &elements([7; 4]));
    vpr.write_bytes(VRegIdx::new(3), &elements([1; 4]));
    vpr.write_bytes(VRegIdx::new(4), &elements([10; 4]));
    vpr.write_bytes(VRegIdx::new(5), &elements([2; 4]));
    ctx.sim.sync_arch_regs();

    let done = ctx.run_until(10_000, |ctx| ctx.get_reg(DONE_REG) == DONE);

    assert!(done.is_some(), "{backend:?}: the program finished");
    ctx.cpu().harts[0].regs.vpr().read_bytes(VRegIdx::new(1)).to_vec()
}

#[test]
fn a_masked_op_uses_the_reset_v0_it_was_renamed_with() {
    let expected = elements([10 / 2 + 1, 7, 10 / 2 + 1, 7]);
    for backend in BACKENDS {
        assert_eq!(masked_add_result(backend), expected, "{backend:?}");
    }
}

/// An illegal instruction, to flush the pipeline with a trap.
const ILLEGAL: u32 = 0;
const HANDLER: u64 = PROGRAM_BASE + 0x100;

#[test]
fn rewriting_v0_from_reset_returns_its_physical_register() {
    for backend in BACKENDS {
        let mut config = Config::minimal();
        config.pipeline.backend = backend;
        config.system.console = crate::config::Console::Quiet;
        let i = InstructionBuilder::new;
        let program = [VSETIVLI_4_E32_M1, VMV_V0_ALL_ONES, ILLEGAL];
        let handler = [i().addi(DONE_REG as u32, 0, DONE as i32).build(), i().jal(0, 0).build()];
        let mut ctx = TestContext::new_with_config(&config)
            .load_program(PROGRAM_BASE, &program)
            .load_program(HANDLER, &handler);
        ctx.sim.set_pc(0, PROGRAM_BASE);
        ctx.sim.state.harts[0].csrs.mtvec = HANDLER;

        // The trap's flush checks that every physical register is accounted for.
        let done = ctx.run_until(10_000, |ctx| ctx.get_reg(DONE_REG) == DONE);

        assert!(done.is_some(), "{backend:?}: the trap handler ran");
    }
}
