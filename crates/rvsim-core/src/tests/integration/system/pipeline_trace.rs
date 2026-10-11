//! The pipeline trace sees every instruction from fetch to retirement or
//! squash, in stage order, on both backends; the Kanata writer turns it
//! into the log Konata reads.

use std::sync::{Arc, Mutex};

use crate::common::{HartId, InstSeq, SimError};
use crate::config::{BackendKind, Config, Console};
use crate::tests::support::builder::instruction::{InstructionBuilder, exit_with_a0};
use crate::tests::support::harness::TestContext;
use crate::uarch::pipeline::trace::kanata::KanataWriter;
use crate::uarch::pipeline::trace::{Departure, PipelineTrace, Stage};

const PROGRAM_BASE: u64 = 0x8000_0000;
const T0: u32 = 5;
const T1: u32 = 6;
const A0: u32 = 10;

/// What the trace was told about one instruction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Seen {
    pc: Option<u64>,
    inst: Option<u32>,
    stages: Vec<Stage>,
    left: Option<Departure>,
}

type Key = (HartId, InstSeq);

/// A trace that remembers everything, shared with the test that reads it.
#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<(Key, Seen)>>>);

impl Recording {
    fn with(&self, hart: HartId, seq: InstSeq, update: impl FnOnce(&mut Seen)) {
        let mut seen = self.0.lock().expect("the recording is free");
        let key = (hart, seq);
        if let Some((_, entry)) = seen.iter_mut().find(|(k, _)| *k == key) {
            update(entry);
        } else {
            let mut entry = Seen::default();
            update(&mut entry);
            seen.push((key, entry));
        }
    }

    fn all(&self) -> Vec<Seen> {
        self.0.lock().expect("the recording is free").iter().map(|(_, s)| s.clone()).collect()
    }

    fn all_with_seq(&self) -> Vec<(InstSeq, Seen)> {
        self.0
            .lock()
            .expect("the recording is free")
            .iter()
            .map(|((_, seq), s)| (*seq, s.clone()))
            .collect()
    }
}

impl PipelineTrace for Recording {
    fn fetched(&mut self, _cycle: u64, hart: HartId, seq: InstSeq, pc: u64) {
        self.with(hart, seq, |seen| {
            seen.pc = Some(pc);
            seen.stages.push(Stage::Fetch);
        });
    }

    fn decoded(&mut self, _cycle: u64, hart: HartId, seq: InstSeq, inst: u32) {
        self.with(hart, seq, |seen| seen.inst = Some(inst));
    }

    fn entered(&mut self, _cycle: u64, hart: HartId, seq: InstSeq, stage: Stage) {
        self.with(hart, seq, |seen| {
            if seen.stages.last() != Some(&stage) {
                seen.stages.push(stage);
            }
        });
    }

    fn left(&mut self, _cycle: u64, hart: HartId, seq: InstSeq, how: Departure) {
        self.with(hart, seq, |seen| seen.left = Some(how));
    }

    fn flush(&mut self) -> Result<(), SimError> {
        Ok(())
    }
}

/// Three `addi`s, a load, then the exit.
fn straight_line() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T0, 0, 1).build(),
        i().addi(T0, T0, 2).build(),
        i().addi(T1, T0, 3).build(),
        i().auipc(A0, 0).build(),
        i().ld(A0, A0, 0).build(),
        i().addi(A0, 0, 0).build(),
    ];
    program.extend(exit_with_a0());
    program
}

/// A loop whose exit branch is mispredicted once, so some fetched
/// instructions are squashed.
fn mispredicted_loop() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().addi(T0, 0, 20).build(),
        i().addi(T0, T0, -1).build(),
        i().bne(T0, 0, -4).build(),
        i().addi(A0, 0, 0).build(),
    ];
    program.extend(exit_with_a0());
    program
}

fn traced_run(backend: BackendKind, program: &[u32]) -> Recording {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, program);
    let recording = Recording::default();
    ctx.sim.state.uncore.pipeline_trace = Some(Box::new(recording.clone()));
    ctx.run(20_000);
    assert_eq!(ctx.sim.state.check_exit(), Some(0), "{backend:?}: the program exits");
    recording
}

/// Every instruction passes its stages in pipeline order and leaves once.
#[test]
fn every_retired_instruction_passes_its_stages_in_order() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let seen = traced_run(backend, &straight_line()).all();

        let retired: Vec<&Seen> =
            seen.iter().filter(|s| s.left == Some(Departure::Retired)).collect();
        assert_eq!(retired.len(), straight_line().len(), "{backend:?}: {seen:?}");
        for (index, inst) in retired.iter().enumerate() {
            assert_eq!(inst.pc, Some(PROGRAM_BASE + 4 * index as u64), "{backend:?}");
            assert_eq!(inst.inst, Some(straight_line()[index]), "{backend:?}");
            let order = [
                Stage::Fetch,
                Stage::Decode,
                Stage::Rename,
                Stage::Dispatch,
                Stage::Issue,
                Stage::Execute,
                Stage::Memory,
                Stage::Writeback,
                Stage::Commit,
            ];
            let positions: Vec<usize> =
                inst.stages.iter().map(|s| order.iter().position(|o| o == s).unwrap()).collect();
            assert!(positions.windows(2).all(|w| w[0] < w[1]), "{backend:?}: {:?}", inst.stages);
            assert_eq!(inst.stages.first(), Some(&Stage::Fetch), "{backend:?}");
            assert_eq!(inst.stages.last(), Some(&Stage::Commit), "{backend:?}");
            assert!(inst.stages.contains(&Stage::Execute), "{backend:?}: {:?}", inst.stages);
        }
        let load = retired[4];
        assert!(load.stages.contains(&Stage::Memory), "{backend:?}: {:?}", load.stages);
    }
}

/// A mispredicted branch squashes what was fetched behind it; the trace
/// sees those instructions leave as squashed, and never as retired. Only
/// what was fetched behind the exit, when the run ended, is still in.
#[test]
fn squashed_instructions_leave_the_trace_as_squashed() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let seen = traced_run(backend, &mispredicted_loop()).all_with_seq();

        let squashed = seen.iter().filter(|(_, s)| s.left == Some(Departure::Squashed)).count();
        let last_retired = seen
            .iter()
            .filter(|(_, s)| s.left == Some(Departure::Retired))
            .map(|(seq, _)| *seq)
            .max()
            .expect("the exit retired");
        let stuck: Vec<InstSeq> =
            seen.iter().filter(|(_, s)| s.left.is_none()).map(|(seq, _)| *seq).collect();
        assert!(squashed > 0, "{backend:?}: a mispredicted exit squashes something");
        assert!(
            stuck.iter().all(|seq| *seq > last_retired),
            "{backend:?}: every instruction before the exit leaves: {stuck:?}"
        );
    }
}

/// The Kanata writer lays the events out as Konata reads them.
#[test]
fn the_kanata_log_has_the_header_cycles_stages_and_retirements() {
    let hart = HartId::new(0);
    let mut writer = KanataWriter::new(Vec::new(), "test.kanata").expect("the header writes");

    writer.fetched(10, hart, InstSeq::new(7), 0x8000_0000);
    writer.decoded(11, hart, InstSeq::new(7), 0x0010_0093);
    writer.entered(11, hart, InstSeq::new(7), Stage::Decode);
    writer.entered(12, hart, InstSeq::new(7), Stage::Execute);
    writer.entered(12, hart, InstSeq::new(7), Stage::Execute);
    writer.fetched(12, hart, InstSeq::new(8), 0x8000_0004);
    writer.entered(14, hart, InstSeq::new(7), Stage::Commit);
    writer.left(14, hart, InstSeq::new(7), Departure::Retired);
    writer.left(15, hart, InstSeq::new(8), Departure::Squashed);

    let log = String::from_utf8(writer.into_inner()).expect("the log is text");
    let expected = "Kanata\t0004\n\
        C=\t10\n\
        I\t0\t7\t0\n\
        L\t0\t0\t0x80000000: \n\
        L\t0\t1\thart 0 seq 7\n\
        S\t0\t0\tF\n\
        C\t1\n\
        L\t0\t0\taddi ra, zero, 1\n\
        L\t0\t1\t inst 0x00100093\n\
        E\t0\t0\tF\n\
        S\t0\t0\tDc\n\
        C\t1\n\
        E\t0\t0\tDc\n\
        S\t0\t0\tEx\n\
        I\t1\t8\t0\n\
        L\t1\t0\t0x80000004: \n\
        L\t1\t1\thart 0 seq 8\n\
        S\t1\t0\tF\n\
        C\t2\n\
        E\t0\t0\tEx\n\
        S\t0\t0\tCm\n\
        E\t0\t0\tCm\n\
        R\t0\t1\t0\n\
        C\t1\n\
        E\t1\t0\tF\n\
        R\t1\t1\t1\n";
    assert_eq!(log, expected);
}
