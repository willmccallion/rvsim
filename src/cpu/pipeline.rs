use crate::cpu::control::ControlSignals;

#[derive(Clone, Copy)]
pub struct IFID {
    pub pc: u64,
    pub inst: u32,
}

impl Default for IFID {
    fn default() -> Self {
        Self {
            inst: 0x0000_0013, // ADDI x0,x0,0 (NOP)
            pc: 0,
        }
    }
}

#[derive(Default, Clone)]
pub struct IDEx {
    pub pc: u64,
    pub inst: u32,
    pub rs1: usize,
    pub rs2: usize,
    pub rd: usize,
    pub imm: i64,
    pub rv1: u64,
    pub rv2: u64,
    pub ctrl: ControlSignals,
    pub trap: Option<String>,
}

#[derive(Default, Clone)]
pub struct EXMEM {
    pub pc: u64,
    pub inst: u32,
    pub rd: usize,
    pub alu: u64,
    pub store_data: u64,
    pub ctrl: ControlSignals,
    pub trap: Option<String>,
}

#[derive(Default, Clone)]
pub struct MEMWB {
    pub pc: u64,
    pub inst: u32,
    pub rd: usize,
    pub alu: u64,
    pub load_data: u64,
    pub ctrl: ControlSignals,
    pub trap: Option<String>,
}

pub(crate) fn bubble_idex() -> IDEx {
    IDEx::default()
}
