use riscv_core::core::pipeline::latches::{ExMemEntry, IdExEntry, IfIdEntry, MemWbEntry};
use riscv_core::core::pipeline::signals::ControlSignals;

pub struct IfIdBuilder(IfIdEntry);

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

impl IdExBuilder {
    pub fn new() -> Self {
        Self(IdExEntry::default())
    }

    pub fn pc(mut self, pc: u64) -> Self {
        self.0.pc = pc;
        self
    }

    pub fn inst(mut self, inst: u32) -> Self {
        self.0.inst = inst;
        self
    }

    pub fn rs1(mut self, rs1: usize, val: u64) -> Self {
        self.0.rs1 = rs1;
        self.0.rv1 = val;
        self
    }

    pub fn rs2(mut self, rs2: usize, val: u64) -> Self {
        self.0.rs2 = rs2;
        self.0.rv2 = val;
        self
    }

    pub fn rd(mut self, rd: usize) -> Self {
        self.0.rd = rd;
        self
    }

    pub fn imm(mut self, imm: i64) -> Self {
        self.0.imm = imm;
        self
    }

    pub fn control(mut self, ctrl: ControlSignals) -> Self {
        self.0.ctrl = ctrl;
        self
    }

    pub fn build(self) -> IdExEntry {
        self.0
    }
}

pub struct ExMemBuilder(ExMemEntry);

impl ExMemBuilder {
    pub fn new() -> Self {
        Self(ExMemEntry::default())
    }

    pub fn pc(mut self, pc: u64) -> Self {
        self.0.pc = pc;
        self
    }

    pub fn alu_result(mut self, res: u64) -> Self {
        self.0.alu = res;
        self
    }

    pub fn store_data(mut self, data: u64) -> Self {
        self.0.store_data = data;
        self
    }

    pub fn rd(mut self, rd: usize) -> Self {
        self.0.rd = rd;
        self
    }

    pub fn control(mut self, ctrl: ControlSignals) -> Self {
        self.0.ctrl = ctrl;
        self
    }

    pub fn build(self) -> ExMemEntry {
        self.0
    }
}

pub struct MemWbBuilder(MemWbEntry);

impl MemWbBuilder {
    pub fn new() -> Self {
        Self(MemWbEntry::default())
    }

    pub fn pc(mut self, pc: u64) -> Self {
        self.0.pc = pc;
        self
    }

    pub fn rd(mut self, rd: usize) -> Self {
        self.0.rd = rd;
        self
    }

    pub fn alu_result(mut self, res: u64) -> Self {
        self.0.alu = res;
        self
    }

    pub fn load_data(mut self, data: u64) -> Self {
        self.0.load_data = data;
        self
    }

    pub fn control(mut self, ctrl: ControlSignals) -> Self {
        self.0.ctrl = ctrl;
        self
    }

    pub fn build(self) -> MemWbEntry {
        self.0
    }
}
