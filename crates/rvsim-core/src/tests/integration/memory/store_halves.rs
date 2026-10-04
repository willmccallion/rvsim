//! A store whose address issues ahead of its data still delivers the data:
//! a load of the same address waits for it and reads it, and memory holds
//! it once the store retires.

use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, MemDepPredictorKind};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const BASE_ADDR: u64 = 0x8000_0000;
const MEM_SIZE: usize = 0x1000;
const DATA_OFFSET: i32 = 0x400;
const DONE_REG: usize = 2;
const DONE_VALUE: u64 = 7;

/// Stores `100 / 7` from a divide, so the store's address is known long
/// before its data, then loads it back.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().auipc(10, 0).build(),
        i().addi(10, 10, DATA_OFFSET).build(),
        i().addi(11, 0, 100).build(),
        i().addi(12, 0, 7).build(),
        i().div(13, 11, 12).build(),
        i().sd(10, 13, 0).build(),
        i().ld(6, 10, 0).build(),
        i().addi(DONE_REG as u32, 0, DONE_VALUE as i32).build(),
        // Spin, so the store's write can reach memory after it retires.
        i().jal(0, 0).build(),
    ];
    program.extend(std::iter::repeat_n(i().nop().build(), 4));
    program
}

fn run(predictor: MemDepPredictorKind) -> TestContext {
    let mut config = Config::default();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.width = 4;
    config.pipeline.mem_dep_predictor = predictor;
    config.pipeline.fu_config.int_div_latency = 20;
    config.cache.l1_d.enabled = true;
    let mut tc = TestContext::new_with_config(&config)
        .with_memory(MEM_SIZE, BASE_ADDR)
        .load_program(BASE_ADDR, &program());
    tc.run_until(2_000, |tc| tc.get_reg(DONE_REG) == DONE_VALUE).expect("program did not finish");
    tc.run(200);
    tc
}

#[test]
fn a_load_of_a_store_whose_data_comes_late_reads_that_data() {
    for predictor in [MemDepPredictorKind::Blind, MemDepPredictorKind::StoreSet] {
        let tc = run(predictor);
        assert_eq!(tc.get_reg(6), 100 / 7, "{predictor:?}");
    }
}

#[test]
fn a_store_whose_data_comes_late_writes_that_data() {
    let mut tc = run(MemDepPredictorKind::Blind);
    let address = PhysAddr::new(BASE_ADDR + DATA_OFFSET as u64);
    assert_eq!(tc.sim.probe_mem_load(address, 8), 100 / 7);
}

#[test]
fn the_data_half_issues_after_the_address_half() {
    let tc = run(MemDepPredictorKind::Blind);
    let paths = &tc.sim.state.cores[0].units.stat_paths.lsq;
    assert_eq!(tc.sim.state.stats.get(paths.split_stores), Some(1.0));
}
