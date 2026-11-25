//! Branch Target Buffer (BTB).
//!
//! The BTB is a set-associative cache that stores target addresses for control
//! flow instructions. It allows the fetch stage to predict the target of a
//! branch or jump before the instruction is decoded.

use crate::common::VirtAddr;
use crate::config::PipelineConfig;

/// The kind of control instruction a BTB entry records: what fetch learns
/// about the instruction at a PC before it has been decoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BranchKind {
    /// A conditional branch.
    #[default]
    Conditional,
    /// A direct jump (`jal`); `call` when it links a return address.
    Jump {
        /// Pushes a return address.
        call: bool,
    },
    /// An indirect jump (`jalr`).
    Indirect {
        /// Pops a return address.
        returns: bool,
        /// Pushes a return address.
        call: bool,
    },
}

/// What a BTB hit tells fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BtbHit {
    /// The last target the instruction went to.
    pub target: VirtAddr,
    /// The kind of control instruction.
    pub kind: BranchKind,
}

/// An entry in the Branch Target Buffer.
#[derive(Clone, Copy, Debug, Default)]
struct BtbEntry {
    /// The tag used to verify if this entry corresponds to the requested PC.
    tag: VirtAddr,
    /// The predicted target address.
    target: VirtAddr,
    /// The kind of control instruction at the tagged PC.
    kind: BranchKind,
    /// Indicates if this entry contains valid data.
    valid: bool,
}

/// Set-associative Branch Target Buffer.
#[derive(Debug)]
pub struct Btb {
    /// Flat array of entries: `num_sets * ways` elements.
    table: Vec<BtbEntry>,
    /// Number of sets (must be a power of 2).
    num_sets: usize,
    /// Number of ways (associativity).
    ways: usize,
    /// Per-set replacement pointer (round-robin index into the way).
    replace_ptr: Vec<u8>,
}

impl Btb {
    /// Creates a new set-associative Branch Target Buffer.
    ///
    /// # Arguments
    ///
    /// * `size` - Total number of entries. Must be a power of 2.
    /// * `ways` - Associativity (entries per set). Must be >= 1 and divide `size`.
    pub fn new(size: usize, ways: usize) -> Self {
        let ways = ways.max(1);
        let num_sets = (size / ways).max(1);
        debug_assert!(num_sets.is_power_of_two(), "BTB num_sets must be a power of 2");
        Self {
            table: vec![BtbEntry::default(); num_sets * ways],
            num_sets,
            ways,
            replace_ptr: vec![0; num_sets],
        }
    }

    /// Calculates the set index for a given program counter.
    #[inline]
    const fn set_index(&self, pc: VirtAddr) -> usize {
        ((pc.val() >> 2) as usize) & (self.num_sets - 1)
    }

    /// The entry for the control instruction at `pc`, if the BTB holds one.
    pub fn lookup(&self, pc: VirtAddr) -> Option<BtbHit> {
        let set = self.set_index(pc);
        let base = set * self.ways;
        self.table[base..base + self.ways]
            .iter()
            .find(|e| e.valid && e.tag == pc)
            .map(|e| BtbHit { target: e.target, kind: e.kind })
    }

    /// Drops the entry for `pc`: the instruction there is not the control
    /// instruction the BTB recorded.
    pub fn invalidate(&mut self, pc: VirtAddr) {
        let set = self.set_index(pc);
        let base = set * self.ways;
        for e in &mut self.table[base..base + self.ways] {
            if e.valid && e.tag == pc {
                e.valid = false;
            }
        }
    }

    /// Records the control instruction at `pc`: its kind and latest target.
    ///
    /// If the tag already exists in the set, updates it in place. Otherwise,
    /// replaces the first invalid entry or uses round-robin replacement.
    pub fn update(&mut self, pc: VirtAddr, target: VirtAddr, kind: BranchKind) {
        let set = self.set_index(pc);
        let base = set * self.ways;
        let entry = BtbEntry { tag: pc, target, kind, valid: true };

        for w in 0..self.ways {
            let e = &mut self.table[base + w];
            if e.valid && e.tag == pc {
                *e = entry;
                return;
            }
        }

        for w in 0..self.ways {
            let e = &mut self.table[base + w];
            if !e.valid {
                *e = entry;
                return;
            }
        }

        let victim = self.replace_ptr[set] as usize % self.ways;
        self.replace_ptr[set] = ((victim + 1) % self.ways) as u8;
        self.table[base + victim] = entry;
    }
}

/// Which BTB recognised a control instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BtbLevel {
    /// The micro-BTB in front of the main one.
    Micro,
    /// The main BTB.
    Main,
}

/// What fetch learns from the BTBs about the instruction at a PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BtbPrediction {
    /// The recorded kind and target.
    pub hit: BtbHit,
    /// The BTB that held it.
    pub level: BtbLevel,
    /// Cycles fetch idles before the target when the instruction is
    /// predicted taken.
    pub taken_bubbles: u64,
}

/// A micro-BTB and the bubbles its taken branches cost.
#[derive(Debug)]
struct MicroBtb {
    btb: Btb,
    taken_bubbles: u64,
}

/// The main BTB and, when configured, the micro-BTB looked up alongside
/// it. The main BTB learns every control instruction resolution finds; the
/// micro-BTB holds the branches most recently retired taken.
#[derive(Debug)]
pub struct TargetBuffers {
    main: Btb,
    main_taken_bubbles: u64,
    micro: Option<MicroBtb>,
}

impl TargetBuffers {
    /// The BTBs `pipeline` configures.
    pub fn new(pipeline: &PipelineConfig) -> Self {
        Self {
            main: Btb::new(pipeline.btb_size, pipeline.btb_ways),
            main_taken_bubbles: pipeline.btb_taken_bubbles,
            micro: pipeline.micro_btb.map(|micro| MicroBtb {
                btb: Btb::new(micro.size, micro.ways),
                taken_bubbles: micro.taken_bubbles,
            }),
        }
    }

    /// A main BTB of `size` entries in `ways` ways, with no bubbles and no
    /// micro-BTB.
    #[cfg(test)]
    pub fn main_only(size: usize, ways: usize) -> Self {
        Self { main: Btb::new(size, ways), main_taken_bubbles: 0, micro: None }
    }

    /// The control instruction at `pc`, from the micro-BTB when it holds
    /// it and the main BTB otherwise.
    pub fn lookup(&self, pc: VirtAddr) -> Option<BtbPrediction> {
        let micro = self.micro.as_ref().and_then(|micro| {
            micro.btb.lookup(pc).map(|hit| BtbPrediction {
                hit,
                level: BtbLevel::Micro,
                taken_bubbles: micro.taken_bubbles,
            })
        });
        micro.or_else(|| {
            self.main.lookup(pc).map(|hit| BtbPrediction {
                hit,
                level: BtbLevel::Main,
                taken_bubbles: self.main_taken_bubbles,
            })
        })
    }

    /// Records the control instruction at `pc` in the main BTB, and
    /// refreshes the micro-BTB's copy if it has one.
    pub fn update(&mut self, pc: VirtAddr, target: VirtAddr, kind: BranchKind) {
        self.main.update(pc, target, kind);
        if let Some(micro) = &mut self.micro
            && micro.btb.lookup(pc).is_some()
        {
            micro.btb.update(pc, target, kind);
        }
    }

    /// Records a control instruction that retired taken in the micro-BTB.
    pub fn record_taken(&mut self, pc: VirtAddr, target: VirtAddr, kind: BranchKind) {
        if let Some(micro) = &mut self.micro {
            micro.btb.update(pc, target, kind);
        }
    }

    /// Drops every entry for `pc`: the instruction there is not the
    /// control instruction the BTBs recorded.
    pub fn invalidate(&mut self, pc: VirtAddr) {
        self.main.invalidate(pc);
        if let Some(micro) = &mut self.micro {
            micro.btb.invalidate(pc);
        }
    }
}
