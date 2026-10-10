//! On the out-of-order backend an instruction that must be the oldest
//! learns that it is a cycle after the instruction ahead of it retires: the
//! retirement is registered before the issue logic sees it, so the two
//! never happen in the same cycle. The in-order backend issues a CSR read
//! in order and performs it when it retires.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const T0: u32 = 5;
const T1: u32 = 6;
const PROGRAM_BASE: u64 = 0x8000_0000;
const MSCRATCH: u32 = 0x340;
const ADDS: u64 = 8;

/// Adds, then a CSR read.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> = (0..ADDS).map(|_| i().addi(T0, T0, 1).build()).collect();
    program.push(i().csrrs(T1, MSCRATCH, 0).build());
    program.push(i().jal(0, 0).build());
    program
}

/// The cycle the last add retired, if it had when the CSR read left the
/// issue queue, and the cycle the read left it.
fn retire_and_issue_cycles(backend: BackendKind) -> (Option<u64>, u64) {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 1;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    let csr_pc = PROGRAM_BASE + 4 * ADDS;
    let (mut last_add_retired, mut queued) = (None, false);

    for _ in 0..500 {
        ctx.run(1);
        let cycle = ctx.sim.state.cycle;
        if last_add_retired.is_none() && ctx.sim.state.harts[0].instructions_retired == ADDS {
            last_add_retired = Some(cycle);
        }
        let snapshot = ctx.sim.state.cores[0].pipeline.snapshot(1);
        let in_queue = snapshot.issue_queue.iter().any(|e| e.inst.pc == csr_pc);
        if queued && !in_queue {
            return (last_add_retired, cycle);
        }
        queued |= in_queue;
    }
    panic!("{backend:?}: the CSR read never issued");
}

#[test]
fn a_csr_read_issues_before_the_older_instructions_retire_inorder() {
    let (retired, _) = retire_and_issue_cycles(BackendKind::InOrder);

    assert_eq!(retired, None);
}

#[test]
fn a_csr_read_issues_the_cycle_after_the_last_older_instruction_retires_o3() {
    let (retired, issued) = retire_and_issue_cycles(BackendKind::OutOfOrder);

    assert_eq!(Some(issued), retired.map(|cycle| cycle + 1));
}
