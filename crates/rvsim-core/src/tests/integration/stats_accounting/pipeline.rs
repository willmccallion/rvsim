//! Pipeline flushes by cause and the cycles each stage stalled.

use super::program::{
    A1, BACKENDS, MHARTID, T0, T1, T2, config, ending_in_spin, exit_sequence, run_to_exit,
    run_to_pc, system, system_with, system_with_configured_latency, three_handled_ecalls,
};
use super::{Recorder, accounting_checks};
use crate::config::{BackendKind, Config};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const LOOPS: i32 = 5;
const MIE: u32 = 0x304;
const MIP: u32 = 0x344;

const CAUSES: [&str; 5] = ["branch", "system", "mem_violations", "coherence", "trap"];

/// The flush counts by cause, in `CAUSES` order, asserting they sum to the
/// total.
fn flushes_by_cause(rec: &mut Recorder, sim: &crate::Simulator, context: &str) -> [u64; 5] {
    let counts = CAUSES.map(|cause| rec.read(sim, &format!("core0.pipeline.flushes.{cause}")));
    let total = rec.read(sim, "core0.pipeline.flushes.total");
    assert_eq!(counts.iter().sum::<u64>(), total, "{context}: {counts:?}");
    counts
}

/// A loop whose branch is taken `LOOPS - 1` times; the static predictor
/// predicts every one not taken.
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

fn a_mispredicted_branch_flushes_once_under_branch(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut ctx = system(backend, &mispredicted_loop(), &[]);

        run_to_exit(&mut ctx, &context);

        let [branch, system, memory, coherence, trap] = flushes_by_cause(rec, &ctx.sim, &context);
        assert_eq!((branch, system, memory, coherence), (LOOPS as u64 - 1, 0, 0, 0), "{context}");
        assert_eq!(trap, 1, "{context}: the exit ecall");
        assert!(rec.read(&ctx.sim, "core0.pipeline.flushes.squashed_insns") > 0, "{context}");
    }
}

fn traps_flush_under_trap_and_returns_under_system(rec: &mut Recorder) {
    for (backend, system_flushes) in [
        // Six CSR accesses in the handlers and the mtvec write refetch what
        // follows them; each mret flushes at execute and again at commit.
        (BackendKind::InOrder, 6 + 1 + 3 * 2),
        // CSR accesses serialize rename instead; each mret commits before
        // its execute-stage redirect is due, so only commit's flush is taken.
        (BackendKind::OutOfOrder, 3),
    ] {
        let context = format!("{backend:?}");
        let (program, handler) = three_handled_ecalls();
        let mut ctx = system(backend, &program, &handler);

        run_to_exit(&mut ctx, &context);

        let [branch, system, memory, coherence, trap] = flushes_by_cause(rec, &ctx.sim, &context);
        assert_eq!(trap, 3 + 1, "{context}: three ecalls and the exit");
        assert_eq!(system, system_flushes, "{context}");
        // The loop branch is taken twice, and resolves taken twice more on
        // the wrong path behind the first two ecalls before they trap.
        assert_eq!((branch, memory, coherence), (2 + 2, 0, 0), "{context}");
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

fn a_load_that_passed_an_aliasing_store_flushes_under_mem_violations(rec: &mut Recorder) {
    for (backend, violations) in [(BackendKind::InOrder, 0), (BackendKind::OutOfOrder, 1)] {
        let context = format!("{backend:?}");
        let mut ctx = system(backend, &load_past_unresolved_store(), &[]);

        run_to_exit(&mut ctx, &context);

        let counts = flushes_by_cause(rec, &ctx.sim, &context);
        assert_eq!(counts, [0, 0, violations, 0, 1], "{context}");
        assert_eq!(ctx.get_reg(T1 as usize), 7, "{context}: the load saw the store");
    }
}

fn every_flush_counts_the_rob_entries_it_dropped(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let (program, handler) = three_handled_ecalls();
        let mut ctx = system(backend, &program, &handler);

        run_to_exit(&mut ctx, &context);

        let flushes = rec.read(&ctx.sim, "core0.pipeline.flushes.total");
        let dropped = rec.read(&ctx.sim, "core0.pipeline.flushes.squashed_insns");
        let rob = ctx.sim.state.config.pipeline.rob_size as u64;
        assert!(dropped <= flushes * rob, "{context}: {dropped} from {flushes} flushes");
        assert!(dropped > 0, "{context}");
    }
}

const DIVIDES: i32 = 4;
const STRAIGHT_LINE: u32 = 24;

/// `count` independent ALU ops that never wait on one another, then the
/// exit.
fn independent_alu_ops(count: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program: Vec<u32> =
        (0..count).map(|n| i().addi(T0 + n % 3, 0, n as i32).build()).collect();
    program.extend(exit_sequence());
    program
}

/// `DIVIDES` divides, each of the previous one's result when `dependent`,
/// otherwise all of the same operands; then the exit.
fn divides(dependent: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().addi(T0, 0, 1).build(), i().addi(T1, 0, 1000).build()];
    for n in 0..DIVIDES as u32 {
        let destination = if dependent { T1 } else { T2 + n % 2 * (A1 - T2) };
        program.push(i().div(destination, T1, T0).build());
    }
    program.extend(exit_sequence());
    program
}

fn divide_latency(ctx: &TestContext) -> u64 {
    ctx.sim.state.config.pipeline.fu_config.int_div_latency
}

fn a_dependent_divide_chain_stalls_issue_on_data_and_independent_ops_do_not(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut chain = system(backend, &divides(true), &[]);
        let mut straight = system(backend, &independent_alu_ops(STRAIGHT_LINE), &[]);

        run_to_exit(&mut chain, &context);
        run_to_exit(&mut straight, &context);

        let waited = rec.read(&chain.sim, "core0.pipeline.stalls.data");
        let least = (DIVIDES as u64 - 1) * (divide_latency(&chain) - 2);
        assert!(waited >= least, "{context}: {waited} data stalls, at least {least}");
        rec.expect(&straight.sim, "core0.pipeline.stalls.data", 0, &context);
    }
}

fn independent_divides_stall_on_the_one_divider_not_on_data(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut config = config(backend);
        config.pipeline.fu_config.num_int_div = 1;
        let mut ctx = system_with(&config, &divides(false), &[]);
        let mut straight = system(backend, &independent_alu_ops(STRAIGHT_LINE), &[]);

        run_to_exit(&mut ctx, &context);
        run_to_exit(&mut straight, &context);

        let structural = rec.read(&ctx.sim, "core0.pipeline.stalls.fu_structural");
        let least = (DIVIDES as u64 - 1) * (divide_latency(&ctx) - 2);
        assert!(structural >= least, "{context}: {structural} structural, at least {least}");
        rec.expect(&ctx.sim, "core0.pipeline.stalls.data", 0, &context);
        rec.expect(&straight.sim, "core0.pipeline.stalls.fu_structural", 0, &context);
    }
}

/// A divide, then `then` behind it, then a spin.
fn behind_a_divide(then: &[u32]) -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T0, 0, 1).build(),
        i().addi(T1, 0, 1000).build(),
        i().div(T1, T1, T0).build(),
    ];
    program.extend_from_slice(then);
    ending_in_spin(program)
}

fn alu_ops(count: u32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    (0..count).map(|n| i().addi(T2 + n % 2 * (A1 - T2), 0, n as i32).build()).collect()
}

fn straight_line() -> (Vec<u32>, u64) {
    ending_in_spin(alu_ops(STRAIGHT_LINE))
}

fn run(config: &Config, (program, end): (Vec<u32>, u64), context: &str) -> TestContext {
    let mut ctx = system_with(config, &program, &[]);
    run_to_pc(&mut ctx, end, context);
    ctx
}

fn a_system_op_waits_to_be_oldest_under_ordering(rec: &mut Recorder) {
    let csr_read = InstructionBuilder::new().csrrs(T2, MHARTID, 0).build();
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let config = config(backend);

        let waiting = run(&config, behind_a_divide(&[csr_read]), &context);
        let straight = run(&config, straight_line(), &context);

        let held = rec.read(&waiting.sim, "core0.pipeline.stalls.ordering");
        let least = divide_latency(&waiting) - 3;
        assert!(held >= least, "{context}: {held} ordering stalls, at least {least}");
        rec.expect(&straight.sim, "core0.pipeline.stalls.ordering", 0, &context);
    }
}

fn rename_waits_behind_a_csr_access_on_out_of_order_only(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    let mut csr_then_alu = vec![i().csrrs(T2, MHARTID, 0).build()];
    csr_then_alu.extend(alu_ops(8));
    for (backend, serializes) in [(BackendKind::InOrder, false), (BackendKind::OutOfOrder, true)] {
        let context = format!("{backend:?}");
        let config = config(backend);

        let waiting = run(&config, behind_a_divide(&csr_then_alu), &context);
        let straight = run(&config, straight_line(), &context);

        let stalls = rec.read(&waiting.sim, "core0.pipeline.stalls.serialize");
        assert_eq!(stalls > 0, serializes, "{context}: {stalls}");
        rec.expect(&straight.sim, "core0.pipeline.stalls.serialize", 0, &context);
    }
}

fn branches_wait_for_a_checkpoint_only_when_they_run_out(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T0, 0, 1).build(),
        i().addi(T1, 0, 1000).build(),
        i().div(T1, T1, T0).build(),
    ];
    program.extend((0..6).map(|_| i().bne(T0, T0, 8).build()));
    // Exits rather than spins: every fetched copy of a spinning jump would
    // take a checkpoint too.
    program.extend(exit_sequence());
    let context = "OutOfOrder";
    let mut stalls = [0; 2];
    for (n, checkpoints) in [1, 16].into_iter().enumerate() {
        let mut config = config(BackendKind::OutOfOrder);
        config.pipeline.checkpoint_count = checkpoints;
        let mut ctx = system_with(&config, &program, &[]);

        run_to_exit(&mut ctx, context);

        stalls[n] = rec.read(&ctx.sim, "core0.pipeline.stalls.checkpoint");
    }
    assert!(stalls[0] > 0, "{context}: one checkpoint: {stalls:?}");
    assert_eq!(stalls[1], 0, "{context}: sixteen checkpoints: {stalls:?}");
}

fn rename_waits_for_a_full_rob_under_dispatch(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let mut stalls = [0; 2];
        for (n, rob_size) in [4, 64].into_iter().enumerate() {
            let mut config = config(backend);
            config.pipeline.rob_size = rob_size;

            let ctx = run(&config, behind_a_divide(&alu_ops(12)), &context);

            stalls[n] = rec.read(&ctx.sim, "core0.pipeline.stalls.dispatch");
        }
        assert!(stalls[0] > 0, "{context}: a 4-entry ROB: {stalls:?}");
        assert_eq!(stalls[1], 0, "{context}: a 64-entry ROB: {stalls:?}");
    }
}

fn a_mispredicted_loop_counts_control_and_squash_cycles(rec: &mut Recorder) {
    let i = InstructionBuilder::new;
    let looping = vec![
        i().addi(T1, 0, LOOPS).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -4).build(),
    ];
    for (backend, walks_the_rob) in [(BackendKind::InOrder, false), (BackendKind::OutOfOrder, true)]
    {
        let context = format!("{backend:?}");
        let config = config(backend);

        let ctx = run(&config, ending_in_spin(looping.clone()), &context);
        let straight = run(&config, straight_line(), &context);

        let flushes = rec.read(&ctx.sim, "core0.pipeline.flushes.branch");
        assert_eq!(flushes, LOOPS as u64 - 1, "{context}");
        let control = rec.read(&ctx.sim, "core0.pipeline.stalls.control");
        assert!(control >= flushes, "{context}: {control} control stalls");
        let squash = rec.read(&ctx.sim, "core0.pipeline.stalls.squash");
        assert_eq!(squash >= flushes, walks_the_rob, "{context}: {squash} squash stalls");
        if !walks_the_rob {
            assert_eq!(squash, 0, "{context}");
        }
        rec.expect(&straight.sim, "core0.pipeline.stalls.control", 0, &context);
        rec.expect(&straight.sim, "core0.pipeline.stalls.squash", 0, &context);
        let rob_empty = rec.read(&ctx.sim, "core0.pipeline.cycles.rob_empty");
        let idle = rec.read(&ctx.sim, "core0.commit.retire_histogram.zero");
        assert!(0 < rob_empty && rob_empty <= idle, "{context}: {rob_empty} of {idle}");
    }
}

fn fetch_waits_longer_on_slower_memory(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let config = config(backend);
        let (program, end) = straight_line();
        let mut slow = system_with_configured_latency(&config, &program, &[]);
        let mut fast = system_with(&config, &program, &[]);

        run_to_pc(&mut slow, end, &context);
        run_to_pc(&mut fast, end, &context);

        let slow_waits = rec.read(&slow.sim, "core0.pipeline.stalls.fetch_wait");
        let fast_waits = rec.read(&fast.sim, "core0.pipeline.stalls.fetch_wait");
        assert!(slow_waits > fast_waits, "{context}: {slow_waits} against {fast_waits}");
    }
}

/// Where the one page table lives.
const PAGE_TABLE: u64 = super::program::PROGRAM_BASE + 0x3000;
/// A valid, readable, writable, executable leaf with A and D set.
const LEAF_RWXAD: u64 = 0xCF;
/// PMP entry 0 covering all memory with read, write and execute.
const PMP_TOR_RWX: u8 = 0b0000_1111;

/// Loads from the data page in supervisor mode, entered by an `mret`, with
/// Sv39 on when `paged`: then the gigapage holding everything maps to
/// itself and the first load walks the table.
fn supervisor_loads(backend: BackendKind, paged: bool) -> (TestContext, u64) {
    use crate::isa::csr::{MSTATUS_MPP, SATP_MODE_SV39};
    use crate::isa::privileged::PrivilegeMode;
    use crate::tests::support::builder::instruction::MRET;
    let i = InstructionBuilder::new;
    let body = vec![
        i().auipc(T1, 0).build(),
        i().addi(T0, T1, 16).build(),
        i().csrrw(0, super::program::MEPC, T0).build(),
        MRET,
        i().ld(T2, A1, 0).build(),
        i().ld(T2, A1, 64).build(),
    ];
    let (program, end) = ending_in_spin(body);
    // Two wide, so the second load reaches memory1 with the first and waits
    // there while the first walks.
    let mut config = config(backend);
    config.pipeline.width = 2;
    let mut ctx = system_with(&config, &program, &[]);
    let gigapage = super::program::PROGRAM_BASE >> 30;
    let pte = ((super::program::PROGRAM_BASE >> 12) << 10) | LEAF_RWXAD;
    ctx.sim.probe_mem_store(crate::common::PhysAddr::new(PAGE_TABLE + 8 * gigapage), pte, 8);
    let hart = &mut ctx.sim.state.harts[0];
    let supervisor = u64::from(PrivilegeMode::Supervisor.to_u8()) << 11;
    hart.csrs.mstatus = (hart.csrs.mstatus & !MSTATUS_MPP) | supervisor;
    if paged {
        hart.csrs.satp = (SATP_MODE_SV39 << 60) | (PAGE_TABLE >> 12);
    }
    hart.pmp.set_addr(0, u64::MAX >> 10);
    hart.pmp.set_cfg(0, PMP_TOR_RWX);
    ctx.sim.state.direct_mode = false;
    (ctx, end)
}

fn memory_ops_held_behind_a_page_walk_count_as_backpressure(rec: &mut Recorder) {
    for backend in BACKENDS {
        let context = format!("{backend:?}");
        let (mut paged, end) = supervisor_loads(backend, true);
        let (mut bare, _) = supervisor_loads(backend, false);

        run_to_pc(&mut paged, end, &context);
        run_to_pc(&mut bare, end, &context);

        let walked = rec.read(&paged.sim, "core0.pipeline.stalls.backpressure");
        assert!(walked > 0, "{context}");
        rec.expect(&bare.sim, "core0.pipeline.stalls.backpressure", 0, &context);
    }
}

/// Hart 0's `mtimecmp`, from the CLINT's base.
const MTIMECMP: u64 = 0x4000;
/// Machine time the timer interrupt is due at.
const WAKE_AT: u64 = 200;

/// Arms the timer for `WAKE_AT`, waits until the old comparison's pending
/// interrupt clears (the write to `mtimecmp` lands after it retires), then
/// waits for the timer in WFI with interrupts globally off, then spins.
fn wait_for_the_timer(clint_base: u64) -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    let clint_mtimecmp_upper = ((clint_base + MTIMECMP) >> 12) as i32;
    ending_in_spin(vec![
        i().lui(T0, clint_mtimecmp_upper).build(),
        i().addi(T1, 0, WAKE_AT as i32).build(),
        i().sd(T0, T1, 0).build(),
        i().csrrs(T2, MIP, 0).build(),
        i().andi(T2, T2, crate::isa::csr::MIP_MTIP as i32).build(),
        i().bne(T2, 0, -8).build(),
        i().addi(T2, 0, crate::isa::csr::MIE_MTIE as i32).build(),
        i().csrrs(0, MIE, T2).build(),
        crate::isa::encoding::privileged::WFI,
        // A waiting hart's PC is already past the WFI; the run stops after
        // this, which runs once it wakes.
        i().addi(T2, 0, 0).build(),
    ])
}

fn a_wfi_counts_the_cycles_it_waited_with_or_without_idle_skipping(rec: &mut Recorder) {
    for backend in BACKENDS {
        let mut waited = Vec::new();
        for skip in [true, false] {
            let context = format!("{backend:?} skipping idle cycles: {skip}");
            let (program, end) = wait_for_the_timer(config(backend).system.clint_base);
            let mut ctx = system(backend, &program, &[]);
            ctx.sim.set_skip_idle_cores(skip);
            let straight = run(&config(backend), straight_line(), &context);

            run_to_pc(&mut ctx, end, &context);

            let wfi = rec.read(&ctx.sim, "core0.pipeline.cycles.wfi");
            let due = WAKE_AT * ctx.sim.state.config.system.clint_divider;
            assert!((due - 100..=due).contains(&wfi), "{context}: {wfi} cycles, due at {due}");
            let total = rec.read(&ctx.sim, "core0.pipeline.cycles.total");
            let by_width: u64 = ["zero", "one", "two", "three_plus"]
                .map(|n| rec.read(&ctx.sim, &format!("core0.commit.retire_histogram.{n}")))
                .iter()
                .sum();
            assert_eq!(by_width, total, "{context}");
            rec.expect(&straight.sim, "core0.pipeline.cycles.wfi", 0, &context);
            waited.push(wfi);
        }
        assert_eq!(waited[0], waited[1], "{backend:?}: skipping idle cycles changes nothing");
    }
}

accounting_checks!(
    a_mispredicted_branch_flushes_once_under_branch,
    traps_flush_under_trap_and_returns_under_system,
    a_load_that_passed_an_aliasing_store_flushes_under_mem_violations,
    every_flush_counts_the_rob_entries_it_dropped,
    a_dependent_divide_chain_stalls_issue_on_data_and_independent_ops_do_not,
    independent_divides_stall_on_the_one_divider_not_on_data,
    a_system_op_waits_to_be_oldest_under_ordering,
    rename_waits_behind_a_csr_access_on_out_of_order_only,
    branches_wait_for_a_checkpoint_only_when_they_run_out,
    rename_waits_for_a_full_rob_under_dispatch,
    a_mispredicted_loop_counts_control_and_squash_cycles,
    fetch_waits_longer_on_slower_memory,
    memory_ops_held_behind_a_page_walk_count_as_backpressure,
    a_wfi_counts_the_cycles_it_waited_with_or_without_idle_skipping,
);
