//! The branch prediction unit, after gem5's `BPredUnit`.
//!
//! Fetch asks it for a prediction for every control instruction, numbered
//! by its [`InstSeq`]. The unit keeps one record per prediction, oldest
//! first. Commit reports the youngest instruction it retired and the
//! predictor trains on every record up to it; a squash undoes the records
//! younger than the instruction it keeps, youngest first, restoring the
//! histories and the return address stack exactly.

use std::collections::VecDeque;

use crate::common::InstSeq;
use crate::core::units::bru::btb::Btb;
use crate::core::units::bru::direction::{BranchClass, DirectionPredictor, Retired};
use crate::core::units::bru::ras::{Ras, RasHistory};

/// What fetch's predecode knows about a control instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlInst {
    /// A conditional branch and its encoded target.
    Branch {
        /// Where it goes when taken.
        target: u64,
    },
    /// A direct jump (`jal`) and its encoded target.
    Jump {
        /// Where it goes.
        target: u64,
        /// The return address a call pushes.
        link: Option<u64>,
    },
    /// An indirect jump (`jalr`).
    IndirectJump {
        /// A return, which pops the return address stack.
        returns: bool,
        /// The return address a call pushes.
        link: Option<u64>,
    },
}

impl ControlInst {
    const fn class(self) -> BranchClass {
        match self {
            Self::Branch { .. } => BranchClass::Conditional,
            Self::Jump { .. } | Self::IndirectJump { .. } => BranchClass::Unconditional,
        }
    }

    const fn predicts_indirect_target(self) -> bool {
        matches!(self, Self::IndirectJump { returns: false, .. })
    }
}

/// One prediction in flight, gem5's `PredictorHistory`.
#[derive(Debug)]
struct PredictorHistory<H> {
    seq: InstSeq,
    pc: u64,
    inst: ControlInst,
    /// The direction, as predicted and then as a misprediction corrects it.
    taken: bool,
    /// The target when taken, as predicted and then as corrected.
    target: Option<u64>,
    ras: RasHistory,
    direction: H,
}

/// A direction predictor with the BTB and return address stack it shares
/// its predictions with.
#[derive(Debug)]
pub struct BranchPredUnit<P: DirectionPredictor> {
    direction: P,
    btb: Btb,
    ras: Ras,
    in_flight: VecDeque<PredictorHistory<P::History>>,
}

impl<P: DirectionPredictor> BranchPredUnit<P> {
    /// Creates a unit around `direction`.
    pub fn new(direction: P, btb_size: usize, btb_ways: usize, ras_size: usize) -> Self {
        Self {
            direction,
            btb: Btb::new(btb_size, btb_ways),
            ras: Ras::new(ras_size),
            in_flight: VecDeque::new(),
        }
    }

    /// The direction predictor.
    pub const fn direction(&self) -> &P {
        &self.direction
    }

    /// Predicts control instruction `seq` at `pc`: its target when it is
    /// predicted taken, `None` when fetch should fall through.
    pub fn predict(&mut self, seq: InstSeq, pc: u64, inst: ControlInst) -> Option<u64> {
        let mut ras = RasHistory::default();
        let (target, direction) = match inst {
            ControlInst::Branch { target } => {
                let (taken, direction) = self.direction.lookup(pc);
                (taken.then(|| self.btb.lookup(pc).unwrap_or(target)), direction)
            }
            ControlInst::Jump { target, link } => {
                if let Some(link) = link {
                    self.ras.push(link, &mut ras);
                }
                (Some(target), self.direction.unconditional(pc))
            }
            ControlInst::IndirectJump { returns, link } => {
                let target = if returns {
                    self.ras.pop(&mut ras)
                } else {
                    self.direction.indirect_target(pc).or_else(|| self.btb.lookup(pc))
                };
                if let Some(link) = link {
                    self.ras.push(link, &mut ras);
                }
                (target, self.direction.unconditional(pc))
            }
        };
        let taken = target.is_some();
        self.direction.update_histories(pc, taken, &direction);
        self.in_flight.push_back(PredictorHistory { seq, pc, inst, taken, target, ras, direction });
        target
    }

    /// Squashes every prediction younger than `keep`.
    pub fn squash_after(&mut self, keep: InstSeq) {
        if self.squash_younger_than(Some(keep)) {
            self.direction.squash_done();
        }
    }

    /// Squashes every prediction not yet committed.
    pub fn squash_all(&mut self) {
        if self.squash_younger_than(None) {
            self.direction.squash_done();
        }
    }

    /// Control instruction `seq` resolved against its prediction: squashes
    /// everything younger, rewrites its own history update with its real
    /// direction, and keeps its real target for commit.
    pub fn mispredict(&mut self, seq: InstSeq, taken: bool, target: u64) {
        let squashed = self.squash_younger_than(Some(seq));
        let Some(record) = self.in_flight.back_mut().filter(|record| record.seq == seq) else {
            if squashed {
                self.direction.squash_done();
            }
            return;
        };
        record.taken = taken;
        record.target = taken.then_some(target);
        self.direction.correct(record.pc, taken, &record.direction);
    }

    /// Trains on every prediction up to and including `done`, the youngest
    /// instruction commit has retired (gem5's `update(doneSeqNum)`).
    pub fn commit(&mut self, done: InstSeq) {
        while let Some(record) = self.in_flight.pop_front_if(|record| record.seq <= done) {
            self.retire(&record);
        }
    }

    /// Teaches the BTB a jump's target as soon as it is known.
    pub fn update_btb(&mut self, pc: u64, target: u64) {
        self.btb.update(pc, target);
    }

    fn retire(&mut self, record: &PredictorHistory<P::History>) {
        let class = record.inst.class();
        if class == BranchClass::Conditional
            && let Some(target) = record.target
        {
            self.btb.update(record.pc, target);
        }
        let indirect_target = record.target.filter(|_| record.inst.predicts_indirect_target());
        let retired = Retired { class, taken: record.taken, indirect_target };
        self.direction.commit(record.pc, retired, &record.direction);
    }

    /// Undoes the predictions younger than `keep` (all of them for `None`);
    /// returns whether there were any.
    fn squash_younger_than(&mut self, keep: Option<InstSeq>) -> bool {
        let mut squashed = false;
        while let Some(record) =
            self.in_flight.pop_back_if(|record| keep.is_none_or(|keep| record.seq > keep))
        {
            self.ras.squash(record.ras);
            self.direction.squash(&record.direction);
            squashed = true;
        }
        squashed
    }
}
