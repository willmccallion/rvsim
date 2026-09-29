//! `mstatus.FS` and `mstatus.VS` gate their units: with the field Off, an
//! instruction or CSR access that needs the unit is illegal; with it on,
//! the same instruction runs.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::arch::csr::{MSTATUS_FS, MSTATUS_FS_INIT, MSTATUS_VS, MSTATUS_VS_INIT};
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x100;
const DATA: u64 = PROGRAM_BASE + 0x200;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A4: u32 = 14;
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
    vec![i().csrrs(A2, MCAUSE, 0).build(), i().csrrs(A3, MTVAL, 0).build(), i().jal(0, 0).build()]
}

/// Runs `inst` followed by a marker write; returns `(mcause, mtval, marker)`.
fn run(backend: BackendType, units: Units, inst: u32) -> (u64, u64, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = rvsim_core::config::Console::Quiet;
    let i = InstructionBuilder::new;
    let program = [inst, i().addi(A4, 0, 1).build(), i().jal(0, 0).build()];
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    for (n, word) in handler().iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + 4 * n as u64), u64::from(*word), 4);
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
    csrs.vtype = VTYPE_E32_M1;
    csrs.vl = 4;
    ctx.set_reg(A0 as usize, DATA);
    ctx.sim.state.direct_mode = false;
    ctx.sim.sync_arch_regs();

    ctx.run(1_000);
    (ctx.get_reg(A2 as usize), ctx.get_reg(A3 as usize), ctx.get_reg(A4 as usize))
}

/// `inst` is illegal with `off` and runs to the marker with both units on.
fn check(inst: u32, off: Units) {
    for backend in [BackendType::InOrder, BackendType::OutOfOrder] {
        let (mcause, mtval, marker) = run(backend, off, inst);
        assert_eq!(mcause, ILLEGAL_INSTRUCTION, "{backend:?} {inst:#010x} {off:?}: illegal");
        assert_eq!(mtval, u64::from(inst), "{backend:?} {inst:#010x}: tval holds the instruction");
        assert_eq!(marker, 0, "{backend:?} {inst:#010x}: nothing after the fault retired");

        let (mcause, _, marker) = run(backend, BOTH_ON, inst);
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
    check(InstructionBuilder::new().csrrs(A1, FCSR, 0).build(), Units { fp: false, vector: true });
}

#[test]
fn vector_csr_access_is_illegal_with_vs_off() {
    check(InstructionBuilder::new().csrrs(A1, VL, 0).build(), Units { fp: true, vector: false });
}
