//! ROB tags wrap after `u32::MAX` allocations. Every structure orders its
//! entries by age, so where the tags start must not change what a program
//! computes, how many cycles it takes, or any statistic.

use crate::config::BackendKind;
use crate::config::Config;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;
use crate::uarch::pipeline::rob::RobTag;

const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const S0: u32 = 8;
const S1: u32 = 9;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A4: u32 = 14;
const A5: u32 = 15;
const A6: u32 = 16;
const S2: u32 = 18;
const PROGRAM_BASE: u64 = 0x8000_0000;
const ITERATIONS: i32 = 24;

/// A loop whose divide produces the address that four loads, three adds
/// and a store wait on, so more of them wake in one cycle than issue can
/// take; then a done flag and a spin.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let setup =
        [i().auipc(A0, 2).build(), i().addi(T0, 0, ITERATIONS).build(), i().addi(T1, 0, 1).build()];
    let body = [
        i().div(T2, A0, T1).build(),
        i().ld(A1, T2, 0).build(),
        i().add(A2, T2, T1).build(),
        i().ld(A3, T2, 8).build(),
        i().add(A4, T2, T0).build(),
        i().ld(A5, T2, 16).build(),
        i().add(A6, T2, T2).build(),
        i().ld(S0, T2, 24).build(),
        i().sd(T2, T0, 32).build(),
        i().add(S1, S1, A1).build(),
        i().add(S1, S1, A2).build(),
        i().add(S1, S1, A3).build(),
        i().add(S1, S1, A4).build(),
        i().add(S1, S1, A5).build(),
        i().add(S1, S1, A6).build(),
        i().add(S1, S1, S0).build(),
        i().addi(A0, A0, 8).build(),
        i().addi(T0, T0, -1).build(),
    ];
    let back = -4 * (body.len() as i32);
    let mut program = setup.to_vec();
    program.extend(body);
    program.push(i().bne(T0, 0, back).build());
    program.push(i().addi(S2, 0, 1).build());
    program.push(i().jal(0, 0).build());
    program
}

/// What a run produced: the sum, the cycle it finished and every
/// statistic.
#[derive(Debug, PartialEq)]
struct Outcome {
    sum: u64,
    cycle: u64,
    stats: Vec<(String, f64)>,
}

fn run(backend: BackendKind, first: RobTag) -> Outcome {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.console = crate::config::Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());
    ctx.sim.state.cores[0].pipeline.start_rob_tags_at(first);

    let finished = ctx.run_until(50_000, |ctx| ctx.get_reg(S2 as usize) == 1);

    let cycle = finished.unwrap_or_else(|| panic!("{backend:?}: the loop did not finish"));
    let stats =
        ctx.sim.state.stats.query("**").iter().map(|(path, v)| (path.to_string(), v)).collect();
    Outcome { sum: ctx.get_reg(S1 as usize), cycle, stats }
}

/// The first statistic two outcomes disagree on, or the cycle and sum.
fn first_difference(a: &Outcome, b: &Outcome) -> String {
    a.stats.iter().zip(&b.stats).find(|(x, y)| x != y).map_or_else(
        || format!("cycle {} vs {}, sum {} vs {}", a.cycle, b.cycle, a.sum, b.sum),
        |(x, y)| format!("{x:?} vs {y:?}"),
    )
}

/// Starts the tags far enough below the wrap for every instruction of the
/// loop body to be the first one tagged after it.
fn timing_ignores_where_tags_start(backend: BackendKind) {
    let reference = run(backend, RobTag::new(1));

    for before_wrap in 0..program().len() as u32 * 3 {
        let outcome = run(backend, RobTag::new(u32::MAX - before_wrap));

        assert!(
            outcome == reference,
            "{backend:?}, {before_wrap} tags before the wrap: {}",
            first_difference(&outcome, &reference)
        );
    }
}

#[test]
fn crossing_the_tag_wrap_changes_nothing_inorder() {
    timing_ignores_where_tags_start(BackendKind::InOrder);
}

#[test]
fn crossing_the_tag_wrap_changes_nothing_o3() {
    timing_ignores_where_tags_start(BackendKind::OutOfOrder);
}
