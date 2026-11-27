//! Out-of-order rename takes a load-queue entry from the count as it stood
//! at the start of the cycle, less what it has taken since: an entry the
//! oldest load frees by retiring goes to a younger load the cycle after,
//! not the same cycle.

use crate::common::VirtAddr;
use crate::config::{BackendKind, Config};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{X0, X10, X11, X12};

const CYCLE_LIMIT: usize = 500;
const DOUBLEWORD: i32 = 8;

/// `count` loads from the program's own words after an `auipc` and a
/// divide, which holds their retirement back until each has its data. A
/// two-wide core fetches the loads in pairs.
fn loads(count: usize) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![i().auipc(X10, 0).build(), i().div(X12, X10, X10).build()];
    program.extend((0..count).map(|n| i().ld(X11, X10, DOUBLEWORD * n as i32).build()));
    program.push(i().jal(X0, 0).build());
    program
}

/// When each load retired and when each was renamed.
struct Timeline {
    retired: Vec<u64>,
    renamed: Vec<u64>,
}

/// An out-of-order core whose load queue holds `entries` loads.
fn out_of_order(entries: usize) -> Config {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.load_queue_size = entries;
    config
}

/// An out-of-order core that renames two instructions a cycle but retires
/// one.
fn renaming_two_retiring_one(entries: usize) -> Config {
    let mut config = out_of_order(entries);
    config.pipeline.width = 2;
    config.pipeline.commit_width = Some(1);
    config
}

fn timeline(config: &Config, count: usize) -> Timeline {
    let program = loads(count);
    let first_load = program.len() - 1 - count;
    let mut ctx = TestContext::with_fast_memory(config).load_program(PROGRAM_BASE, &program);
    let load_pc = |n: usize| VirtAddr::new(PROGRAM_BASE + 4 * (first_load + n) as u64);
    let mut timeline = Timeline { retired: Vec::new(), renamed: Vec::new() };

    for _ in 0..CYCLE_LIMIT {
        ctx.run(1);
        let cycle = ctx.sim.state.uncore.kernel.cycle;
        let retired = ctx.sim.state.harts[0].instructions_retired as usize;
        timeline.retired.resize(retired.saturating_sub(first_load).min(count), cycle);
        let snapshot = ctx.sim.state.cores[0].pipeline.snapshot(1);
        while timeline.renamed.len() < count {
            let pc = load_pc(timeline.renamed.len());
            let renamed = snapshot.rename_issue.iter().any(|slot| slot.inst.pc == pc)
                || snapshot.issue_queue.iter().any(|slot| slot.inst.pc == pc);
            if !renamed {
                break;
            }
            timeline.renamed.push(cycle);
        }
        if timeline.retired.len() == count {
            return timeline;
        }
    }
    panic!("the loads did not all retire in {CYCLE_LIMIT} cycles");
}

#[test]
fn a_load_takes_a_retired_loads_entry_the_cycle_after_it_frees() {
    let timeline = timeline(&out_of_order(1), 2);

    assert_eq!(timeline.renamed[1], timeline.retired[0] + 1);
}

/// One entry is free as the cycle starts and the second load retires
/// during it, so only the first of two waiting loads may take one.
#[test]
fn the_second_of_two_waiting_loads_takes_an_entry_a_cycle_later() {
    let timeline = timeline(&renaming_two_retiring_one(2), 4);

    assert_eq!(
        timeline.retired[1], timeline.renamed[2],
        "the second entry frees as the first goes"
    );
    assert_eq!(timeline.renamed[3], timeline.renamed[2] + 1);
}
