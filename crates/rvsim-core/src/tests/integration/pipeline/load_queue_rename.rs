//! Out-of-order rename takes a load-queue entry from the count as it stood
//! at the start of the cycle: an entry the oldest load frees by retiring
//! goes to a younger load the cycle after, not the same cycle.

use crate::common::VirtAddr;
use crate::config::{BackendKind, Config};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{T0, T1, X0, X10};

/// Instructions up to and including the first load.
const THROUGH_FIRST_LOAD: u64 = 2;
/// The second load's address.
const SECOND_LOAD_PC: u64 = PROGRAM_BASE + 8;

/// Two loads from the program's own words, then a spin.
fn two_loads() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(X10, 0).build(),
        i().ld(T0, X10, 0).build(),
        i().ld(T1, X10, 8).build(),
        i().jal(X0, 0).build(),
    ]
}

/// The cycle the first load retired and the cycle the second was renamed,
/// on an out-of-order core whose load queue holds one load.
fn first_retires_and_second_renames() -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.load_queue_size = 1;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &two_loads());
    let mut first_retired = None;

    for _ in 0..500 {
        ctx.run(1);
        let cycle = ctx.sim.state.uncore.kernel.cycle;
        if first_retired.is_none()
            && ctx.sim.state.harts[0].instructions_retired >= THROUGH_FIRST_LOAD
        {
            first_retired = Some(cycle);
        }
        let snapshot = ctx.sim.state.cores[0].pipeline.snapshot(1);
        let is_second = |pc: VirtAddr| pc == VirtAddr::new(SECOND_LOAD_PC);
        let renamed = snapshot.rename_issue.iter().any(|slot| is_second(slot.inst.pc))
            || snapshot.issue_queue.iter().any(|slot| is_second(slot.inst.pc));
        if renamed {
            return (first_retired.expect("the second load waited for the first"), cycle);
        }
    }
    panic!("the second load was never renamed");
}

#[test]
fn a_load_takes_a_retired_loads_entry_the_cycle_after_it_frees() {
    let (retired, renamed) = first_retires_and_second_renames();

    assert_eq!(renamed, retired + 1);
}
