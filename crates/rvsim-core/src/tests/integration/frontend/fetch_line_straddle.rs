//! An instruction straddling a cache line waits for its second line.
//!
//! A loop such an instruction ends pays I-cache accesses for both lines on
//! every iteration, where the same loop inside one line pays none. Fetch
//! that lands on such an instruction fetches its first line before its
//! second; one straddling two fetch blocks of a line leads the group that
//! fetches the second block.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::BackendKind;
use crate::config::Config;
use crate::config::defaults::RAM_BASE;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::harness::TestContext;
use crate::tests::support::regs::X31;

const RAM_SIZE: usize = 0x4000;
const LINE_BYTES: usize = 64;
const I_CACHE_LATENCY: u64 = 8;
const C_NOP: u16 = 0x0001;
/// `addi x31, x31, 1` as its two half-words.
const MARKER: [u16; 2] = [0x8F93, 0x001F];

/// `jal x0, offset` as its two half-words.
fn jal_back(offset_bytes: i32) -> [u16; 2] {
    let imm = offset_bytes.cast_unsigned();
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

/// A `width`-wide core on `backend` with an I-cache of `LINE_BYTES` lines.
fn core(backend: BackendKind, width: usize) -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    config.cache.l1_i.enabled = true;
    config.cache.l1_i.line_bytes = std::num::NonZero::new(LINE_BYTES).unwrap();
    config.cache.l1_i.latency = I_CACHE_LATENCY;
    config
}

/// `config` running `program`'s half-words from the start of RAM.
fn loaded(config: &Config, program: &[u16]) -> TestContext {
    let mut tc = TestContext::with_fast_memory(config).with_memory(RAM_SIZE, RAM_BASE);
    for (i, half) in program.iter().enumerate() {
        tc.sim.probe_mem_store(
            PhysAddr::new(RAM_BASE + 2 * i as u64),
            u64::from(*half),
            ProbeWidth::B2,
        );
    }
    tc.sim.state.harts[0].pc = VirtAddr::new(RAM_BASE);
    tc.sim.sync_arch_regs();
    tc
}

/// Cycles the loop's third iteration takes, with both lines already cached.
fn warm_iteration_cycles(backend: BackendKind, program: &[u16]) -> u64 {
    let mut tc = loaded(&core(backend, 4), program);

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

fn check_straddling_instruction_waits_for_its_second_line(backend: BackendKind) {
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
    check_straddling_instruction_waits_for_its_second_line(BackendKind::InOrder);
}

#[test]
fn a_straddling_instruction_waits_for_its_second_line_o3() {
    check_straddling_instruction_waits_for_its_second_line(BackendKind::OutOfOrder);
}

/// The I-cache accesses `tc` has made.
fn i_cache_accesses(tc: &TestContext) -> u64 {
    let paths = &tc.sim.state.cores[0].units.l1_i_cache.stat_paths;
    let stats = &tc.sim.state.uncore.kernel.stats;
    [paths.hits, paths.misses].iter().map(|&path| stats.get(path).unwrap_or(0.0) as u64).sum()
}

/// Iterations of a marker loop whose warm-up the per-iteration figures
/// skip, and the iterations they average over.
const WARM_UP: u64 = 10;
const MEASURED: u64 = 10;

/// Cycles and I-cache accesses each iteration of `program`'s marker loop
/// takes once warm.
fn per_iteration(config: &Config, program: &[u16]) -> (f64, f64) {
    let mut tc = loaded(config, program);
    let run_to = |tc: &mut TestContext, marker: u64| {
        tc.run_until(100_000, |tc| tc.get_reg(X31) == marker).expect("the loop ran")
    };
    let start = run_to(&mut tc, WARM_UP);
    let start_accesses = i_cache_accesses(&tc);
    let end = run_to(&mut tc, WARM_UP + MEASURED);
    let accesses = i_cache_accesses(&tc) - start_accesses;
    ((end - start) as f64 / MEASURED as f64, accesses as f64 / MEASURED as f64)
}

/// A jump to the line's last half-word, a loop that starts there with the
/// marker straddling into the next line, three compressed nops, and the
/// jump back to the marker.
fn loop_from_a_straddling_marker() -> Vec<u16> {
    let marker_offset = LINE_BYTES as i32 - 2;
    let mut program = jal_back(marker_offset).to_vec();
    program.resize(LINE_BYTES / 2 - 1, C_NOP);
    program.extend(MARKER);
    program.extend([C_NOP; 3]);
    program.extend(jal_back(-(2 * 3 + 4)));
    program
}

#[test]
fn fetch_landing_on_a_straddling_instruction_fetches_both_its_lines() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        // The jump back is on the second line, so the fetch buffer holds
        // that one; the marker's first line is fetched again, then its
        // second.
        let (_, accesses) = per_iteration(&core(backend, 4), &loop_from_a_straddling_marker());

        assert_eq!(accesses, 2.0, "{backend:?}");
    }
}

/// A loop inside one line: seven compressed nops, the marker straddling
/// the first two 16-byte blocks, nine compressed nops and the jump back.
fn loop_with_a_marker_straddling_blocks() -> Vec<u16> {
    let mut program = vec![C_NOP; 7];
    program.extend(MARKER);
    program.extend([C_NOP; 9]);
    let loop_bytes = program.len() as i32 * 2;
    program.extend(jal_back(-loop_bytes));
    program
}

/// The offsets from the start of RAM of the first fetch group `config`
/// forms that starts at `offset` in `program`.
fn group_starting_at(config: &Config, program: &[u16], offset: u64) -> Vec<u64> {
    let mut tc = loaded(config, program);
    for _ in 0..1_000 {
        tc.run(1);
        let group = tc.sim.state.cores[0].pipeline.snapshot(1).fetch1_fetch2;
        if group.first().is_some_and(|slot| slot.pc == VirtAddr::new(RAM_BASE + offset)) {
            return group.iter().map(|slot| slot.pc.val() - RAM_BASE).collect();
        }
    }
    panic!("fetch never formed a group at offset {offset}");
}

#[test]
fn an_instruction_straddling_two_blocks_leads_the_group_that_fetches_the_second() {
    // Wide enough that the block, not the width, ends the group.
    let mut config = core(BackendKind::OutOfOrder, 12);
    config.pipeline.fetch_block_bytes = Some(16);
    let marker = 14;

    let group = group_starting_at(&config, &loop_with_a_marker_straddling_blocks(), marker);

    // The marker, then the compressed nops up to the second block's end.
    let nops = (marker + 4..32).step_by(2);
    assert_eq!(group, std::iter::once(marker).chain(nops).collect::<Vec<_>>());
}
