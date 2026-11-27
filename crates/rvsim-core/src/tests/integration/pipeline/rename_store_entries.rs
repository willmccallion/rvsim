//! Out-of-order rename takes a store-buffer or vector-store-buffer entry
//! from the entries free as the last cycle ended, less what it has taken
//! since, so an entry freed in one cycle is taken the next: whether a write
//! acknowledgement frees it before the engine ticks, or commit frees it
//! during the tick by handing a store's data to the write-combining buffer
//! or retiring a vector store with no elements.
//!
//! Each program fills its buffer with stores to lines of their own, then
//! has more stores wait for entries. An entry's free shows as a store
//! renamed without the buffer holding one more.

use crate::common::VirtAddr;
use crate::config::{BackendKind, Config};
use crate::isa::reg::RegIdx;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::tests::support::regs::{A0, A1, A2, A3, X0};
use crate::uarch::pipeline::engine::{ExecutionEngine, PipelineDispatch};

const DATA: u64 = PROGRAM_BASE + 0x400;
const LINE_BYTES: i32 = 64;
const CYCLE_LIMIT: usize = 1_000;
const WCB_ENTRIES: usize = 4;
const ELEMENTS: u32 = 8;

/// `vsetivli zero, vl, e8, m1, ta, ma`.
const fn vsetivli_e8(vl: u32) -> u32 {
    0xcc00_7057 | (vl << 15)
}

/// `vse8.v v1, (rs1)`.
const fn vse8_v1(rs1: RegIdx) -> u32 {
    0x0200_00a7 | (rs1.field() << 15)
}

/// The stores a program waits with.
#[derive(Clone, Copy)]
enum Stores {
    /// Doubleword stores, in the store buffer.
    Scalar,
    /// Vector stores of eight bytes, in the vector store buffer.
    Vector,
    /// Vector stores with no elements (`vl` = 0), which free their vector
    /// store buffer entries as they retire.
    EmptyVector,
}

impl Stores {
    /// `count` stores of this kind, each to a line of its own.
    fn program(self, count: usize) -> Vec<u32> {
        let i = InstructionBuilder::new;
        let bases = [A0, A1, A2, A3];
        let mut program = vec![i().auipc(A0, 0).build()];
        program.push(i().addi(A0, A0, (DATA - PROGRAM_BASE) as i32).build());
        for (&base, line) in bases[1..].iter().zip(1..) {
            program.push(i().addi(base, A0, line * LINE_BYTES).build());
        }
        match self {
            Self::Scalar => {}
            Self::Vector => program.push(vsetivli_e8(ELEMENTS)),
            Self::EmptyVector => program.push(vsetivli_e8(0)),
        }
        program.extend(bases.iter().cycle().take(count).map(|&base| match self {
            Self::Scalar => i().sd(base, X0, 0).build(),
            Self::Vector | Self::EmptyVector => vse8_v1(base),
        }));
        program
    }

    /// The entries the stores' buffer holds.
    fn occupancy(self, pipeline: &PipelineDispatch) -> usize {
        let PipelineDispatch::OutOfOrder(pipeline) = pipeline else {
            unreachable!("the tests run the out-of-order backend")
        };
        match self {
            Self::Scalar => pipeline.engine.store_buffer().len(),
            Self::Vector | Self::EmptyVector => pipeline.engine.vec_store_buffer().len(),
        }
    }
}

/// When the buffer's entries were freed and when each store was renamed.
struct Timeline {
    /// The cycle of each free, in order.
    frees: Vec<u64>,
    /// The cycle each store was renamed, in program order.
    renamed: Vec<u64>,
}

/// A program of `count` stores waiting on a buffer of `entries`.
struct Case {
    config: Config,
    stores: Stores,
    count: usize,
}

impl Case {
    fn out_of_order(stores: Stores, entries: usize, count: usize) -> Self {
        let mut config = Config::minimal();
        config.pipeline.backend = BackendKind::OutOfOrder;
        match stores {
            Stores::Scalar => config.pipeline.store_buffer_size = entries,
            Stores::Vector | Stores::EmptyVector => {
                config.pipeline.vec_store_buffer_size = entries;
            }
        }
        Self { config, stores, count }
    }

    /// An out-of-order core that renames two instructions a cycle.
    fn two_wide(stores: Stores, entries: usize, count: usize) -> Self {
        let mut case = Self::out_of_order(stores, entries, count);
        case.config.pipeline.width = 2;
        case
    }

    /// Two-wide, with a write-combining buffer that takes each store's
    /// data as it leaves the store buffer.
    fn two_wide_combining_writes(count: usize) -> Self {
        let mut case = Self::two_wide(Stores::Scalar, 2, count);
        case.config.cache.wcb_entries = WCB_ENTRIES;
        case
    }

    fn timeline(&self) -> Timeline {
        let mut program = self.stores.program(self.count);
        let first_store = program.len() - self.count;
        program.push(InstructionBuilder::new().jal(X0, 0).build());
        let mut ctx =
            TestContext::with_fast_memory(&self.config).load_program(PROGRAM_BASE, &program);
        let store_pc = |n: usize| VirtAddr::new(PROGRAM_BASE + 4 * (first_store + n) as u64);
        let mut timeline = Timeline { frees: Vec::new(), renamed: Vec::new() };

        for _ in 0..CYCLE_LIMIT {
            ctx.run(1);
            let cycle = ctx.sim.state.uncore.kernel.cycle;
            let pipeline = &ctx.sim.state.cores[0].pipeline;
            let snapshot = pipeline.snapshot(1);
            while timeline.renamed.len() < self.count {
                let pc = store_pc(timeline.renamed.len());
                let renamed = snapshot.rename_issue.iter().any(|slot| slot.inst.pc == pc)
                    || snapshot.issue_queue.iter().any(|slot| slot.inst.pc == pc);
                if !renamed {
                    break;
                }
                timeline.renamed.push(cycle);
            }
            let freed = timeline.renamed.len() - self.stores.occupancy(pipeline);
            timeline.frees.resize(freed, cycle);
            if timeline.frees.len() == self.count {
                let retired = ctx.state().harts[0].instructions_retired;
                assert!(retired >= program.len() as u64 - 1, "a store trapped instead of retiring");
                return timeline;
            }
        }
        panic!("the stores did not all drain in {CYCLE_LIMIT} cycles");
    }
}

/// The fourth store waits behind the third, renamed the cycle before the
/// second entry frees, and still takes that entry the cycle after.
#[test]
fn a_store_takes_an_acknowledged_stores_entry_the_cycle_after_it_frees() {
    let timeline = Case::out_of_order(Stores::Scalar, 2, 4).timeline();

    assert_eq!(timeline.renamed[2..], [timeline.frees[0] + 1, timeline.frees[1] + 1]);
}

#[test]
fn a_store_takes_the_entry_of_a_store_that_left_for_the_wcb_the_cycle_after() {
    let timeline = Case::two_wide_combining_writes(4).timeline();

    assert_eq!(timeline.renamed[2], timeline.frees[0] + 1);
}

/// One entry is free as the cycle starts and commit frees another during
/// it, so only the first of two waiting stores may take one.
#[test]
fn the_second_of_two_waiting_stores_takes_an_entry_a_cycle_later() {
    let timeline = Case::two_wide_combining_writes(4).timeline();

    assert_eq!(timeline.frees[1], timeline.renamed[2], "the second entry frees as the first goes");
    assert_eq!(timeline.renamed[3], timeline.renamed[2] + 1);
}

#[test]
fn a_vector_store_takes_an_acknowledged_vector_stores_entry_the_cycle_after_it_frees() {
    let timeline = Case::out_of_order(Stores::Vector, 2, 4).timeline();

    assert_eq!(timeline.renamed[2..], [timeline.frees[0] + 1, timeline.frees[1] + 1]);
}

/// Empty vector stores free their entries as they retire, one a cycle.
#[test]
fn the_second_of_two_waiting_vector_stores_takes_an_entry_a_cycle_later() {
    let timeline = Case::two_wide(Stores::EmptyVector, 2, 4).timeline();

    assert_eq!(timeline.frees[1], timeline.renamed[2], "the second entry frees as the first goes");
    assert_eq!(timeline.renamed[3], timeline.renamed[2] + 1);
}
