//! Privilege and CSR-access rules from the privileged and Zicsr specs.

use crate::config::{BackendKind, Config, Console};
use crate::isa::csr::{MSTATUS_FS, MSTATUS_FS_DIRTY, MSTATUS_FS_INIT};
use crate::isa::privileged::PrivilegeMode;
use crate::tests::support::builder::instruction::{InstructionBuilder, JAL_SELF, trap_recorder};
use crate::tests::support::harness::{PROGRAM_BASE, TestContext, store_words};
use crate::tests::support::open_pmp;
use crate::tests::support::regs::{A1, A2, A3, T1, X0};

const HANDLER: u64 = PROGRAM_BASE + 0x100;
const CYCLE: u32 = 0xC00;
const FFLAGS: u32 = 0x001;
const HPMCOUNTER3: u32 = 0xC03;
const ILLEGAL_INSTRUCTION: u64 = 2;
const RAN: u64 = 99;
/// `sfence.vma x0, x0`.
const SFENCE_VMA: u32 = 0x1200_0073;
/// `c.fld f8, 0(x8)` then `c.nop`: an RV64C encoding of a D instruction.
const C_FLD_THEN_C_NOP: u32 = 0x0001_2000;
const C_FLD: u64 = 0x2000;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

/// What a run left behind: `mcause` and `mtval` as the handler recorded
/// them (zero when nothing trapped) and whether the code after the
/// instruction ran.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Outcome {
    mcause: u64,
    mtval: u64,
    ran_on: bool,
}

fn context(backend: BackendKind, isa: Option<&str>, program: &[u32]) -> TestContext {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    config.isa.string = isa.map(str::to_owned);
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, program);
    store_words(&mut ctx.sim, HANDLER, &trap_recorder());
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx
}

fn run(mut ctx: TestContext) -> Outcome {
    ctx.sim.sync_arch_regs();
    ctx.run(1_000);
    Outcome { mcause: ctx.get_reg(A2), mtval: ctx.get_reg(A3), ran_on: ctx.get_reg(A1) == RAN }
}

/// `inst`, then a marker and a spin, run at `privilege`.
fn run_at(backend: BackendKind, privilege: PrivilegeMode, inst: u32) -> Outcome {
    let i = InstructionBuilder::new;
    let program = [inst, i().addi(A1, X0, RAN as i32).build(), i().jal(X0, 0).build()];
    let mut ctx = context(backend, None, &program);
    let hart = &mut ctx.sim.state.harts[0];
    open_pmp(&mut hart.pmp);
    hart.privilege = privilege;
    run(ctx)
}

#[test]
fn sfence_vma_in_user_mode_is_an_illegal_instruction() {
    let outcomes = BACKENDS.map(|backend| run_at(backend, PrivilegeMode::User, SFENCE_VMA));

    let illegal =
        Outcome { mcause: ILLEGAL_INSTRUCTION, mtval: u64::from(SFENCE_VMA), ran_on: false };
    assert_eq!(outcomes, [illegal.clone(), illegal], "[InOrder, OutOfOrder]");
}

#[test]
fn sfence_vma_in_supervisor_mode_runs() {
    let outcomes = BACKENDS.map(|backend| run_at(backend, PrivilegeMode::Supervisor, SFENCE_VMA));

    assert!(outcomes.iter().all(|o| o.ran_on), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn csrrs_of_a_read_only_csr_with_a_zero_valued_rs1_other_than_x0_is_illegal() {
    let csrrs = InstructionBuilder::new().csrrs(A1, CYCLE, T1).build();

    let mcauses = BACKENDS.map(|backend| run_at(backend, PrivilegeMode::Machine, csrrs).mcause);

    assert_eq!(mcauses, [ILLEGAL_INSTRUCTION; 2], "[InOrder, OutOfOrder]");
}

#[test]
fn csrrs_of_a_read_only_csr_with_rs1_x0_only_reads() {
    let csrrs = InstructionBuilder::new().csrrs(A2, CYCLE, X0).build();

    let outcomes = BACKENDS.map(|backend| run_at(backend, PrivilegeMode::Machine, csrrs));

    assert!(outcomes.iter().all(|o| o.ran_on), "[InOrder, OutOfOrder]: {outcomes:?}");
}

/// `mstatus.FS` after running `csrrs x0, fflags, t1` with `t1 == 0` from
/// FS=Initial.
fn fs_after_zero_valued_csrrs_of_fflags(backend: BackendKind) -> u64 {
    let i = InstructionBuilder::new;
    let csrrs = i().csrrs(X0, FFLAGS, T1).build();
    let program = [csrrs, i().addi(A1, X0, RAN as i32).build(), i().jal(X0, 0).build()];
    let mut ctx = context(backend, None, &program);
    let mstatus = &mut ctx.sim.state.harts[0].csrs.mstatus;
    *mstatus = (*mstatus & !MSTATUS_FS) | MSTATUS_FS_INIT;
    ctx.sim.sync_arch_regs();
    ctx.run_until(1_000, |ctx| ctx.get_reg(A1) == RAN).expect("ran");
    ctx.sim.state.harts[0].csrs.mstatus & MSTATUS_FS
}

#[test]
fn csrrs_with_a_zero_valued_rs1_other_than_x0_still_writes_fflags() {
    let fs = BACKENDS.map(fs_after_zero_valued_csrrs_of_fflags);

    assert_eq!(fs, [MSTATUS_FS_DIRTY; 2], "[InOrder, OutOfOrder]: the write dirties FS");
}

#[test]
fn hpmcounter3_reads_in_machine_mode_when_zihpm_is_advertised() {
    let read = InstructionBuilder::new().csrrs(A2, HPMCOUNTER3, X0).build();

    let outcomes = BACKENDS.map(|backend| run_at(backend, PrivilegeMode::Machine, read));

    assert!(outcomes.iter().all(|o| o.ran_on), "[InOrder, OutOfOrder]: {outcomes:?}");
}

#[test]
fn hpmcounter3_is_illegal_in_user_mode_while_mcounteren_hides_it() {
    let read = InstructionBuilder::new().csrrs(A2, HPMCOUNTER3, X0).build();

    let mcauses = BACKENDS.map(|backend| run_at(backend, PrivilegeMode::User, read).mcause);

    assert_eq!(mcauses, [ILLEGAL_INSTRUCTION; 2], "[InOrder, OutOfOrder]");
}

#[test]
fn mtval_of_an_illegal_compressed_instruction_holds_its_sixteen_bits() {
    let traps = BACKENDS.map(|backend| {
        let outcome = run(context(backend, Some("RV64IMAFC"), &[C_FLD_THEN_C_NOP, JAL_SELF]));
        (outcome.mcause, outcome.mtval)
    });

    assert_eq!(traps, [(ILLEGAL_INSTRUCTION, C_FLD); 2], "[InOrder, OutOfOrder]");
}
