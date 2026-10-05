//! Instructions retired by class, traps, and the cycle counts every core
//! and hart keeps: by privilege mode and by how many instructions retired.

use super::{Recorder, accounting_checks};
use crate::common::PhysAddr;
use crate::config::{BackendKind, Config};
use crate::isa::csr::{MSTATUS_FS, MSTATUS_FS_INIT, MSTATUS_MPP, MSTATUS_VS, MSTATUS_VS_INIT};
use crate::isa::privileged::PrivilegeMode;
use crate::system::simulator::{StopAt, StopReason};
use crate::tests::support::builder::instruction::{ECALL, InstructionBuilder, MRET};
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = PROGRAM_BASE + 0x400;
const DATA: u64 = PROGRAM_BASE + 0x1000;
const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A7: u32 = 17;
const MEPC: u32 = 0x341;
const MTVEC: u32 = 0x305;
const MHARTID: u32 = 0xF14;
const SYS_EXIT: i32 = 93;
const LOOPS: i32 = 5;
/// Independent ops with no branch, so a wide core retires several a cycle
/// whatever its branch predictor.
const STRAIGHT_LINE: u32 = 160;
const BODY: u64 = PROGRAM_BASE + 0x100;
const _: () = assert!(BODY + 4 * (STRAIGHT_LINE as u64 + 1) <= HANDLER);
/// PMP entry 0 covering all memory with read, write and execute.
const PMP_TOR_RWX: u8 = 0b0000_1111;

/// `fld ft1, 0(a1)`.
const FLD: u32 = 0x0005_b087;
/// `fsd ft1, 8(a1)`.
const FSD: u32 = 0x0015_b427;
/// `fadd.d ft2, ft1, ft1`.
const FADD_D: u32 = 0x0210_f153;
/// `fmadd.d ft3, ft1, ft2, ft2`.
const FMADD_D: u32 = 0x1220_f1c3;
/// `fdiv.d ft4, ft2, ft1`.
const FDIV_D: u32 = 0x1a11_7253;
/// `fsqrt.d ft5, ft2`.
const FSQRT_D: u32 = 0x5a01_72d3;
/// `vsetvli t0, zero, e64, m1, ta, ma`.
const VSETVLI: u32 = 0x0d80_72d7;
/// `vle64.v v1, (a1)`.
const VLE64: u32 = 0x0205_f087;
/// `vadd.vv v2, v1, v1`.
const VADD_VV: u32 = 0x0210_8157;
/// `vfadd.vv v3, v1, v1`.
const VFADD_VV: u32 = 0x0210_91d7;
/// `vse64.v v2, (a2)`.
const VSE64: u32 = 0x0206_7127;
/// `vandn.vv v6, v1, v1` (Zvbb).
const VANDN_VV: u32 = 0x0610_8357;
/// `vsetvli t0, zero, e32, m1, ta, ma`.
const VSETVLI_E32: u32 = 0x0d00_72d7;
/// `vaesz.vs v5, v2` (Zvkned).
const VAESZ_VS: u32 = 0xa623_a2f7;

/// A width at which three or more instructions can retire in a cycle.
const WIDE: usize = 4;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

fn config(backend: BackendKind) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = crate::config::Console::Quiet;
    config
}

fn store_words(ctx: &mut TestContext, at: u64, words: &[u32]) {
    for (n, word) in words.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(at + 4 * n as u64), u64::from(*word), 4);
    }
}

/// Loads `program`, and `handler` at `HANDLER` when given, with the FP and
/// vector units on; the guest's exit ecall ends the run.
fn system(backend: BackendKind, program: &[u32], handler: &[u32]) -> TestContext {
    system_with(&config(backend), program, handler)
}

fn system_with(config: &Config, program: &[u32], handler: &[u32]) -> TestContext {
    let mut ctx = TestContext::new_with_config(config).load_program(PROGRAM_BASE, program);
    store_words(&mut ctx, HANDLER, handler);
    let csrs = &mut ctx.sim.state.harts[0].csrs;
    csrs.mstatus = (csrs.mstatus & !(MSTATUS_FS | MSTATUS_VS)) | MSTATUS_FS_INIT | MSTATUS_VS_INIT;
    if !handler.is_empty() {
        csrs.mtvec = HANDLER;
    }
    ctx.set_reg(A1 as usize, DATA);
    ctx.set_reg(A2 as usize, DATA + 0x100);
    ctx.sim.set_direct_mode(true);
    ctx.sim.sync_arch_regs();
    ctx
}

fn run_to_exit(ctx: &mut TestContext, context: &str) {
    let reason = ctx
        .sim
        .run_to(&StopAt { cycles: Some(50_000), ..StopAt::default() })
        .expect("the run ticks");
    assert_eq!(reason, StopReason::Exited(0), "{context}");
}

fn exit_sequence() -> [u32; 3] {
    let i = InstructionBuilder::new;
    [i().addi(A0, 0, 0).build(), i().addi(A7, 0, SYS_EXIT).build(), ECALL]
}

/// One instruction or more of every class, in counts that differ, then the
/// exit; the exit ecall traps, so it does not retire.
fn every_class() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T1, 0, LOOPS).build(),
        i().ld(T2, A1, 0).build(),
        i().addi(T2, T2, 1).build(),
        i().sd(A1, T2, 16).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -16).build(),
        i().jal(0, 4).build(),
        FLD,
        FSD,
        FADD_D,
        FMADD_D,
        FDIV_D,
        FSQRT_D,
        VSETVLI,
        VLE64,
        VADD_VV,
        VFADD_VV,
        VSE64,
        VANDN_VV,
        VSETVLI_E32,
        VAESZ_VS,
        i().amoadd_d(T0, A2, T1).build(),
        i().lr_d(T0, A2).build(),
        i().sc_d(T0, A2, T1).build(),
        i().csrrs(T0, MHARTID, 0).build(),
    ];
    program.extend(exit_sequence());
    program
}

fn retired_instructions_are_counted_by_class(rec: &mut Recorder) {
    let loops = LOOPS as u64;
    let expected = [
        ("core0.commit.op.alu", 1 + 2 * loops + 2),
        ("core0.commit.op.load", loops),
        ("core0.commit.op.store", loops),
        ("core0.commit.op.branch", loops + 1),
        ("core0.commit.op.system", 1),
        ("core0.commit.op.atomic", 3),
        ("core0.commit.fp.load", 1),
        ("core0.commit.fp.store", 1),
        ("core0.commit.fp.arith", 1),
        ("core0.commit.fp.fma", 1),
        ("core0.commit.fp.div_sqrt", 2),
        ("core0.commit.vec.misc", 2),
        ("core0.commit.vec.load", 1),
        ("core0.commit.vec.int", 2),
        ("core0.commit.vec.crypto", 1),
        ("core0.commit.vec.fp", 1),
        ("core0.commit.vec.store", 1),
    ];
    let total: u64 = expected.iter().map(|(_, n)| n).sum();
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut ctx = system(backend, &every_class(), &[]);

        run_to_exit(&mut ctx, &context);

        for (path, count) in expected {
            rec.expect(&ctx.sim, path, count, &context);
        }
        rec.expect(&ctx.sim, "hart0.retired_insts", total, &context);
        rec.expect(&ctx.sim, "system.retired_insts", total, &context);
    }
}

/// Takes three ecalls into a handler that steps over each, then exits.
fn three_handled_ecalls() -> (Vec<u32>, Vec<u32>) {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T1, 0, 3).build(),
        ECALL,
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -8).build(),
        i().csrrw(0, MTVEC, 0).build(),
    ];
    program.extend(exit_sequence());
    let handler = vec![
        i().csrrs(T0, MEPC, 0).build(),
        i().addi(T0, T0, 4).build(),
        i().csrrw(0, MEPC, T0).build(),
        MRET,
    ];
    (program, handler)
}

fn a_trapping_ecall_counts_a_trap_and_does_not_retire(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let (program, handler) = three_handled_ecalls();
        let mut ctx = system(backend, &program, &handler);

        run_to_exit(&mut ctx, &context);

        rec.expect(&ctx.sim, "hart0.traps", 3, &context);
        rec.expect(&ctx.sim, "system.traps", 3, &context);
        rec.expect(&ctx.sim, "core0.commit.op.system", 1 + 3 * 3, &context);
        rec.expect(&ctx.sim, "core0.commit.op.alu", 1 + 3 + 2 + 3, &context);
        rec.expect(&ctx.sim, "core0.commit.op.branch", 3, &context);
        rec.expect(&ctx.sim, "hart0.retired_insts", 22, &context);
    }
}

/// Runs `STRAIGHT_LINE` independent ALU ops in `mode`, entered by an
/// `mret`, then traps back to a machine-mode handler that exits.
fn run_in(backend: BackendKind, width: usize, mode: PrivilegeMode) -> TestContext {
    let i = InstructionBuilder::new;
    let program = [
        i().auipc(T1, 0).build(),
        i().addi(T0, T1, (BODY - PROGRAM_BASE) as i32).build(),
        i().csrrw(0, MEPC, T0).build(),
        MRET,
    ];
    let mut spin: Vec<u32> =
        (0..STRAIGHT_LINE).map(|n| i().addi(T0 + n % 3, 0, n as i32).build()).collect();
    spin.push(ECALL);
    let mut handler = vec![i().csrrw(0, MTVEC, 0).build()];
    handler.extend(exit_sequence());
    let mut config = config(backend);
    config.pipeline.width = width;
    let mut ctx = system_with(&config, &program, &handler);
    store_words(&mut ctx, BODY, &spin);
    let hart = &mut ctx.sim.state.harts[0];
    hart.csrs.mstatus = (hart.csrs.mstatus & !MSTATUS_MPP) | (u64::from(mode.to_u8()) << 11);
    hart.pmp.set_addr(0, u64::MAX >> 10);
    hart.pmp.set_cfg(0, PMP_TOR_RWX);
    ctx
}

fn every_cycle_is_counted_once_by_mode_and_once_by_retire_width(rec: &mut Recorder) {
    for (backend, width) in
        [(BackendKind::InOrder, 1), (BackendKind::OutOfOrder, 1), (BackendKind::OutOfOrder, WIDE)]
    {
        for (mode, user, kernel) in
            [(PrivilegeMode::User, true, false), (PrivilegeMode::Supervisor, false, true)]
        {
            let context = format!("{backend:?} width {width} {mode:?}");
            let mut ctx = run_in(backend, width, mode);

            run_to_exit(&mut ctx, &context);

            let total = rec.read(&ctx.sim, "core0.pipeline.cycles.total");
            let by_mode = [
                rec.read(&ctx.sim, "hart0.cycles.user"),
                rec.read(&ctx.sim, "hart0.cycles.kernel"),
                rec.read(&ctx.sim, "hart0.cycles.machine"),
            ];
            assert_eq!(by_mode.iter().sum::<u64>(), total, "{context}: {by_mode:?}");
            assert_eq!((by_mode[0] > 0, by_mode[1] > 0), (user, kernel), "{context}: {by_mode:?}");
            assert!(by_mode[2] > 0, "{context}");
            let by_width = [
                rec.read(&ctx.sim, "core0.commit.retire_histogram.zero"),
                rec.read(&ctx.sim, "core0.commit.retire_histogram.one"),
                rec.read(&ctx.sim, "core0.commit.retire_histogram.two"),
                rec.read(&ctx.sim, "core0.commit.retire_histogram.three_plus"),
            ];
            assert_eq!(by_width.iter().sum::<u64>(), total, "{context}: {by_width:?}");
            if width == WIDE {
                assert!(by_width[2] > 0 && by_width[3] > 0, "{context}: {by_width:?}");
            }
            let retired = rec.read(&ctx.sim, "hart0.retired_insts");
            let width = width as u64;
            let least = by_width[1] + 2 * by_width[2] + 3 * by_width[3];
            let most = by_width[1] + 2 * by_width[2] + width * by_width[3];
            assert!((least..=most).contains(&retired), "{context}: {retired} vs {by_width:?}");
            let (retired, total) = (retired as f64, total as f64);
            rec.expect_ratio(&ctx.sim, "core0.ipc", retired / total, &context);
            rec.expect_ratio(&ctx.sim, "core0.cpi", total / retired, &context);
        }
    }
}

accounting_checks!(
    retired_instructions_are_counted_by_class,
    a_trapping_ecall_counts_a_trap_and_does_not_retire,
    every_cycle_is_counted_once_by_mode_and_once_by_retire_width,
);
