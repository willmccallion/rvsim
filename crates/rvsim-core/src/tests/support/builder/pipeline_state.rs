use crate::exec::signals::ControlSignals;
use crate::isa::reg::RegIdx;
use crate::uarch::pipeline::latches::{IdExEntry, IfIdEntry};

pub struct IfIdBuilder(IfIdEntry);

impl Default for IfIdBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IfIdBuilder {
    pub fn new() -> Self {
        Self(IfIdEntry::default())
    }

    pub fn pc(mut self, pc: u64) -> Self {
        self.0.pc = pc;
        self
    }

    pub fn inst(mut self, inst: u32) -> Self {
        self.0.inst = inst;
        self
    }

    pub fn predicted(mut self, target: u64) -> Self {
        self.0.pred_taken = true;
        self.0.pred_target = target;
        self
    }

    pub fn build(self) -> IfIdEntry {
        self.0
    }
}

pub struct IdExBuilder(IdExEntry);

impl Default for IdExBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IdExBuilder {
    pub fn new() -> Self {
        Self(IdExEntry::default())
    }

    pub fn pc(mut self, pc: u64) -> Self {
        self.0.inst.pc = pc;
        self
    }

    pub fn inst(mut self, inst: u32) -> Self {
        self.0.inst.bits = inst;
        self
    }

    pub fn rs1(mut self, rs1: usize, val: u64) -> Self {
        self.0.inst.rs1 = RegIdx::new(rs1 as u8);
        self.0.inst.rv1 = val;
        self
    }

    pub fn rs2(mut self, rs2: usize, val: u64) -> Self {
        self.0.inst.rs2 = RegIdx::new(rs2 as u8);
        self.0.inst.rv2 = val;
        self
    }

    pub fn rd(mut self, rd: usize) -> Self {
        self.0.inst.rd = RegIdx::new(rd as u8);
        self
    }

    pub fn imm(mut self, imm: i64) -> Self {
        self.0.inst.imm = imm;
        self
    }

    pub fn control(mut self, ctrl: ControlSignals) -> Self {
        self.0.inst.ctrl = ctrl;
        self
    }

    pub fn build(self) -> IdExEntry {
        self.0
    }
}
