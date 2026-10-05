//! The branch predictor, the memory dependence predictor, the load/store
//! queue, the write-combining buffer and the load prefetcher.

use super::program::{
    A1, BACKENDS, T0, T1, T2, config, ending_in_spin, exit_sequence, run_to_exit, run_to_pc,
    system, system_with,
};
use super::{Recorder, accounting_checks};
use crate::config::{BackendKind, LoadPrefetcherConfig, MemDepPredictorKind, PageBoundary};
use crate::tests::integration::translation::load_prefetch_paging::{Data, Mode, run_system};
use crate::tests::support::builder::instruction::InstructionBuilder;

const LOOPS: i32 = 5;

/// A loop whose branch is taken `LOOPS - 1` times and falls through once;
/// the static predictor predicts every one not taken.
fn mispredicted_loop() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T1, 0, LOOPS).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -4).build(),
    ];
    program.extend(exit_sequence());
    program
}

fn branch_outcomes_are_counted_at_resolution_and_at_commit(rec: &mut Recorder) {
    let (wrong, right) = (LOOPS as u64 - 1, 1);
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut ctx = system(backend, &mispredicted_loop(), &[]);

        run_to_exit(&mut ctx, &context);

        rec.expect(&ctx.sim, "core0.bp.committed.mispredicts", wrong, &context);
        rec.expect(&ctx.sim, "core0.bp.committed.hits", right, &context);
        let accuracy = right as f64 / (right + wrong) as f64;
        rec.expect_ratio(&ctx.sim, "core0.bp.committed.accuracy", accuracy, &context);
        let spec_wrong = rec.read(&ctx.sim, "core0.bp.spec.mispredicts");
        let spec_right = rec.read(&ctx.sim, "core0.bp.spec.hits");
        assert!(spec_wrong >= wrong && spec_right >= right, "{context}: wrong-path ones too");
        let spec_accuracy = spec_right as f64 / (spec_right + spec_wrong) as f64;
        rec.expect_ratio(&ctx.sim, "core0.bp.spec.accuracy", spec_accuracy, &context);
    }
}

fn decode_redirects_fetch_to_each_jump_it_finds(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    let jumps = 3;
    let mut program = Vec::new();
    for _ in 0..jumps {
        program.extend([i().jal(0, 8).build(), i().addi(T0, 0, 1).build()]);
    }
    program.extend(exit_sequence());
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut ctx = system(backend, &program, &[]);
        let mut straight = system(backend, &mispredicted_loop(), &[]);

        run_to_exit(&mut ctx, &context);
        run_to_exit(&mut straight, &context);

        rec.expect(&ctx.sim, "core0.bp.decode_redirects", jumps, &context);
        rec.expect(&ctx.sim, "core0.bp.committed.hits", jumps, &context);
        rec.expect(&straight.sim, "core0.bp.decode_redirects", 0, &context);
    }
}

/// A store whose address waits on a divide, then a load from the same
/// address that does not.
fn load_past_unresolved_store() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T0, 0, 7).build(),
        i().div(T1, T0, T0).build(),
        i().add(T2, A1, T1).build(),
        i().addi(T2, T2, -1).build(),
        i().sd(T2, T0, 0).build(),
        i().ld(T1, A1, 0).build(),
    ];
    program.extend(exit_sequence());
    program
}

fn a_violation_trains_store_sets_to_make_the_load_wait(rec: &mut Recorder) {
    let context = "OutOfOrder";
    let mut ctx = system(BackendKind::OutOfOrder, &load_past_unresolved_store(), &[]);

    run_to_exit(&mut ctx, context);

    rec.expect(&ctx.sim, "core0.mdp.predictions.bypass", 1, context);
    rec.expect(&ctx.sim, "core0.mdp.violations", 1, context);
    rec.expect(&ctx.sim, "core0.mdp.predictions.wait_for", 1, context);
    rec.expect(&ctx.sim, "core0.mdp.predictions.wait_all", 0, context);
}

/// `count` loads from the data page, then the exit.
fn loads(count: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> = (0..count).map(|n| i().ld(T0, A1, 8 * n).build()).collect();
    program.extend(exit_sequence());
    program
}

fn the_blind_predictor_makes_every_load_wait_for_all_stores(rec: &mut Recorder) {
    let context = "OutOfOrder blind";
    let mut config = config(BackendKind::OutOfOrder);
    config.pipeline.mem_dep_predictor = MemDepPredictorKind::Blind;
    let mut ctx = system_with(&config, &loads(3), &[]);

    run_to_exit(&mut ctx, context);

    rec.expect(&ctx.sim, "core0.mdp.predictions.wait_all", 3, context);
    rec.expect(&ctx.sim, "core0.mdp.predictions.bypass", 0, context);
}

fn predictor_counts_start_again_after_a_stats_reset(rec: &mut Recorder) {
    let context = "OutOfOrder";
    let i = InstructionBuilder::new;
    let before: Vec<u32> = (0..3).map(|n| i().ld(T0, A1, 8 * n).build()).collect();
    let (program, end) = ending_in_spin(before);
    let mut after: Vec<u32> = (0..2).map(|n| i().ld(T0, A1, 64 + 8 * n).build()).collect();
    after.extend(exit_sequence());
    let mut ctx = system(BackendKind::OutOfOrder, &program, &[]);
    let continuation = super::program::PROGRAM_BASE + 0x100;
    super::program::store_words(&mut ctx, continuation, &after);
    run_to_pc(&mut ctx, end, context);

    ctx.sim.reset_stats();
    ctx.sim.set_pc(0, continuation);
    run_to_exit(&mut ctx, context);

    rec.expect(&ctx.sim, "core0.mdp.predictions.bypass", 2, context);
}

fn a_store_whose_data_comes_late_issues_in_two_halves(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    let late_data = |late: bool| {
        let mut program = vec![i().addi(T0, 0, 7).build(), i().div(T1, T0, T0).build()];
        let data = if late { T1 } else { T0 };
        program.push(i().sd(A1, data, 0).build());
        program.extend(exit_sequence());
        program
    };
    let context = "OutOfOrder";
    let mut late = system(BackendKind::OutOfOrder, &late_data(true), &[]);
    let mut ready = system(BackendKind::OutOfOrder, &late_data(false), &[]);

    run_to_exit(&mut late, context);
    run_to_exit(&mut ready, context);

    rec.expect(&late.sim, "core0.lsq.split_stores", 1, context);
    rec.expect(&ready.sim, "core0.lsq.split_stores", 0, context);
}

/// `mtime`, from the CLINT's base.
const MTIME: u64 = 0xBFF8;

fn a_device_read_waiting_to_be_oldest_counts_one_wait(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mtime = config(backend).system.clint_base + MTIME;
        let mut program = vec![
            i().addi(T0, 0, 7).build(),
            i().div(T1, T0, T0).build(),
            i().lui(T0, ((mtime + 8) >> 12) as i32).build(),
            i().ld(T2, T0, -8).build(),
        ];
        program.extend(exit_sequence());
        let mut ctx = system(backend, &program, &[]);
        let mut independent = system(backend, &loads(2), &[]);

        run_to_exit(&mut ctx, &context);
        run_to_exit(&mut independent, &context);

        rec.expect(&ctx.sim, "core0.lsq.rescheduled_mem_ops", 1, &context);
        rec.expect(&independent.sim, "core0.lsq.rescheduled_mem_ops", 0, &context);
    }
}

/// Stores to `lines` lines, `per_line` doublewords each, then the exit.
fn stores(lines: i32, per_line: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = Vec::new();
    for line in 0..lines {
        for word in 0..per_line {
            program.push(i().sd(A1, T0, 64 * line + 8 * word).build());
        }
    }
    program.extend(exit_sequence());
    program
}

fn the_write_combining_buffer_counts_merges_into_held_lines_and_line_writes(rec: &mut Recorder) {
    for backend in BACKENDS {
        for (lines, per_line) in [(1, 4), (4, 1)] {
            let context = format!("{backend:?} {lines} lines of {per_line}");
            let mut config = config(backend);
            config.cache.wcb_entries = 8;
            let mut ctx = system_with(&config, &stores(lines, per_line), &[]);

            run_to_exit(&mut ctx, &context);

            let merged = (lines * (per_line - 1)) as u64;
            rec.expect(&ctx.sim, "core0.wcb.coalesces", merged, &context);
            rec.expect(&ctx.sim, "core0.wcb.drains", lines as u64, &context);
        }
    }
}

const STREAM_LOADS: i32 = 24;
const RAM_SIZE: usize = 0x1_0000;

/// One load a line for `STREAM_LOADS` lines from `a1`, each load's address
/// depending on the last's (zero) value so the misses do not fill every
/// MSHR and leave the prefetches none.
fn line_stride() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T1, 0, STREAM_LOADS).build(),
        i().ld(T2, A1, 0).build(),
        i().add(A1, A1, T2).build(),
        i().addi(A1, A1, 64).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -16).build(),
    ];
    program.extend(exit_sequence());
    program
}

fn prefetching(backend: BackendKind, page_boundary: PageBoundary) -> crate::config::Config {
    let mut config = config(backend);
    config.cache.l1_d.enabled = true;
    config.cache.l1_d.size_bytes = 32 * 1024;
    config.cache.l1_d.ways = 8;
    config.cache.l2.enabled = true;
    config.cache.load_prefetcher =
        LoadPrefetcherConfig::Stride { table_size: 64, l1_lines: 2, l2_lines: 4, page_boundary };
    config.memory.ram_size = RAM_SIZE;
    config
}

fn a_stride_stream_is_prefetched_into_each_level_and_dropped_where_it_must_be(rec: &mut Recorder) {
    let ram_end = super::program::PROGRAM_BASE + RAM_SIZE as u64;
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut inside =
            system_with(&prefetching(backend, PageBoundary::Stop), &line_stride(), &[]);
        let mut to_the_end =
            system_with(&prefetching(backend, PageBoundary::CrossWithTlb), &line_stride(), &[]);
        to_the_end.set_reg(A1 as usize, ram_end - 64 * STREAM_LOADS as u64);
        to_the_end.sim.sync_arch_regs();

        run_to_exit(&mut inside, &context);
        run_to_exit(&mut to_the_end, &context);

        let lines = STREAM_LOADS as u64;
        let into_l1 = rec.read(&inside.sim, "core0.prefetch.loads.l1");
        let into_l2 = rec.read(&inside.sim, "core0.prefetch.loads.l2");
        assert!((lines - 4..=lines + 2).contains(&into_l1), "{context}: {into_l1} into the L1D");
        assert!((lines - 4..=lines + 4).contains(&into_l2), "{context}: {into_l2} into the L2");
        rec.expect(&inside.sim, "core0.prefetch.loads.dropped.not_ram", 0, &context);
        rec.expect(&inside.sim, "core0.prefetch.loads.dropped.page_boundary", 0, &context);
        let past_ram = rec.read(&to_the_end.sim, "core0.prefetch.loads.dropped.not_ram");
        assert!(past_ram > 0, "{context}");
    }
}

fn a_stream_stops_at_the_page_or_drops_what_the_tlb_cannot_place(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let stopped = run_system(backend, Mode::Sv39, PageBoundary::Stop, Data::TouchedSecondPage);
        let missed =
            run_system(backend, Mode::Sv39, PageBoundary::CrossWithTlb, Data::UntouchedSecondPage);
        let denied = run_system(
            backend,
            Mode::Sv39,
            PageBoundary::CrossWithTlb,
            Data::ExecuteOnlySecondPage,
        );
        let crossed =
            run_system(backend, Mode::Sv39, PageBoundary::CrossWithTlb, Data::TouchedSecondPage);

        for (ctx, dropped) in
            [(&stopped, "page_boundary"), (&missed, "tlb_miss"), (&denied, "denied")]
        {
            let path = format!("core0.prefetch.loads.dropped.{dropped}");
            assert!(rec.read(&ctx.sim, &path) > 0, "{context}: {path}");
        }
        for dropped in ["page_boundary", "tlb_miss", "denied"] {
            let path = format!("core0.prefetch.loads.dropped.{dropped}");
            rec.expect(&crossed.sim, &path, 0, &context);
        }
    }
}

accounting_checks!(
    branch_outcomes_are_counted_at_resolution_and_at_commit,
    decode_redirects_fetch_to_each_jump_it_finds,
    a_violation_trains_store_sets_to_make_the_load_wait,
    the_blind_predictor_makes_every_load_wait_for_all_stores,
    predictor_counts_start_again_after_a_stats_reset,
    a_store_whose_data_comes_late_issues_in_two_halves,
    a_device_read_waiting_to_be_oldest_counts_one_wait,
    the_write_combining_buffer_counts_merges_into_held_lines_and_line_writes,
    a_stride_stream_is_prefetched_into_each_level_and_dropped_where_it_must_be,
    a_stream_stops_at_the_page_or_drops_what_the_tlb_cannot_place,
);
