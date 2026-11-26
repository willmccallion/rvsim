//! The free integer and floating-point physical registers, each register
//! file's on a list of its own, so rename stalls when the file its
//! destination is in runs out, whatever is free in the other.
//!
//! Both files share one index space: the integer architectural registers
//! sit at 0..32 and the FP ones at 32..64 (see [`RenameMap::new`]), then the
//! integer rename registers, then the FP ones.
//!
//! [`RenameMap::new`]: super::map::RenameMap::new

use super::free_list::FreeList;
use super::prf::PhysReg;
use crate::isa::reg::{ArchReg, RegFile, RegIdx};

/// The integer and FP architectural registers, which start mapped and are
/// never on a free list.
pub const ARCH_SCALAR_REGS: usize = 2 * RegIdx::COUNT;

/// The free integer and FP physical registers.
#[derive(Debug)]
pub struct ScalarFreeLists {
    int: FreeList<PhysReg>,
    fp: FreeList<PhysReg>,
    /// The first FP rename register; the integer ones sit below it.
    fp_renames_start: usize,
}

impl ScalarFreeLists {
    /// The free registers of an integer file of `gpr_size` and an FP file
    /// of `fpr_size` registers, each counting its 32 architectural ones.
    #[must_use]
    pub fn new(gpr_size: usize, fpr_size: usize) -> Self {
        let fp_renames_start = ARCH_SCALAR_REGS + gpr_size - RegIdx::COUNT;
        let end = fp_renames_start + fpr_size - RegIdx::COUNT;
        Self {
            int: FreeList::new((ARCH_SCALAR_REGS..fp_renames_start).map(PhysReg::from_index)),
            fp: FreeList::new((fp_renames_start..end).map(PhysReg::from_index)),
            fp_renames_start,
        }
    }

    /// A free register of `dest`'s file, or `None` when that file has none.
    pub fn allocate(&mut self, dest: ArchReg) -> Option<PhysReg> {
        self.list_for(dest).allocate()
    }

    /// Whether `dest`'s file has a register free.
    #[must_use]
    pub fn has_free(&self, dest: ArchReg) -> bool {
        match dest {
            ArchReg::Int(_) => self.int.available() > 0,
            ArchReg::Fp(_) => self.fp.available() > 0,
        }
    }

    /// Returns `reg` to its file's list; the hardwired zero stays out.
    pub fn reclaim(&mut self, reg: PhysReg) {
        if self.is_fp(reg) { self.fp.reclaim(reg) } else { self.int.reclaim(reg) }
    }

    /// The free registers of `file`.
    #[must_use]
    pub fn available_in(&self, file: RegFile) -> usize {
        match file {
            RegFile::Int => self.int.available(),
            RegFile::Fp => self.fp.available(),
        }
    }

    /// The free registers of both files.
    #[must_use]
    pub fn available(&self) -> usize {
        self.int.available() + self.fp.available()
    }

    const fn list_for(&mut self, dest: ArchReg) -> &mut FreeList<PhysReg> {
        match dest {
            ArchReg::Int(_) => &mut self.int,
            ArchReg::Fp(_) => &mut self.fp,
        }
    }

    const fn is_fp(&self, reg: PhysReg) -> bool {
        let index = reg.index();
        (RegIdx::COUNT <= index && index < ARCH_SCALAR_REGS) || index >= self.fp_renames_start
    }
}
