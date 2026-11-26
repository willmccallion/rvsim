//! Out-of-order rename takes physical registers and rename-map checkpoints
//! from the counts as they stood at the start of the cycle, less what it has
//! taken since, as it takes ROB and queue entries: what the oldest
//! instruction frees by retiring goes to a younger one the cycle after, not
//! the same cycle.
//!
//! The single-wide tests have one instruction wait for one freed resource.
//! The two-wide ones rename two a cycle but retire one: one resource is free
//! as a cycle starts and commit frees another during it, so only the first
//! of two waiting instructions may take one, the second a cycle later.

use crate::common::VirtAddr;
use crate::config::{BackendKind, Config};
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{T0, T1, T2, X0, X5, X6, X7, X28};

/// One more physical register than architectural ones, so a write waits
/// for the register the write before it frees.
const ONE_SPARE: usize = RegIdx::COUNT + 1;
const TWO_SPARE: usize = RegIdx::COUNT + 2;
/// The fewest vector physical registers a core may have: eight spare.
const LEAST_VECTOR_REGISTERS: usize = 40;

/// `fmv.d.x f1, x0` and `fmv.d.x f2, x0`.
const FMV_D_X_F1: u32 = 0xf200_00d3;
const FMV_D_X_F2: u32 = 0xf200_0153;
const FMV_D_X_F3: u32 = 0xf200_01d3;
const FMV_D_X_F4: u32 = 0xf200_0253;
/// `vsetivli zero, 2, e64, m1, ta, ma`.
const VSETIVLI: u32 = 0xcd81_7057;
/// `vmv.v.x v1, t2`.
const VMV_V1_FROM_T2: u32 = 0x5e03_c0d7;
/// `vmv.v.i v0, 0`, with the destination to fill in.
const VMV_V_I_ZERO: u32 = 0x5e00_3057;

/// A program in which the instruction at `waiter` needs what the first
/// `freer` instructions free as they retire.
struct Case {
    config: Config,
    program: Vec<u32>,
    freer: u64,
    waiter: u64,
}

impl Case {
    fn out_of_order(program: Vec<u32>, freer: u64, waiter: u64) -> Self {
        let mut config = Config::minimal();
        config.pipeline.backend = BackendKind::OutOfOrder;
        Self { config, program, freer, waiter }
    }

    /// An out-of-order core that renames two instructions a cycle but
    /// retires one.
    fn renaming_two_retiring_one(program: Vec<u32>, freer: u64, waiter: u64) -> Self {
        let mut case = Self::out_of_order(program, freer, waiter);
        case.config.pipeline.width = 2;
        case.config.pipeline.commit_width = Some(1);
        case
    }

    /// The cycle the freeing instruction retired and the cycle the waiter
    /// was renamed.
    fn freed_and_renamed(&self) -> (u64, u64) {
        let mut program = self.program.clone();
        program.push(InstructionBuilder::new().jal(X0, 0).build());
        let mut ctx =
            TestContext::with_fast_memory(&self.config).load_program(PROGRAM_BASE, &program);
        let waiter_pc = VirtAddr::new(PROGRAM_BASE + 4 * self.waiter);
        let mut freed = None;

        for _ in 0..500 {
            ctx.run(1);
            let cycle = ctx.sim.state.uncore.kernel.cycle;
            if freed.is_none() && ctx.sim.state.harts[0].instructions_retired >= self.freer {
                freed = Some(cycle);
            }
            let snapshot = ctx.sim.state.cores[0].pipeline.snapshot(1);
            let renamed = snapshot.rename_issue.iter().any(|slot| slot.inst.pc == waiter_pc)
                || snapshot.issue_queue.iter().any(|slot| slot.inst.pc == waiter_pc);
            if renamed {
                return (freed.expect("the waiter waited for what retiring freed"), cycle);
            }
        }
        panic!("the waiter was never renamed");
    }
}

#[test]
fn a_write_takes_a_retired_writes_integer_register_the_cycle_after_it_frees() {
    let i = InstructionBuilder::new;
    let program = vec![i().addi(T0, X0, 1).build(), i().addi(T1, X0, 2).build()];
    let mut case = Case::out_of_order(program, 1, 1);
    case.config.pipeline.prf_gpr_size = ONE_SPARE;

    let (freed, renamed) = case.freed_and_renamed();

    assert_eq!(renamed, freed + 1);
}

#[test]
fn a_write_takes_a_retired_writes_fp_register_the_cycle_after_it_frees() {
    let mut case = Case::out_of_order(vec![FMV_D_X_F1, FMV_D_X_F2], 1, 1);
    case.config.pipeline.prf_fpr_size = ONE_SPARE;

    let (freed, renamed) = case.freed_and_renamed();

    assert_eq!(renamed, freed + 1);
}

/// The first vector write waits on a divide, so the eight after it take
/// every spare vector register and the ninth waits for it to retire.
#[test]
fn a_write_takes_a_retired_writes_vector_register_the_cycle_after_it_frees() {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T0, X0, 100).build(),
        i().addi(T1, X0, 7).build(),
        i().div(T2, T0, T1).build(),
        VSETIVLI,
        VMV_V1_FROM_T2,
    ];
    let first_vector_write = program.len() as u64;
    program.extend((2..=9).map(|vd| VMV_V_I_ZERO | vd << 7));
    let ninth = program.len() as u64 - 1;
    let mut case = Case::out_of_order(program, first_vector_write, ninth);
    case.config.pipeline.prf_vpr_size = LEAST_VECTOR_REGISTERS;

    let (freed, renamed) = case.freed_and_renamed();

    assert_eq!(renamed, freed + 1);
}

#[test]
fn a_branch_takes_a_retired_branchs_checkpoint_the_cycle_after_it_frees() {
    let i = InstructionBuilder::new;
    let not_taken = i().bne(X0, X0, 8).build();
    let mut case = Case::out_of_order(vec![not_taken, not_taken], 1, 1);
    case.config.pipeline.checkpoint_count = 1;

    let (freed, renamed) = case.freed_and_renamed();

    assert_eq!(renamed, freed + 1);
}

/// Four writes on two spare integer registers: the third takes the one the
/// first frees, and the fourth the one the second frees a cycle later.
#[test]
fn the_second_of_two_waiting_writes_takes_an_integer_register_a_cycle_later() {
    let i = InstructionBuilder::new;
    let program =
        [X5, X6, X7, X28].iter().map(|&rd| i().addi(rd, X0, 1).build()).collect::<Vec<_>>();
    let mut case = Case::renaming_two_retiring_one(program, 2, 3);
    case.config.pipeline.prf_gpr_size = TWO_SPARE;

    let (freed, renamed) = case.freed_and_renamed();

    assert_eq!(renamed, freed + 1);
}

#[test]
fn the_second_of_two_waiting_writes_takes_an_fp_register_a_cycle_later() {
    let program = vec![FMV_D_X_F1, FMV_D_X_F2, FMV_D_X_F3, FMV_D_X_F4];
    let mut case = Case::renaming_two_retiring_one(program, 2, 3);
    case.config.pipeline.prf_fpr_size = TWO_SPARE;

    let (freed, renamed) = case.freed_and_renamed();

    assert_eq!(renamed, freed + 1);
}

/// Ten vector writes on eight spare vector registers, the first waiting on
/// a divide: the ninth takes the register the first frees, and the tenth
/// the one the second frees a cycle later.
#[test]
fn the_second_of_two_waiting_writes_takes_a_vector_register_a_cycle_later() {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T0, X0, 100).build(),
        i().addi(T1, X0, 7).build(),
        i().div(T2, T0, T1).build(),
        VSETIVLI,
        VMV_V1_FROM_T2,
    ];
    let second_vector_write = program.len() as u64 + 1;
    program.extend((2..=10).map(|vd| VMV_V_I_ZERO | vd << 7));
    let tenth = program.len() as u64 - 1;
    let mut case = Case::renaming_two_retiring_one(program, second_vector_write, tenth);
    case.config.pipeline.prf_vpr_size = LEAST_VECTOR_REGISTERS;

    let (freed, renamed) = case.freed_and_renamed();

    assert_eq!(renamed, freed + 1);
}

/// Four branches on two checkpoints, the first waiting on a divide: the
/// third takes the checkpoint the first frees, and the fourth the one the
/// second frees a cycle later. The leading `addi`s put the third and fourth
/// in one two-wide bundle, so both reach rename in the same cycle.
#[test]
fn the_second_of_two_waiting_branches_takes_a_checkpoint_a_cycle_later() {
    let i = InstructionBuilder::new;
    let not_taken = i().bne(X0, X0, 8).build();
    let program = vec![
        i().addi(T0, X0, 100).build(),
        i().addi(T1, X0, 7).build(),
        i().addi(X5, X0, 0).build(),
        i().div(T2, T0, T1).build(),
        i().bne(T2, T2, 8).build(),
        not_taken,
        not_taken,
        not_taken,
    ];
    let mut case = Case::renaming_two_retiring_one(program, 6, 7);
    case.config.pipeline.checkpoint_count = 2;

    let (freed, renamed) = case.freed_and_renamed();

    assert_eq!(renamed, freed + 1);
}
