//! Cycle-exact checks of the backend timing model: a functional unit's
//! latency is what its dependents wait for, on both backends.

use crate::common::builder::instruction::InstructionBuilder;
use crate::common::harness::TestContext;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

const BASE_ADDR: u64 = 0x8000_0000;
const MEM_SIZE: usize = 0x1000;
const CHAIN_LEN: u32 = 20;
const DONE_REG: usize = 2;
const DONE_VALUE: u64 = 7;

fn config(backend: BackendType, int_mul_latency: u64) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.fu_config.int_mul_latency = int_mul_latency;
    config
}

/// Cycle at which `program` writes `DONE_VALUE` into `DONE_REG`.
fn cycles_to_finish(config: &Config, program: &[u32]) -> u64 {
    let mut tc = TestContext::new_with_config(config)
        .with_memory(MEM_SIZE, BASE_ADDR)
        .load_program(BASE_ADDR, program);
    tc.run_until(5_000, |tc| tc.get_reg(DONE_REG) == DONE_VALUE).expect("program did not finish")
}

fn done_marker() -> Vec<u32> {
    let nop = InstructionBuilder::new().nop().build();
    let mut tail =
        vec![InstructionBuilder::new().addi(DONE_REG as u32, 0, DONE_VALUE as i32).build()];
    tail.extend(std::iter::repeat_n(nop, 4));
    tail
}

/// `x1 = 1`, then `CHAIN_LEN` multiplies each reading the previous result.
fn dependent_multiply_chain() -> Vec<u32> {
    let mut program = vec![InstructionBuilder::new().addi(1, 0, 1).build()];
    program.extend((0..CHAIN_LEN).map(|_| InstructionBuilder::new().mul(1, 1, 1).build()));
    program.extend(done_marker());
    program
}

/// `x1 = 1`, then `CHAIN_LEN` multiplies that only read `x1`.
fn independent_multiplies() -> Vec<u32> {
    let mut program = vec![InstructionBuilder::new().addi(1, 0, 1).build()];
    program.extend((0..CHAIN_LEN).map(|i| InstructionBuilder::new().mul(3 + i, 1, 1).build()));
    program.extend(done_marker());
    program
}

fn assert_chain_pays_latency_per_link(backend: BackendType) {
    let fast = cycles_to_finish(&config(backend, 3), &dependent_multiply_chain());
    let slow = cycles_to_finish(&config(backend, 9), &dependent_multiply_chain());

    assert_eq!(
        slow - fast,
        u64::from(CHAIN_LEN) * 6,
        "{backend:?}: each link should wait the unit latency"
    );
}

fn assert_independent_ops_pay_latency_once(backend: BackendType) {
    let fast = cycles_to_finish(&config(backend, 3), &independent_multiplies());
    let slow = cycles_to_finish(&config(backend, 9), &independent_multiplies());

    assert_eq!(slow - fast, 6, "{backend:?}: a pipelined unit overlaps independent ops");
}

#[test]
fn o3_dependent_chain_pays_the_unit_latency_per_link() {
    assert_chain_pays_latency_per_link(BackendType::OutOfOrder);
}

#[test]
fn inorder_dependent_chain_pays_the_unit_latency_per_link() {
    assert_chain_pays_latency_per_link(BackendType::InOrder);
}

#[test]
fn o3_independent_ops_on_a_pipelined_unit_pay_the_latency_once() {
    assert_independent_ops_pay_latency_once(BackendType::OutOfOrder);
}

#[test]
fn inorder_independent_ops_on_a_pipelined_unit_pay_the_latency_once() {
    assert_independent_ops_pay_latency_once(BackendType::InOrder);
}
