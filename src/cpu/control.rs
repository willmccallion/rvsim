use crate::cpu::pipeline::{EXMEM, IDEx, MEMWB};

#[derive(Clone, Copy, Debug, Default)]
pub enum AluOp {
    #[default]
    Add,
    Sub,
    Sll,
    Slt,
    Sltu,
    Xor,
    Srl,
    Sra,
    Or,
    And,
    Mul,
    Mulh,
    Mulhsu,
    Mulhu,
    Div,
    Divu,
    Rem,
    Remu,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum MemWidth {
    #[default]
    Nop,
    Byte,
    Half,
    Word,
    Double,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum OpASrc {
    #[default]
    Reg1,
    Pc,
    Zero,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum OpBSrc {
    #[default]
    Imm,
    Reg2,
    Zero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsrOp {
    None,
    RW,
    RS,
    RC,
    RWI,
    RSI,
    RCI,
}
impl Default for CsrOp {
    fn default() -> Self {
        CsrOp::None
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ControlSignals {
    pub reg_write: bool,
    pub mem_read: bool,
    pub mem_write: bool,
    pub branch: bool,
    pub jump: bool,
    pub is_rv32: bool,
    pub width: MemWidth,
    pub signed_load: bool,
    pub alu: AluOp,
    pub a_src: OpASrc,
    pub b_src: OpBSrc,
    pub is_system: bool,
    pub csr_addr: u32,
    pub is_mret: bool,
    pub is_sret: bool,
    pub csr_op: CsrOp,
}

pub fn need_stall_load_use(id_ex: &IDEx, if_id_inst: u32) -> bool {
    if !id_ex.ctrl.mem_read || id_ex.rd == 0 {
        return false;
    }

    let next_rs1 = ((if_id_inst >> 15) & 0x1f) as usize;
    let next_rs2 = ((if_id_inst >> 20) & 0x1f) as usize;

    id_ex.rd == next_rs1 || id_ex.rd == next_rs2
}

pub fn forward_rs(id_ex: &IDEx, ex_mem: &EXMEM, mem_wb: &MEMWB) -> (u64, u64) {
    let mut a = id_ex.rv1;
    let mut b = id_ex.rv2;

    if mem_wb.ctrl.reg_write && mem_wb.rd != 0 {
        let wb_val = if mem_wb.ctrl.mem_read {
            mem_wb.load_data
        } else if mem_wb.ctrl.jump {
            mem_wb.pc.wrapping_add(4)
        } else {
            mem_wb.alu
        };

        if mem_wb.rd == id_ex.rs1 {
            a = wb_val;
        }
        if mem_wb.rd == id_ex.rs2 {
            b = wb_val;
        }
    }

    if ex_mem.ctrl.reg_write && ex_mem.rd != 0 && !ex_mem.ctrl.mem_read {
        let ex_val = if ex_mem.ctrl.jump {
            ex_mem.pc.wrapping_add(4)
        } else {
            ex_mem.alu
        };

        if ex_mem.rd == id_ex.rs1 {
            a = ex_val;
        }
        if ex_mem.rd == id_ex.rs2 {
            b = ex_val;
        }
    }

    (a, b)
}
