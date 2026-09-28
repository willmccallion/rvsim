//! Each pipeline stage can have its own width; the narrowest bounds
//! throughput.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const PROGRAM_BASE: u64 = 0x8000_0000;
const ADDS: u64 = 32;
const X31: usize = 31;

type Narrowing = fn(&mut Config);

/// A loop of thirty-two independent adds over eight registers and a
/// counter increment.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> =
        (0..ADDS).map(|n| i().addi(5 + (n % 8) as u32, 0, 1).build()).collect();
    program.push(i().addi(X31 as u32, X31 as u32, 1).build());
    program.push(i().jal(0, -(program.len() as i32) * 4).build());
    program
}

/// Cycles the loop's third iteration takes, with the caches warm.
fn warm_iteration_cycles(backend: BackendType, narrow: impl Fn(&mut Config)) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.cache.l1_i.enabled = true;
    config.system.console = rvsim_core::config::Console::Quiet;
    narrow(&mut config);
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program());

    let mut cycles = 0;
    let mut second_iteration = None;
    while ctx.get_reg(X31) != 3 {
        ctx.run(1);
        cycles += 1;
        if ctx.get_reg(X31) == 2 && second_iteration.is_none() {
            second_iteration = Some(cycles);
        }
        assert!(cycles < 2000, "{backend:?}: the loop never ran three times");
    }
    cycles - second_iteration.unwrap()
}

fn check_each_stage_width_bounds_throughput(backend: BackendType) {
    let wide = warm_iteration_cycles(backend, |_| {});
    assert!(wide < ADDS, "{backend:?}: four-wide took {wide} cycles for {ADDS} adds");

    let narrowed: [(&str, Narrowing); 5] = [
        ("fetch", |c| c.pipeline.fetch_width = Some(1)),
        ("decode", |c| c.pipeline.decode_width = Some(1)),
        ("rename", |c| c.pipeline.rename_width = Some(1)),
        ("issue", |c| c.pipeline.issue_width = Some(1)),
        ("commit", |c| c.pipeline.commit_width = Some(1)),
    ];
    for (stage, narrow) in narrowed {
        let cycles = warm_iteration_cycles(backend, narrow);
        assert!(cycles >= ADDS, "{backend:?}: {stage} width 1 took only {cycles} cycles");
    }
}

#[test]
fn each_stage_width_bounds_throughput_inorder() {
    check_each_stage_width_bounds_throughput(BackendType::InOrder);
}

#[test]
fn each_stage_width_bounds_throughput_o3() {
    check_each_stage_width_bounds_throughput(BackendType::OutOfOrder);
}
