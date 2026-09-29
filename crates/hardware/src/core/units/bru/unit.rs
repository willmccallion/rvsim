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
use crate::core::units::bru::btb::{BranchKind, Btb, BtbHit};
use crate::core::units::bru::direction::{BranchClass, DirectionPredictor, Jump, Retired};
use crate::core::units::bru::ras::{Ras, RasHistory};

/// A control instruction as the predictor sees it: from its BTB entry at
/// fetch, or from its encoding once decode has it.
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

    /// The control instruction `inst` at `pc` is, if it is one: what
    /// decode knows once it has the encoding. `size` is its length in bytes.
    #[must_use]
    pub fn from_encoding(pc: u64, size: u64, inst: u32) -> Option<Self> {
        use crate::common::constants::{OPCODE_MASK, RD_MASK, RD_SHIFT, RS1_MASK, RS1_SHIFT};
        use crate::isa::encoding::rv64i::opcodes;
        use crate::isa::instruction::{decode_b_type_imm, decode_j_type_imm};
        use crate::isa::reg;
        use crate::isa::reg::RegIdx;

        let rd = RegIdx::new(((inst >> RD_SHIFT) & RD_MASK) as u8);
        let rs1 = RegIdx::new(((inst >> RS1_SHIFT) & RS1_MASK) as u8);
        let is_link = |reg: RegIdx| reg == reg::REG_RA || reg == reg::REG_T0;
        let link = is_link(rd).then(|| pc.wrapping_add(size));
        match inst & OPCODE_MASK {
            opcodes::OP_BRANCH => {
                Some(Self::Branch { target: pc.wrapping_add(decode_b_type_imm(inst) as u64) })
            }
            opcodes::OP_JAL => {
                Some(Self::Jump { target: pc.wrapping_add(decode_j_type_imm(inst) as u64), link })
            }
            opcodes::OP_JALR => {
                let returns = is_link(rs1) && (!is_link(rd) || rd != rs1);
                Some(Self::IndirectJump { returns, link })
            }
            _ => None,
        }
    }

    /// The kind the BTB records for it.
    pub const fn kind(self) -> BranchKind {
        match self {
            Self::Branch { .. } => BranchKind::Conditional,
            Self::Jump { link, .. } => BranchKind::Jump { call: link.is_some() },
            Self::IndirectJump { returns, link } => {
                BranchKind::Indirect { returns, call: link.is_some() }
            }
        }
    }

    /// The control instruction a BTB `hit` describes at a PC whose
    /// sequential successor is `next_pc` (what a call links).
    pub fn from_btb(hit: BtbHit, next_pc: u64) -> Self {
        let link = |call: bool| call.then_some(next_pc);
        match hit.kind {
            BranchKind::Conditional => Self::Branch { target: hit.target },
            BranchKind::Jump { call } => Self::Jump { target: hit.target, link: link(call) },
            BranchKind::Indirect { returns, call } => {
                Self::IndirectJump { returns, link: link(call) }
            }
        }
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
                let (taken, direction) = self.direction.lookup(pc, target);
                (taken.then_some(target), direction)
            }
            ControlInst::Jump { target, link } => {
                if let Some(link) = link {
                    self.ras.push(link, &mut ras);
                }
                (Some(target), self.direction.unconditional(pc, Jump::Direct))
            }
            ControlInst::IndirectJump { returns, link } => {
                let target = if returns {
                    self.ras.pop(&mut ras)
                } else {
                    self.direction
                        .indirect_target(pc)
                        .or_else(|| self.btb.lookup(pc).map(|hit| hit.target))
                };
                if let Some(link) = link {
                    self.ras.push(link, &mut ras);
                }
                (target, self.direction.unconditional(pc, Jump::Indirect))
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

    /// What the BTB holds for the control instruction at `pc`, the only
    /// thing fetch knows about it before decode.
    pub fn btb_lookup(&self, pc: u64) -> Option<BtbHit> {
        self.btb.lookup(pc)
    }

    /// True when fetch predicted instruction `seq`.
    pub fn is_predicted(&self, seq: InstSeq) -> bool {
        self.in_flight.iter().any(|record| record.seq == seq)
    }

    /// Decode found control instruction `seq` that fetch, missing it in the
    /// BTB, did not predict: undoes the predictions younger than it (made
    /// without it in the histories), predicts it, and records a taken one in
    /// the BTB. Returns its predicted target and whether younger
    /// predictions were undone.
    pub fn discover(&mut self, seq: InstSeq, pc: u64, inst: ControlInst) -> (Option<u64>, bool) {
        let squashed = self.squash_younger_than(Some(seq));
        if squashed {
            self.direction.squash_done();
        }
        let target = self.predict(seq, pc, inst);
        if let Some(target) = target {
            self.btb.update(pc, target, inst.kind());
        }
        (target, squashed)
    }

    /// Decode found that fetch followed the stale BTB target of taken
    /// control instruction `seq`; `target` is its real target.
    pub fn correct_target(&mut self, seq: InstSeq, target: u64) {
        self.mispredict(seq, true, target);
    }

    /// Decode found that instruction `seq` at `pc`, which fetch predicted
    /// from the BTB, is not a control instruction: undoes that prediction
    /// and everything younger, and drops the BTB entry.
    pub fn forget(&mut self, seq: InstSeq, pc: u64) {
        let mut squashed = false;
        while let Some(record) = self.in_flight.pop_back_if(|record| record.seq >= seq) {
            self.ras.squash(record.ras);
            self.direction.squash(&record.direction);
            squashed = true;
        }
        if squashed {
            self.direction.squash_done();
        }
        self.btb.invalidate(pc);
    }

    /// Control instruction `seq` resolved against its prediction: squashes
    /// everything younger, rewrites its own history update with its real
    /// direction, keeps its real target for commit, and teaches the BTB a
    /// taken target, as gem5 does only here.
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
        let (pc, inst) = (record.pc, record.inst);
        if taken {
            // Every kind is kept: the BTB is how fetch knows a control
            // instruction is there at all.
            self.btb.update(pc, target, inst.kind());
        }
    }

    /// Trains on every prediction up to and including `done`, the youngest
    /// instruction commit has retired (gem5's `update(doneSeqNum)`).
    pub fn commit(&mut self, done: InstSeq) {
        while let Some(record) = self.in_flight.pop_front_if(|record| record.seq <= done) {
            self.retire(&record);
        }
    }

    fn retire(&mut self, record: &PredictorHistory<P::History>) {
        let class = record.inst.class();
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
