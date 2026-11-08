//! `mstatus.FS` and `mstatus.VS` gate their units: with the field Off, an
//! instruction or CSR access that needs the unit is illegal; with it on,
//! the same instruction runs. Likewise an instruction that rounds by `frm`
//! is illegal while `frm` holds a reserved mode.

use crate::common::PhysAddr;
use crate::config::BackendKind;
use crate::config::Config;
use crate::isa::csr::{MSTATUS_FS, MSTATUS_FS_INIT, MSTATUS_VS, MSTATUS_VS_INIT};
use crate::isa::fp::Frm;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::tests::support::regs::{A0, A1, A2, A3, A4, X0};

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const DATA: u64 = PROGRAM_BASE + 0x200;
const MCAUSE: u32 = 0x342;
const MTVAL: u32 = 0x343;
const FCSR: u32 = 0x003;
const VL: u32 = 0xC20;
const ILLEGAL_INSTRUCTION: u64 = 2;
/// e32, m1.
const VTYPE_E32_M1: u64 = 0x10;
/// `vadd.vv v1, v2, v3`.
const VADD_VV: u32 = 0x0221_80D7;
/// `vfadd.vv v1, v2, v3`.
const VFADD_VV: u32 = 0x0221_90D7;
/// `vle8.v v1, (a0)`.
const VLE8_V: u32 = 0x0205_0087;
/// `fadd.s f1, f2, f3, dyn`.
const FADD_S_DYN: u32 = 0x0031_70D3;
/// `fadd.s f1, f2, f3, rne`.
const FADD_S_RNE: u32 = 0x0031_00D3;
/// `fsgnj.s f1, f2, f3`, which does not round.
const FSGNJ_S: u32 = 0x2031_00D3;
/// `frm` value for round to nearest, ties to even.
const FRM_RNE: u64 = 0;
/// The `frm` values the F extension leaves reserved.
const RESERVED_FRMS: [u64; 3] = [5, 6, 7];

/// Which units `mstatus` has on.
#[derive(Clone, Copy, Debug)]
struct Units {
    fp: bool,
    vector: bool,
}

const BOTH_ON: Units = Units { fp: true, vector: true };

/// Records `mcause` and `mtval`, then spins.
fn handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().csrrs(A2, MCAUSE, X0).build(),
        i().csrrs(A3, MTVAL, X0).build(),
        i().jal(X0, 0).build(),
    ]
}

/// Runs `inst` followed by a marker write; returns `(mcause, mtval, marker)`.
fn run(backend: BackendKind, units: Units, frm: u64, inst: u32) -> (u64, u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    let i = InstructionBuilder::new;
    let program = [inst, i().addi(A4, X0, 1).build(), i().jal(X0, 0).build()];
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &program);
    for (n, word) in handler().iter().enumerate() {
        ctx.sim.probe_mem_store(
            PhysAddr::new(HANDLER + 4 * n as u64),
            u64::from(*word),
            ProbeWidth::B4,
        );
    }
    let csrs = &mut ctx.sim.state.harts[0].csrs;
    csrs.mtvec = HANDLER;
    csrs.mstatus &= !(MSTATUS_FS | MSTATUS_VS);
    if units.fp {
        csrs.mstatus |= MSTATUS_FS_INIT;
    }
    if units.vector {
        csrs.mstatus |= MSTATUS_VS_INIT;
    }
    csrs.frm = Frm::from_bits(frm);
    csrs.vtype = VTYPE_E32_M1;
    csrs.vl = 4;
    ctx.set_reg(A0, DATA);
    ctx.sim.sync_arch_regs();

    ctx.run(1_000);
    (ctx.get_reg(A2), ctx.get_reg(A3), ctx.get_reg(A4))
}

/// `inst` is illegal with `off` and runs to the marker with both units on.
fn check(inst: u32, off: Units) {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let (mcause, mtval, marker) = run(backend, off, FRM_RNE, inst);
        assert_eq!(mcause, ILLEGAL_INSTRUCTION, "{backend:?} {inst:#010x} {off:?}: illegal");
        assert_eq!(mtval, u64::from(inst), "{backend:?} {inst:#010x}: tval holds the instruction");
        assert_eq!(marker, 0, "{backend:?} {inst:#010x}: nothing after the fault retired");

        let (mcause, _, marker) = run(backend, BOTH_ON, FRM_RNE, inst);
        assert_eq!(mcause, 0, "{backend:?} {inst:#010x}: legal with both units on");
        assert_eq!(marker, 1, "{backend:?} {inst:#010x}: ran on with both units on");
    }
}

#[test]
fn vector_arithmetic_is_illegal_with_vs_off() {
    check(VADD_VV, Units { fp: true, vector: false });
}

#[test]
fn vector_load_is_illegal_with_vs_off() {
    check(VLE8_V, Units { fp: true, vector: false });
}

#[test]
fn vector_floating_point_is_illegal_with_fs_off() {
    check(VFADD_VV, Units { fp: false, vector: true });
}

#[test]
fn fp_csr_access_is_illegal_with_fs_off() {
    check(InstructionBuilder::new().csrrs(A1, FCSR, X0).build(), Units { fp: false, vector: true });
}

#[test]
fn vector_csr_access_is_illegal_with_vs_off() {
    check(InstructionBuilder::new().csrrs(A1, VL, X0).build(), Units { fp: true, vector: false });
}

/// `inst` is illegal with every reserved `frm` and runs with `frm` = RNE.
fn check_reserved_frm_is_illegal(inst: u32) {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        for frm in RESERVED_FRMS {
            let (mcause, mtval, marker) = run(backend, BOTH_ON, frm, inst);
            assert_eq!(mcause, ILLEGAL_INSTRUCTION, "{backend:?} {inst:#010x} frm={frm}: illegal");
            assert_eq!(
                mtval,
                u64::from(inst),
                "{backend:?} {inst:#010x}: tval holds the instruction"
            );
            assert_eq!(marker, 0, "{backend:?} {inst:#010x}: nothing after the fault retired");
        }
        let (mcause, _, marker) = run(backend, BOTH_ON, FRM_RNE, inst);
        assert_eq!((mcause, marker), (0, 1), "{backend:?} {inst:#010x}: runs with frm=RNE");
    }
}

/// `inst` runs with every reserved `frm`.
fn check_reserved_frm_is_ignored(inst: u32) {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        for frm in RESERVED_FRMS {
            let (mcause, _, marker) = run(backend, BOTH_ON, frm, inst);
            assert_eq!((mcause, marker), (0, 1), "{backend:?} {inst:#010x} frm={frm}: runs");
        }
    }
}

#[test]
fn dynamic_rounding_is_illegal_with_a_reserved_frm() {
    check_reserved_frm_is_illegal(FADD_S_DYN);
}

#[test]
fn vector_floating_point_is_illegal_with_a_reserved_frm() {
    check_reserved_frm_is_illegal(VFADD_VV);
}

#[test]
fn static_rounding_ignores_a_reserved_frm() {
    check_reserved_frm_is_ignored(FADD_S_RNE);
}

#[test]
fn an_op_that_does_not_round_ignores_a_reserved_frm() {
    check_reserved_frm_is_ignored(FSGNJ_S);
}
