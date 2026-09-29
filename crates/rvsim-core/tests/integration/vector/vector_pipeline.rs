//! Pipeline-integration regression tests for vector ops.
//!
//! These tests exercise the full Rename → Issue → Execute path in the
//! in-order backend to catch bugs that don't surface in the per-unit tests
//! of `core::units::vpu::*` (which test the compute kernels in isolation).
//!
//! In particular: a `vsetvli` that consumes a value produced by an
//! immediately-preceding scalar instruction must observe the new scalar
//! value, not a stale/zero GPR read.

use crate::support::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 4096;

/// A long-enough trailing pad of NOPs (`addi x0, x0, 0`) so the program
/// drains the pipeline before the test inspects CSR state.
const NOP: u32 = 0x0000_0013;

/// Build a Config with a wider in-order pipeline. The original Python brief
/// reproduced the bug at width=4 / InOrder, where the rename-to-issue
/// distance is large enough for forwarding/serialization races to surface.
fn wide_inorder_config() -> Config {
    let mut c = Config::default();
    c.pipeline.width = 4;
    c
}

/// Encode `addi rd, rs1, imm` (12-bit signed immediate).
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    let imm = (imm as u32) & 0xFFF;
    (imm << 20) | (rs1 << 15) | (rd << 7) | 0x13
}

/// Encode `vsetvli rd, rs1, vtype` where `zimm` packs SEW/LMUL/TA/MA.
fn vsetvli(rd: u32, rs1: u32, zimm: u32) -> u32 {
    ((zimm & 0x7FF) << 20) | (rs1 << 15) | (0b111 << 12) | (rd << 7) | 0x57
}

// vtype encoding helpers — match `zimm_vsetvli` decoding in `isa::rvv::encoding`.
const fn vtype(sew_bits: u32, lmul_bits: u32, ta: u32, ma: u32) -> u32 {
    (ma << 7) | (ta << 6) | (sew_bits << 3) | lmul_bits
}

const SEW_E8: u32 = 0b000;
const SEW_E32: u32 = 0b010;
const LMUL_M1: u32 = 0b000;
const LMUL_M8: u32 = 0b011;
const LMUL_MF8: u32 = 0b101;

fn run_program(config: &Config, program: &[u32], cycles: u64) -> TestContext {
    let mut ctx = TestContext::new_with_config(config).with_memory(RAM_SIZE, RAM_BASE);
    for (i, inst) in program.iter().enumerate() {
        let addr = RAM_BASE + (i as u64) * 4;
        ctx.sim.probe_mem_store(PhysAddr::new(addr), u64::from(*inst), 4);
    }
    ctx.sim.set_pc(0, RAM_BASE);
    ctx.run(cycles);
    ctx
}

/// Bare minimum: `li t0, 4; vsetvli t1, t0, e32, m1, tu, mu`.
///
/// The vsetvli must read t0 = 4, not the pre-li value of 0. After execution,
/// `vl == 4` (since AVL=4 and VLMAX = LMUL=1 * VLEN=128 / SEW=32 = 4).
#[test]
fn vsetvli_consumes_immediately_preceding_li() {
    let program: Vec<u32> = std::iter::empty()
        .chain([
            addi(5, 0, 4),                                // li t0, 4
            vsetvli(6, 5, vtype(SEW_E32, LMUL_M1, 0, 0)), // vsetvli t1, t0, e32,m1,tu,mu
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .collect();

    let ctx = run_program(&wide_inorder_config(), &program, 500);

    assert_eq!(ctx.get_reg(5), 4, "t0 should hold 4 after li");
    assert_eq!(
        ctx.cpu().harts[0].csrs.vl,
        4,
        "vsetvli e32/m1 with AVL=4 should yield vl=4 (got {})",
        ctx.cpu().harts[0].csrs.vl
    );
    assert_eq!(ctx.get_reg(6), 4, "t1 (vsetvli rd) should equal new vl");
}

/// Same as above but at the default single-issue width (1). Establishes a
/// baseline: if this passes and the wide variant fails, the bug is
/// width-specific (intra-bundle hazards, fetch-group serialization, etc.).
#[test]
fn vsetvli_consumes_immediately_preceding_li_width1() {
    let program: Vec<u32> = std::iter::empty()
        .chain([addi(5, 0, 4), vsetvli(6, 5, vtype(SEW_E32, LMUL_M1, 0, 0))])
        .chain(std::iter::repeat_n(NOP, 32))
        .collect();

    let ctx = run_program(&Config::default(), &program, 500);

    assert_eq!(ctx.get_reg(5), 4);
    assert_eq!(ctx.cpu().harts[0].csrs.vl, 4, "got vl={}", ctx.cpu().harts[0].csrs.vl);
}

/// Closest to the chipsalliance failing-ELF pattern: a flushing vsetvli
/// (which sets the pipeline's redirect_pending bit), then a scalar `li`,
/// then a second vsetvli that needs to consume the just-written t0.
///
/// The first vsetvli is the prior "vector op that flushes". We then expect
/// the second vsetvli to see t0 = 12 and produce vl = 2 (e8/mf8 with VLEN=128
/// gives VLMAX = 1/8 * 128 / 8 = 2; AVL=12 saturates to 2).
#[test]
fn vsetvli_after_flushing_vsetvli_then_li() {
    let program: Vec<u32> = std::iter::empty()
        .chain([
            // Initial config (this also flushes the pipeline).
            vsetvli(6, 0, vtype(SEW_E32, LMUL_M8, 0, 0)),
            // Set up new AVL.
            addi(5, 0, 12),
            // Reconfigure with new AVL — this must see t0 = 12.
            vsetvli(6, 5, vtype(SEW_E8, LMUL_MF8, 0, 0)),
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .collect();

    let ctx = run_program(&wide_inorder_config(), &program, 500);

    assert_eq!(ctx.get_reg(5), 12, "t0 should hold 12");
    assert_eq!(
        ctx.cpu().harts[0].csrs.vl,
        2,
        "vsetvli e8/mf8 with AVL=12 (VLEN=128) saturates to vl=2 (got {})",
        ctx.cpu().harts[0].csrs.vl
    );
    assert_eq!(ctx.get_reg(6), 2, "t1 (vsetvli rd) should equal new vl=2");
}

/// Run the same widening-config sequence at default width=1 as a baseline.
#[test]
fn vsetvli_after_flushing_vsetvli_then_li_width1() {
    let program: Vec<u32> = std::iter::empty()
        .chain([
            vsetvli(6, 0, vtype(SEW_E32, LMUL_M8, 0, 0)),
            addi(5, 0, 12),
            vsetvli(6, 5, vtype(SEW_E8, LMUL_MF8, 0, 0)),
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .collect();

    let ctx = run_program(&Config::default(), &program, 500);

    assert_eq!(ctx.get_reg(5), 12);
    assert_eq!(
        ctx.cpu().harts[0].csrs.vl,
        2,
        "vsetvli e8/mf8 with AVL=12 (VLEN=128) saturates to vl=2 (got {})",
        ctx.cpu().harts[0].csrs.vl
    );
}

/// Encode `vwaddu.vv vd, vs2, vs1` (OPMVV, funct6=0b110000, vm=1).
fn vwaddu_vv(vd: u32, vs2: u32, vs1: u32) -> u32 {
    (0b110000 << 26) | (1 << 25) | (vs2 << 20) | (vs1 << 15) | (0b010 << 12) | (vd << 7) | 0x57
}

/// Encode `vid.v vd` (VMUNARY0 funct6=0b010100 with vs1=0b10001, vm=1).
/// The vs2 field is part of the opcode (must be 0 per spec) and is not a
/// register operand. Optionally pass a non-zero `vs2_field` to verify the
/// implementation tolerates non-canonical encodings (spike does).
fn vid_v(vd: u32, vs2_field: u32) -> u32 {
    (0b010100 << 26)
        | (1 << 25)
        | ((vs2_field & 0x1F) << 20)
        | (0b10001 << 15)
        | (0b010 << 12)
        | (vd << 7)
        | 0x57
}

/// `vwaddu.vv` at fractional LMUL (e8/mf8) must NOT raise an
/// illegal-instruction trap purely because vd happens to be misaligned to a
/// 2-register boundary. With mf8 the doubled EMUL is mf4 — still ≤ 1
/// register — so any vd is legal as long as vd < 32.
///
/// Regression for the chipsalliance widening fails: rvsim's
/// `operand_groups()` was doubling `lmul=1` to `2` for widening even when
/// the source LMUL was fractional, causing vd-alignment checks to reject
/// odd-numbered vd registers like v27.
#[test]
fn vwaddu_vv_at_mf8_does_not_trap_for_odd_vd() {
    let program: Vec<u32> = std::iter::empty()
        .chain([
            addi(5, 0, 2),                                // li t0, 2
            vsetvli(6, 5, vtype(SEW_E8, LMUL_MF8, 0, 0)), // vl=2 at e8/mf8
            vwaddu_vv(27, 26, 1),                         // odd vd=v27, legal at mf8
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .collect();

    let ctx = run_program(&wide_inorder_config(), &program, 500);

    // mcause = 2 (illegal instruction) is the failure mode we're catching.
    let mcause = ctx.cpu().harts[0].csrs.mcause;
    assert_eq!(mcause, 0, "vwaddu.vv at e8/mf8 with vd=v27 must not trap (mcause={:#x})", mcause);
    assert_eq!(ctx.cpu().harts[0].csrs.vl, 2);
}

/// `vid.v` does not read any vector source register — its vs2 field is part
/// of the opcode (the spec requires it to be 0, but spike tolerates other
/// values). The decode-time alignment check must NOT treat the vs2 field as
/// a vector register operand, otherwise a non-canonical (yet harmless)
/// encoding will fault as illegal-instruction.
///
/// Regression for the chipsalliance vid_v-0/-1 fails: rvsim's
/// `operand_groups` was claiming `vs2: lmul` for VIdV, so the alignment
/// check rejected encodings whose vs2 field happened to be non-zero
/// (e.g. test programs setting vs2_field=1 with LMUL=m2).
#[test]
fn vid_v_does_not_check_vs2_alignment() {
    let program: Vec<u32> = std::iter::empty()
        .chain([
            addi(5, 0, 4),                                      // li t0, 4
            vsetvli(6, 5, vtype(SEW_E8, 0b001 /* m2 */, 0, 0)), // vl=4 at e8/m2
            // Non-canonical vs2_field=1 would be misaligned at lmul=m2 if treated as an operand.
            vid_v(12, 1),
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .collect();

    let ctx = run_program(&wide_inorder_config(), &program, 500);

    let mcause = ctx.cpu().harts[0].csrs.mcause;
    assert_eq!(mcause, 0, "vid.v with vs2_field=1 must not trap (mcause={:#x})", mcause);
}

/// Encode a masked-off (`vm = 1`) vector-vector op.
const fn vv(funct6: u32, funct3: u32, vd: u32, vs2: u32, vs1: u32) -> u32 {
    (funct6 << 26) | (1 << 25) | (vs2 << 20) | (vs1 << 15) | (funct3 << 12) | (vd << 7) | 0x57
}

const OPIVV: u32 = 0b000;
const OPMVV: u32 = 0b010;
const VADD: u32 = 0b00_0000;
const VDIV: u32 = 0b10_0001;
const A0: u32 = 10;

/// Cycles until `a0` is set after four dependent `op`s on v1 at e32, m1,
/// vl = 4, on the in-order backend.
fn cycles_through_four(funct6: u32, funct3: u32) -> u64 {
    let program: Vec<u32> = std::iter::empty()
        .chain([addi(5, 0, 4), vsetvli(6, 5, vtype(SEW_E32, LMUL_M1, 0, 0))])
        .chain(std::iter::repeat_n(vv(funct6, funct3, 1, 1, 2), 4))
        .chain([addi(A0, 0, 1), 0x0000_006F])
        .collect();
    let mut ctx =
        TestContext::new_with_config(&wide_inorder_config()).with_memory(RAM_SIZE, RAM_BASE);
    for (i, inst) in program.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(RAM_BASE + (i as u64) * 4), u64::from(*inst), 4);
    }
    ctx.sim.set_pc(0, RAM_BASE);
    for cycle in 1..=2_000 {
        ctx.run(1);
        if ctx.get_reg(A0 as usize) == 1 {
            return cycle;
        }
    }
    panic!("the program did not finish");
}

/// A vector divide's result is ready only after its unit's lane-model
/// latency: `ceil(vl / lanes)` groups of the 20-cycle divider, so 40 cycles
/// at VLEN 128 with two lanes, where a vector add takes two.
#[test]
fn an_inorder_vector_op_waits_out_its_unit_latency() {
    let divides = cycles_through_four(VDIV, OPMVV);
    let adds = cycles_through_four(VADD, OPIVV);

    assert!(divides >= adds + 4 * 38, "four divides took {divides} cycles, four adds {adds}");
}
