//! Pipeline-integration regression tests for vector ops.
//!
//! These tests exercise the full Rename → Issue → Execute path in the
//! in-order backend to catch bugs that don't surface in the per-unit tests
//! of `core::units::vpu::*` (which test the compute kernels in isolation).
//!
//! In particular: a `vsetvli` that consumes a value produced by an
//! immediately-preceding scalar instruction must observe the new scalar
//! value, not a stale/zero GPR read.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::Config;
use crate::config::defaults::RAM_BASE;
use crate::isa::privileged::cause::exception;
use crate::isa::reg::RegIdx;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::harness::{TestContext, store_words};
use crate::tests::support::regs::{A0, T0, T1, X0, X5, X6};

/// `jal x0, 0`: holds the program at its end instead of running into zeros.
const SPIN: u32 = 0x0000_006f;

const RAM_SIZE: usize = 4096;

/// A long-enough trailing pad of NOPs (`addi x0, x0, 0`) so the program
/// drains the pipeline before the test inspects CSR state.
const NOP: u32 = 0x0000_0013;

/// Build a Config with a wider in-order pipeline. The original Python brief
/// reproduced the bug at width=4 / InOrder, where the rename-to-issue
/// distance is large enough for forwarding/serialization races to surface.
fn wide_inorder_config() -> Config {
    let mut c = Config::minimal();
    c.pipeline.width = 4;
    c
}

/// Encode `addi rd, rs1, imm` (12-bit signed immediate).
fn addi(rd: RegIdx, rs1: RegIdx, imm: i32) -> u32 {
    let imm = imm.cast_unsigned() & 0xFFF;
    (imm << 20) | (rs1.field() << 15) | (rd.field() << 7) | 0x13
}

/// Encode `vsetvli rd, rs1, vtype` where `zimm` packs SEW/LMUL/TA/MA.
fn vsetvli(rd: RegIdx, rs1: RegIdx, zimm: u32) -> u32 {
    ((zimm & 0x7FF) << 20) | (rs1.field() << 15) | (0b111 << 12) | (rd.field() << 7) | 0x57
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
    let mut ctx = load_program(config, program);
    ctx.run(cycles);
    ctx
}

/// A system with `program` at `RAM_BASE` and the PC on its first instruction.
fn load_program(config: &Config, program: &[u32]) -> TestContext {
    let mut ctx = TestContext::with_fast_memory(config).with_memory(RAM_SIZE, RAM_BASE);
    for (i, inst) in program.iter().enumerate() {
        let addr = RAM_BASE + (i as u64) * 4;
        ctx.sim.probe_mem_store(PhysAddr::new(addr), u64::from(*inst), ProbeWidth::B4);
    }
    ctx.sim.set_pc(0, VirtAddr::new(RAM_BASE));
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
            addi(T0, X0, 4),                                // li t0, 4
            vsetvli(T1, T0, vtype(SEW_E32, LMUL_M1, 0, 0)), // vsetvli t1, t0, e32,m1,tu,mu
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .chain([SPIN])
        .collect();

    let ctx = run_program(&wide_inorder_config(), &program, 500);

    assert_eq!(ctx.get_reg(X5), 4, "t0 should hold 4 after li");
    assert_eq!(
        ctx.state().harts[0].csrs.vl,
        4,
        "vsetvli e32/m1 with AVL=4 should yield vl=4 (got {})",
        ctx.state().harts[0].csrs.vl
    );
    assert_eq!(ctx.get_reg(X6), 4, "t1 (vsetvli rd) should equal new vl");
}

/// Same as above but at the default single-issue width (1). Establishes a
/// baseline: if this passes and the wide variant fails, the bug is
/// width-specific (intra-bundle hazards, fetch-group serialization, etc.).
#[test]
fn vsetvli_consumes_immediately_preceding_li_width1() {
    let program: Vec<u32> = std::iter::empty()
        .chain([addi(T0, X0, 4), vsetvli(T1, T0, vtype(SEW_E32, LMUL_M1, 0, 0))])
        .chain(std::iter::repeat_n(NOP, 32))
        .chain([SPIN])
        .collect();

    let ctx = run_program(&Config::minimal(), &program, 500);

    assert_eq!(ctx.get_reg(X5), 4);
    assert_eq!(ctx.state().harts[0].csrs.vl, 4, "got vl={}", ctx.state().harts[0].csrs.vl);
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
            vsetvli(T1, X0, vtype(SEW_E32, LMUL_M8, 0, 0)),
            // Set up new AVL.
            addi(T0, X0, 12),
            // Reconfigure with new AVL — this must see t0 = 12.
            vsetvli(T1, T0, vtype(SEW_E8, LMUL_MF8, 0, 0)),
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .chain([SPIN])
        .collect();

    let ctx = run_program(&wide_inorder_config(), &program, 500);

    assert_eq!(ctx.get_reg(X5), 12, "t0 should hold 12");
    assert_eq!(
        ctx.state().harts[0].csrs.vl,
        2,
        "vsetvli e8/mf8 with AVL=12 (VLEN=128) saturates to vl=2 (got {})",
        ctx.state().harts[0].csrs.vl
    );
    assert_eq!(ctx.get_reg(X6), 2, "t1 (vsetvli rd) should equal new vl=2");
}

/// Run the same widening-config sequence at default width=1 as a baseline.
#[test]
fn vsetvli_after_flushing_vsetvli_then_li_width1() {
    let program: Vec<u32> = std::iter::empty()
        .chain([
            vsetvli(T1, X0, vtype(SEW_E32, LMUL_M8, 0, 0)),
            addi(T0, X0, 12),
            vsetvli(T1, T0, vtype(SEW_E8, LMUL_MF8, 0, 0)),
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .chain([SPIN])
        .collect();

    let ctx = run_program(&Config::minimal(), &program, 500);

    assert_eq!(ctx.get_reg(X5), 12);
    assert_eq!(
        ctx.state().harts[0].csrs.vl,
        2,
        "vsetvli e8/mf8 with AVL=12 (VLEN=128) saturates to vl=2 (got {})",
        ctx.state().harts[0].csrs.vl
    );
}

/// Encode `vwaddu.vv vd, vs2, vs1` (OPMVV, funct6=0b110000, vm=1).
fn vwaddu_vv(vd: u32, vs2: u32, vs1: u32) -> u32 {
    (0b110000 << 26) | (1 << 25) | (vs2 << 20) | (vs1 << 15) | (0b010 << 12) | (vd << 7) | 0x57
}

/// Encode `vid.v vd` (VMUNARY0 funct6=0b010100 with vs1=0b10001, vm=1).
/// The vs2 field is part of the opcode and must be 0; any other
/// `vs2_field` makes a reserved encoding.
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
            addi(T0, X0, 2),                                // li t0, 2
            vsetvli(T1, T0, vtype(SEW_E8, LMUL_MF8, 0, 0)), // vl=2 at e8/mf8
            vwaddu_vv(27, 26, 1),                           // odd vd=v27, legal at mf8
        ])
        .chain(std::iter::repeat_n(NOP, 32))
        .chain([SPIN])
        .collect();

    let ctx = run_program(&wide_inorder_config(), &program, 500);

    // mcause = 2 (illegal instruction) is the failure mode we're catching.
    let mcause = ctx.state().harts[0].csrs.mcause;
    assert_eq!(mcause, 0, "vwaddu.vv at e8/mf8 with vd=v27 must not trap (mcause={:#x})", mcause);
    assert_eq!(ctx.state().harts[0].csrs.vl, 2);
}

/// `vid.v`'s vs2 field is part of its opcode: spike's decoder fixes it at
/// zero (`MASK_VID_V`), so any other value is an illegal instruction.
#[test]
fn vid_v_with_its_vs2_field_set_is_illegal() {
    let program: Vec<u32> = std::iter::empty()
        .chain([
            addi(T0, X0, 4),                                      // li t0, 4
            vsetvli(T1, T0, vtype(SEW_E8, 0b001 /* m2 */, 0, 0)), // vl=4 at e8/m2
            vid_v(12, 1),
            SPIN,
        ])
        .collect();
    let vid_v_address = RAM_BASE + 2 * 4;
    let mut ctx = load_program(&wide_inorder_config(), &program);
    ctx.sim.state.harts[0].csrs.mtvec = vid_v_address + 4;

    ctx.run(500);

    let csrs = &ctx.state().harts[0].csrs;
    assert_eq!((csrs.mcause, csrs.mepc), (exception::ILLEGAL_INSTRUCTION, vid_v_address));
}

/// Encode a masked-off (`vm = 1`) vector-vector op.
const fn vv(funct6: u32, funct3: u32, vd: u32, vs2: u32, vs1: u32) -> u32 {
    (funct6 << 26) | (1 << 25) | (vs2 << 20) | (vs1 << 15) | (funct3 << 12) | (vd << 7) | 0x57
}

const OPIVV: u32 = 0b000;
const OPMVV: u32 = 0b010;
const VADD: u32 = 0b00_0000;
const VDIV: u32 = 0b10_0001;

/// Cycles until `a0` is set after four dependent `op`s on v1 at e32, m1,
/// vl = 4, on the in-order backend.
fn cycles_through_four(funct6: u32, funct3: u32) -> u64 {
    let program: Vec<u32> = std::iter::empty()
        .chain([addi(T0, X0, 4), vsetvli(T1, T0, vtype(SEW_E32, LMUL_M1, 0, 0))])
        .chain(std::iter::repeat_n(vv(funct6, funct3, 1, 1, 2), 4))
        .chain([addi(A0, X0, 1), 0x0000_006F])
        .collect();
    let mut ctx =
        TestContext::with_fast_memory(&wide_inorder_config()).with_memory(RAM_SIZE, RAM_BASE);
    store_words(&mut ctx.sim, RAM_BASE, &program);
    ctx.sim.set_pc(0, VirtAddr::new(RAM_BASE));
    for cycle in 1..=2_000 {
        ctx.run(1);
        if ctx.get_reg(A0) == 1 {
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
