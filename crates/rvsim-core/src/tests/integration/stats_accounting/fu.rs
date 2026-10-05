//! Functional-unit utilisation: the cycles each type of unit could take no
//! other instruction, one per instruction for a pipelined unit and its
//! latency for one that is not.

use super::program::{A2, BACKENDS, T0, T1, T2, ending_in_spin, run_to_pc, system};
use super::{Recorder, accounting_checks};
use crate::config::BackendKind;
use crate::tests::support::builder::instruction::InstructionBuilder;

/// `fadd.d ft2, ft1, ft1`.
const FADD_D: u32 = 0x0210_f153;
/// `fadd.d ft7, ft1, ft1`.
const FADD_D_AGAIN: u32 = 0x0210_f3d3;
/// `fmul.d ft6, ft1, ft1`.
const FMUL_D: u32 = 0x1210_f353;
/// `fmadd.d ft3, ft1, ft2, ft2`.
const FMADD_D: u32 = 0x1220_f1c3;
/// `fdiv.d ft4, ft2, ft1`.
const FDIV_D: u32 = 0x1a11_7253;
/// `fsqrt.d ft5, ft2`.
const FSQRT_D: u32 = 0x5a01_72d3;
/// `fld ft1, 0(a1)`.
const FLD: u32 = 0x0005_b087;
/// `vsetvli t0, zero, e64, m1, ta, ma`.
const VSETVLI: u32 = 0x0d80_72d7;
/// `vle64.v v1, (a1)`.
const VLE64: u32 = 0x0205_f087;
/// `vse64.v v2, (a2)`.
const VSE64: u32 = 0x0206_7127;
/// `vadd.vv v2, v1, v1`.
const VADD_VV: u32 = 0x0210_8157;
/// `vmul.vv v7, v1, v1`.
const VMUL_VV: u32 = 0x9610_a3d7;
/// `vdiv.vv v8, v1, v1`.
const VDIV_VV: u32 = 0x8610_a457;
/// `vfadd.vv v3, v1, v1`.
const VFADD_VV: u32 = 0x0210_91d7;
/// `vfmul.vv v9, v1, v1`.
const VFMUL_VV: u32 = 0x9210_94d7;
/// `vfdiv.vv v10, v1, v1`.
const VFDIV_VV: u32 = 0x8210_9557;
/// `vrgather.vv v11, v1, v2`.
const VRGATHER_VV: u32 = 0x3211_05d7;

fn util(rec: &mut Recorder, sim: &crate::Simulator, unit: &str) -> u64 {
    rec.read(sim, &format!("core0.fu.util.{unit}"))
}

/// Scalar ops of every class, in counts that differ, with no branch until
/// the final spin.
fn scalar_classes() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().addi(T0, 0, 7).build(),
        i().addi(T1, 0, 3).build(),
        i().addi(T2, 0, 1).build(),
        i().mul(T2, T0, T1).build(),
        i().mul(T2, T1, T1).build(),
        i().div(T2, T0, T1).build(),
        i().div(T2, T1, T0).build(),
        FLD,
        i().sd(A2, T0, 0).build(),
        FADD_D,
        FADD_D_AGAIN,
        FMUL_D,
        FMADD_D,
        FDIV_D,
        FSQRT_D,
    ])
}

fn each_scalar_unit_counts_its_busy_cycles(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let (program, end) = scalar_classes();
        let mut ctx = system(backend, &program, &[]);
        let units = ctx.sim.state.config.pipeline.fu_config.clone();

        run_to_pc(&mut ctx, end, &context);

        let expected = [
            ("int_alu", 3),
            ("int_mul", 2),
            ("int_div", 2 * units.int_div_latency),
            ("mem", 2),
            ("fp_add", 2),
            ("fp_mul", 1),
            ("fp_fma", 1),
            ("fp_div_sqrt", 2 * units.fp_div_sqrt_latency),
        ];
        for (unit, cycles) in expected {
            rec.expect(&ctx.sim, &format!("core0.fu.util.{unit}"), cycles, &context);
        }
        let branch = util(rec, &ctx.sim, "branch");
        assert!(branch >= 1, "{context}: the spinning jump, at least once");
    }
}

/// Vector ops of every class, then the spin.
fn vector_classes() -> (Vec<u32>, u64) {
    ending_in_spin(vec![
        VSETVLI,
        VLE64,
        VADD_VV,
        VMUL_VV,
        VDIV_VV,
        VFADD_VV,
        VFMUL_VV,
        VFDIV_VV,
        VRGATHER_VV,
        VSE64,
    ])
}

fn each_vector_unit_counts_its_busy_cycles(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let (program, end) = vector_classes();
        let mut ctx = system(backend, &program, &[]);
        let units = ctx.sim.state.config.pipeline.fu_config.clone();

        run_to_pc(&mut ctx, end, &context);

        // vsetvli and vadd use the vector integer ALU.
        let pipelined = [
            ("vec_int_alu", 2),
            ("vec_int_mul", 1),
            ("vec_fp_alu", 1),
            ("vec_fp_fma", 1),
            ("vec_mem", 2),
            ("vec_permute", 1),
        ];
        for (unit, ops) in pipelined {
            let busy = util(rec, &ctx.sim, unit);
            // The in-order backend refetches what follows each vector op,
            // so the next one can issue twice.
            match backend {
                BackendKind::OutOfOrder => assert_eq!(busy, ops, "{context}: {unit}"),
                BackendKind::InOrder => assert!(busy >= ops, "{context}: {unit}: {busy}"),
            }
        }
        for (unit, latency) in [
            ("vec_int_div", units.vec_int_div_latency),
            ("vec_fp_div_sqrt", units.vec_fp_div_sqrt_latency),
        ] {
            let busy = util(rec, &ctx.sim, unit);
            assert!(busy >= latency, "{context}: {unit}: {busy} cycles, latency {latency}");
        }
    }
}

fn a_squashed_op_still_counts_the_cycles_its_unit_was_busy(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    // A divide on the path a mispredicted branch skips: it issues before
    // the branch resolves, then is squashed.
    let (program, end) = ending_in_spin(vec![
        i().addi(T0, 0, 1).build(),
        i().bne(T0, 0, 8).build(),
        i().div(T1, T0, T0).build(),
        i().addi(T2, 0, 0).build(),
    ]);
    let context = "OutOfOrder";
    let mut ctx = system(BackendKind::OutOfOrder, &program, &[]);
    let latency = ctx.sim.state.config.pipeline.fu_config.int_div_latency;

    run_to_pc(&mut ctx, end, context);

    assert_eq!(ctx.sim.stats().get("core0.commit.op.alu"), Some(2.0), "the divide never retired");
    let busy = util(rec, &ctx.sim, "int_div");
    assert_eq!(busy, latency, "{context}: the squashed divide held the divider");
}

accounting_checks!(
    each_scalar_unit_counts_its_busy_cycles,
    each_vector_unit_counts_its_busy_cycles,
    a_squashed_op_still_counts_the_cycles_its_unit_was_busy,
);
