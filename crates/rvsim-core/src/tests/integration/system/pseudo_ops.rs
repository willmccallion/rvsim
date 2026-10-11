//! Guest software marks its region of interest with the pseudo-instruction:
//! it resets the stats, dumps them labelled, and ends the run.

use crate::config::Config;
use crate::isa::op::pseudo::PseudoOp;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const COUNTER: u32 = 7;
const LOOPS: i32 = 50;
const A0: u32 = 10;
const A1: u32 = 11;
const M5_FAIL: u32 = 0x22;
const M5_RESET_STATS: u32 = 0x40;
const M5_DUMP_STATS: u32 = 0x41;

/// What the guest asks for before the loop.
#[derive(Clone, Copy)]
enum Before {
    Reset,
    Dump { label: i32 },
}

/// Pushes the pseudo-operation `op` with `a0` and `a1` set to `label` and
/// `code`.
fn request(program: &mut Vec<u32>, op: u32, label: i32, code: i32) {
    let i = InstructionBuilder::new;
    program.push(i().addi(A0, 0, label).build());
    program.push(i().addi(A1, 0, code).build());
    program.push(PseudoOp::encode(op));
}

/// Warms up, then runs a counted loop between `before` and a dump
/// labelled 42, then exits with code 5.
fn marked_program(before: Before) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = Vec::new();
    for _ in 0..40 {
        program.push(i().addi(0, 0, 0).build());
    }
    match before {
        Before::Reset => request(&mut program, M5_RESET_STATS, 0, 0),
        Before::Dump { label } => request(&mut program, M5_DUMP_STATS, label, 0),
    }
    program.push(i().addi(COUNTER, 0, LOOPS).build());
    program.push(i().addi(COUNTER, COUNTER, -1).build());
    program.push(i().bne(COUNTER, 0, -4).build());
    request(&mut program, M5_DUMP_STATS, 42, 0);
    request(&mut program, M5_FAIL, 0, 5);
    program.push(i().jal(0, 0).build());
    program
}

fn run(program: &[u32]) -> TestContext {
    let mut config = Config::default();
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, program);
    ctx.run(20_000);
    assert_eq!(ctx.sim.state.check_exit(), Some(5), "the guest ended the run with its code");
    ctx
}

/// Whether `instructions` is the loop plus the few marker instructions.
fn is_the_loop(instructions: u64) -> bool {
    let loop_instructions = 2 * LOOPS as u64;
    (loop_instructions..loop_instructions + 20).contains(&instructions)
}

#[test]
fn two_dumps_subtract_to_the_region_between_them_and_keep_the_whole_run() {
    let ctx = run(&marked_program(Before::Dump { label: 1 }));

    let dumps = &ctx.sim.state.stats_dumps;
    assert_eq!(dumps.iter().map(|d| d.label).collect::<Vec<_>>(), vec![1, 42]);
    let region = dumps[1].instructions_retired - dumps[0].instructions_retired;
    assert!(is_the_loop(region), "the region holds the loop: {region}");
    let alu_ops = ctx.sim.state.cores[0].units.stat_paths.commit.op_alu;
    let (start, end) = (&dumps[0].stats, &dumps[1].stats);
    let counted = |stats: &crate::sim::stats::Stats| stats.get(alu_ops).unwrap_or(0.0);
    assert_eq!(counted(&end.since(start)), counted(end) - counted(start));
    assert!(counted(start) > 0.0, "the whole run's counts are kept");
}

#[test]
fn a_guest_reset_starts_the_dumped_window_at_the_region() {
    let ctx = run(&marked_program(Before::Reset));

    let dumps = &ctx.sim.state.stats_dumps;
    assert_eq!(dumps.len(), 1);
    assert_eq!(dumps[0].label, 42);
    assert!(is_the_loop(dumps[0].instructions_retired), "{}", dumps[0].instructions_retired);
    assert!(dumps[0].cycles < ctx.sim.state.cycle, "the window starts after the run did");
}
