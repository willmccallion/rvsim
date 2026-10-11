//! Guest software marks its region of interest through the sim-control
//! device or the pseudo-instruction: it resets the stats, dumps them
//! labelled, and ends the run.

use crate::config::Config;
use crate::isa::op::pseudo::PseudoOp;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const SIM_CONTROL: u32 = 5;
const VALUE: u32 = 6;
const COUNTER: u32 = 7;
const LOOPS: i32 = 50;
const A0: u32 = 10;
const A1: u32 = 11;
const M5_FAIL: u32 = 0x22;
const M5_RESET_STATS: u32 = 0x40;
const M5_DUMP_STATS: u32 = 0x41;

/// How the guest asks: through the device's registers, or with the
/// pseudo-instruction.
#[derive(Clone, Copy)]
enum Mechanism {
    Device,
    Instruction,
}

/// A command (the device's number) with `arg`, by either mechanism.
fn request(program: &mut Vec<u32>, mechanism: Mechanism, arg: i32, command: i32) {
    match mechanism {
        Mechanism::Device => device_command(program, arg, command),
        Mechanism::Instruction => {
            let i = InstructionBuilder::new;
            let (op, label, code) = match command {
                1 => (M5_RESET_STATS, arg, 0),
                2 => (M5_DUMP_STATS, arg, 0),
                3 => (M5_FAIL, 0, arg),
                other => panic!("no pseudo-op for command {other}"),
            };
            program.push(i().addi(A0, 0, label).build());
            program.push(i().addi(A1, 0, code).build());
            program.push(PseudoOp::encode(op));
        }
    }
}

fn device_command(program: &mut Vec<u32>, arg: i32, command: i32) {
    let i = InstructionBuilder::new;
    program.push(i().addi(VALUE, 0, arg).build());
    program.push(i().sd(SIM_CONTROL, VALUE, 8).build());
    program.push(i().addi(VALUE, 0, command).build());
    program.push(i().sd(SIM_CONTROL, VALUE, 0).build());
}

/// Warms up, then runs a counted loop between guest commands `before`
/// (with `before_arg`) and a dump labelled 42, then exits with code 5.
fn marked_program(mechanism: Mechanism, before: i32, before_arg: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().lui(SIM_CONTROL, 0x102).build()];
    for _ in 0..40 {
        program.push(i().addi(0, 0, 0).build());
    }
    request(&mut program, mechanism, before_arg, before);
    program.push(i().addi(COUNTER, 0, LOOPS).build());
    program.push(i().addi(COUNTER, COUNTER, -1).build());
    program.push(i().bne(COUNTER, 0, -4).build());
    request(&mut program, mechanism, 42, 2);
    request(&mut program, mechanism, 5, 3);
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
    for mechanism in [Mechanism::Device, Mechanism::Instruction] {
        assert_two_dumps_bracket_the_loop(&run(&marked_program(mechanism, 2, 1)));
    }
}

fn assert_two_dumps_bracket_the_loop(ctx: &TestContext) {
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
    for mechanism in [Mechanism::Device, Mechanism::Instruction] {
        assert_the_window_is_the_region(&run(&marked_program(mechanism, 1, 0)));
    }
}

fn assert_the_window_is_the_region(ctx: &TestContext) {
    let dumps = &ctx.sim.state.stats_dumps;
    assert_eq!(dumps.len(), 1);
    assert_eq!(dumps[0].label, 42);
    assert!(is_the_loop(dumps[0].instructions_retired), "{}", dumps[0].instructions_retired);
    assert!(dumps[0].cycles < ctx.sim.state.cycle, "the window starts after the run did");
}
