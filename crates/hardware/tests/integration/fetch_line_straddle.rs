//! An instruction straddling a cache line waits for its second line.
//!
//! A loop such an instruction ends pays I-cache accesses for both lines on
//! every iteration, where the same loop inside one line pays none.

use crate::common::harness::TestContext;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::uarch::pipeline::engine::BackendType;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_SIZE: usize = 0x4000;
const X31: usize = 31;
const LINE_BYTES: usize = 64;
const I_CACHE_LATENCY: u64 = 8;
const C_NOP: u16 = 0x0001;
/// `addi x31, x31, 1` as its two half-words.
const MARKER: [u16; 2] = [0x8F93, 0x001F];

/// `jal x0, offset` as its two half-words.
fn jal_back(offset_bytes: i32) -> [u16; 2] {
    let imm = offset_bytes as u32;
    let word = ((imm >> 20) & 1) << 31
        | ((imm >> 1) & 0x3FF) << 21
        | ((imm >> 11) & 1) << 20
        | ((imm >> 12) & 0xFF) << 12
        | 0x6F;
    [word as u16, (word >> 16) as u16]
}

/// A loop of `nops` compressed nops, the marker and the jump back.
fn program(nops: usize) -> Vec<u16> {
    let mut program = vec![C_NOP; nops];
    program.extend(MARKER);
    let loop_bytes = program.len() as i32 * 2;
    program.extend(jal_back(-loop_bytes));
    program
}

/// Cycles the loop's third iteration takes, with both lines already cached.
fn warm_iteration_cycles(backend: BackendType, program: &[u16]) -> u64 {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.cache.l1_i.enabled = true;
    config.cache.l1_i.line_bytes = LINE_BYTES;
    config.cache.l1_i.latency = I_CACHE_LATENCY;
    let mut tc = TestContext::new_with_config(&config).with_memory(RAM_SIZE, RAM_BASE);
    for (i, half) in program.iter().enumerate() {
        tc.sim.probe_mem_store(PhysAddr::new(RAM_BASE + 2 * i as u64), u64::from(*half), 2);
    }
    tc.sim.state.harts[0].pc = RAM_BASE;
    tc.sim.sync_arch_regs();

    let mut cycles = 0;
    let mut second_iteration = None;
    while tc.get_reg(X31) != 3 {
        tc.run(1);
        cycles += 1;
        if tc.get_reg(X31) == 2 && second_iteration.is_none() {
            second_iteration = Some(cycles);
        }
        assert!(cycles < 10_000, "{backend:?}: the loop never ran three times");
    }
    cycles - second_iteration.unwrap()
}

fn check_straddling_instruction_waits_for_its_second_line(backend: BackendType) {
    let inside_line = warm_iteration_cycles(backend, &program(LINE_BYTES / 2 - 4));
    let straddling = warm_iteration_cycles(backend, &program(LINE_BYTES / 2 - 3));

    let extra = straddling.saturating_sub(inside_line);
    assert!(
        (I_CACHE_LATENCY..=2 * I_CACHE_LATENCY + 2).contains(&extra),
        "{backend:?}: {inside_line} cycles inside one line, {straddling} straddling: expected one or two I-cache accesses ({I_CACHE_LATENCY} cycles each) more"
    );
}

#[test]
fn a_straddling_instruction_waits_for_its_second_line_inorder() {
    check_straddling_instruction_waits_for_its_second_line(BackendType::InOrder);
}

#[test]
fn a_straddling_instruction_waits_for_its_second_line_o3() {
    check_straddling_instruction_waits_for_its_second_line(BackendType::OutOfOrder);
}
