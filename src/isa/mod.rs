pub mod decoder;
pub mod instruction;

pub mod opcodes {
    pub const OP_LOAD: u32 = 0b0000011;
    pub const OP_IMM: u32 = 0b0010011;
    pub const OP_AUIPC: u32 = 0b0010111;
    pub const OP_IMM_32: u32 = 0b0011011;
    pub const OP_STORE: u32 = 0b0100011;
    pub const OP_REG: u32 = 0b0110011;
    pub const OP_LUI: u32 = 0b0110111;
    pub const OP_REG_32: u32 = 0b0111011;
    pub const OP_BRANCH: u32 = 0b1100011;
    pub const OP_JALR: u32 = 0b1100111;
    pub const OP_JAL: u32 = 0b1101111;
    pub const OP_SYSTEM: u32 = 0b1110011;
}

pub mod funct3 {
    pub const LB: u32 = 0b000;
    pub const LH: u32 = 0b001;
    pub const LW: u32 = 0b010;
    pub const LD: u32 = 0b011;
    pub const LBU: u32 = 0b100;
    pub const LHU: u32 = 0b101;
    pub const LWU: u32 = 0b110;

    pub const SB: u32 = 0b000;
    pub const SH: u32 = 0b001;
    pub const SW: u32 = 0b010;
    pub const SD: u32 = 0b011;

    pub const BEQ: u32 = 0b000;
    pub const BNE: u32 = 0b001;
    pub const BLT: u32 = 0b100;
    pub const BGE: u32 = 0b101;
    pub const BLTU: u32 = 0b110;
    pub const BGEU: u32 = 0b111;

    pub const ADD_SUB: u32 = 0b000;
    pub const SLL: u32 = 0b001;
    pub const SLT: u32 = 0b010;
    pub const SLTU: u32 = 0b011;
    pub const XOR: u32 = 0b100;
    pub const SRL_SRA: u32 = 0b101;
    pub const OR: u32 = 0b110;
    pub const AND: u32 = 0b111;
}

pub mod funct7 {
    pub const DEFAULT: u32 = 0b0000000;
    pub const SUB: u32 = 0b0100000;
    pub const SRA: u32 = 0b0100000;
    pub const M_EXTENSION: u32 = 0b0000001;
}
