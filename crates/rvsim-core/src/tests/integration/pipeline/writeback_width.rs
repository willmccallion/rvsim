//! The out-of-order backend writes back at most `writeback_width` results
//! per cycle (gem5's `wbWidth`); results beyond that wait for later slots.
//! Loads forwarded from the store buffer and vector results take the same
//! slots as every other result.

use crate::config::BackendKind;
use crate::config::Config;
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, S2, T0, T1, X0, after};
use crate::uarch::pipeline::engine::PipelineDispatch;
use crate::uarch::pipeline::rob::RobState;

const ADDS_PER_ITERATION: usize = 32;
const ITERATIONS: usize = 50;
const CYCLE_LIMIT: u64 = 50_000;
const COUNTER: RegIdx = RegIdx::new(28);
const DONE_REG: RegIdx = RegIdx::new(31);
const DONE: u64 = 7;
/// Results each iteration writes back besides its body's: the counter's
/// decrement and the branch.
const LOOP_RESULTS: usize = 2;
const DATA_OFFSET: i32 = 0x400;
/// `vsetivli zero, 1, e64, m1, ta, ma`: one element, so a vector add
/// finishes as soon as a scalar one.
const VSETIVLI_ONE_E64: u32 = 0xcd80_f057;

/// `vadd.vv vd, vs2, vs1`, unmasked.
const fn vadd_vv(vd: u32, vs2: u32, vs1: u32) -> u32 {
    0x0200_0057 | (vs2 << 20) | (vs1 << 15) | (vd << 7)
}

/// `count` adds writing the eight registers from `first` in turn, none
/// reading another's result.
fn independent_adds(count: usize, first: RegIdx) -> Vec<u32> {
    let i = InstructionBuilder::new;
    (0..count).map(|n| i().addi(after(first, n % 8), X0, n as i32).build()).collect()
}

/// `setup`, then `body` run `ITERATIONS` times, long enough for the warm
/// front end to keep the backend fed.
fn looped(setup: &[u32], body: &[u32]) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = setup.to_vec();
    program.push(i().addi(COUNTER, X0, ITERATIONS as i32).build());
    program.extend(body);
    let body_bytes = 4 * (body.len() as i32 + 1);
    program.push(i().addi(COUNTER, COUNTER, -1).build());
    program.push(i().bne(COUNTER, X0, -body_bytes).build());
    program.push(i().addi(DONE_REG, X0, DONE as i32).build());
    program.push(i().jal(X0, 0).build());
    program
}

/// A four-wide out-of-order machine writing back `writeback_width` results
/// a cycle.
fn machine(writeback_width: usize) -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.width = 4;
    config.pipeline.writeback_width = Some(writeback_width);
    config.cache.l1_i.enabled = true;
    config
}

fn cycles_with_writeback_width(program: &[u32], writeback_width: usize) -> u64 {
    let mut ctx = TestContext::with_fast_memory(&machine(writeback_width))
        .load_program(PROGRAM_BASE, program);
    ctx.run_until(CYCLE_LIMIT, |ctx| ctx.get_reg(DONE_REG) == DONE).expect("program finished")
}

/// The most results `program` completed in any one cycle on `config`'s
/// machine: the ROB entries newly completed, counting those that retired
/// the same cycle.
fn most_results_in_a_cycle(config: &Config, program: &[u32]) -> usize {
    let mut ctx = TestContext::with_fast_memory(config).load_program(PROGRAM_BASE, program);
    let (mut completed_before, mut retired_before, mut most) = (0, 0, 0);
    for _ in 0..CYCLE_LIMIT {
        ctx.run(1);
        let PipelineDispatch::OutOfOrder(pipeline) = &ctx.sim.state.cores[0].pipeline else {
            unreachable!("the machine is out of order")
        };
        let rob = pipeline.engine.rob.iter();
        let completed = rob.filter(|entry| entry.state == RobState::Completed).count();
        let retired = ctx.sim.state.harts[0].instructions_retired as usize;
        let newly = (completed + retired - retired_before).saturating_sub(completed_before);
        most = most.max(newly);
        (completed_before, retired_before) = (completed, retired);
        if ctx.get_reg(DONE_REG) == DONE {
            return most;
        }
    }
    panic!("the program did not finish in {CYCLE_LIMIT} cycles");
}

#[test]
fn a_single_writeback_port_writes_back_one_result_per_cycle() {
    let program = looped(&[], &independent_adds(ADDS_PER_ITERATION, T0));
    let results = (ADDS_PER_ITERATION + LOOP_RESULTS) * ITERATIONS;

    let narrow = cycles_with_writeback_width(&program, 1);
    let wide = cycles_with_writeback_width(&program, 4);

    assert!(narrow >= results as u64, "narrow {narrow}: one result per cycle");
    assert!(wide * 2 < narrow, "wide {wide} vs narrow {narrow}");
}

/// Each iteration stores a doubleword and loads it back, forwarded from
/// the store buffer and written back the cycle it is forwarded, beside
/// independent adds.
#[test]
fn a_forwarded_load_takes_the_single_writeback_ports_slot() {
    const ADDS: usize = 6;
    let mut config = machine(1);
    config.pipeline.store_forward_latency = Some(0);
    let i = InstructionBuilder::new;
    let setup = [i().auipc(A0, 0).build(), i().addi(A0, A0, DATA_OFFSET).build()];
    let mut body = vec![i().sd(A0, T0, 0).build(), i().ld(T1, A0, 0).build()];
    body.extend(independent_adds(ADDS, S2));

    assert_eq!(most_results_in_a_cycle(&config, &looped(&setup, &body)), 1);
}

#[test]
fn a_vector_result_takes_the_single_writeback_ports_slot() {
    const VECTOR_ADDS: u32 = 4;
    const ADDS: usize = 4;
    let mut body: Vec<u32> = (0..VECTOR_ADDS).map(|n| vadd_vv(1 + n, 8 + n, 16 + n)).collect();
    body.extend(independent_adds(ADDS, S2));

    let program = looped(&[VSETIVLI_ONE_E64], &body);

    assert_eq!(most_results_in_a_cycle(&machine(1), &program), 1);
}
