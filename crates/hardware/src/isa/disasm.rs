//! Instruction Disassembler for RISC-V RV64GC.
//!
//! Converts a 32-bit instruction encoding into a human-readable mnemonic
//! string for debug tracing, logging, and test diagnostics.
//!
//! # Supported Extensions
//!
//! - RV64I (base integer)
//! - RV64M (multiply/divide)
//! - RV64A (atomic)
//! - RV64F (single-precision float)
//! - RV64D (double-precision float)
//! - Privileged (ECALL, EBREAK, xRET, CSR, FENCE, WFI)
//!
//! # Usage
//!
//! ```ignore
//! use riscv_core::isa::disasm::disassemble;
//! let text = disassemble(0x00A00513); // ADDI x10, x0, 10
//! assert_eq!(text, "addi x10, x0, 10");
//! ```

use crate::isa::instruction::InstructionBits;
use crate::isa::privileged::opcodes as sys_op;
use crate::isa::rv64a::{funct5 as a_f5, opcodes as a_op};
use crate::isa::rv64d::funct7 as d_f7;
use crate::isa::rv64f::{funct3 as f_f3, funct7 as f_f7, opcodes as f_op};
use crate::isa::rv64i::{funct3 as i_f3, funct7 as i_f7, opcodes as i_op};
use crate::isa::rv64m::{funct3 as m_f3, opcodes as m_op};

/// ABI register names for x0–x31.
const REG_NAMES: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

/// ABI register names for f0–f31.
const FREG_NAMES: [&str; 32] = [
    "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
    "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
    "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
];

/// Returns the ABI name for an integer register index.
#[inline]
fn xreg(idx: usize) -> &'static str {
    REG_NAMES.get(idx).copied().unwrap_or("x??")
}

/// Returns the ABI name for a floating-point register index.
#[inline]
fn freg(idx: usize) -> &'static str {
    FREG_NAMES.get(idx).copied().unwrap_or("f??")
}

/// Disassembles a 32-bit RISC-V instruction into a human-readable string.
///
/// Returns a mnemonic like `"add a0, a1, a2"` or `"unknown"` for
/// unrecognised encodings.
///
/// # Arguments
///
/// * `inst` - The raw 32-bit instruction encoding.
pub fn disassemble(inst: u32) -> String {
    let opcode = inst.opcode();
    let rd = inst.rd();
    let rs1 = inst.rs1();
    let rs2 = inst.rs2();
    let f3 = inst.funct3();
    let f7 = inst.funct7();

    // Sign-extended I-type immediate.
    let imm_i = ((inst as i32) >> 20) as i64;

    // S-type immediate.
    let imm_s = {
        let lo = (inst >> 7) & 0x1F;
        let hi = (inst >> 25) & 0x7F;
        let v = (hi << 5) | lo;
        ((v as i32) << 20 >> 20) as i64
    };

    match opcode {
        // ── R-type register-register ──────────────────────
        i_op::OP_REG => disasm_op_reg(rd, rs1, rs2, f3, f7, false),
        i_op::OP_REG_32 => disasm_op_reg(rd, rs1, rs2, f3, f7, true),

        // ── I-type immediate arithmetic ───────────────────
        i_op::OP_IMM => disasm_op_imm(rd, rs1, f3, imm_i, false),
        i_op::OP_IMM_32 => disasm_op_imm(rd, rs1, f3, imm_i, true),

        // ── Loads ─────────────────────────────────────────
        i_op::OP_LOAD => {
            let mn = match f3 {
                i_f3::LB => "lb",
                i_f3::LH => "lh",
                i_f3::LW => "lw",
                i_f3::LD => "ld",
                i_f3::LBU => "lbu",
                i_f3::LHU => "lhu",
                i_f3::LWU => "lwu",
                _ => "l??",
            };
            format!("{mn} {}, {imm_i}({})", xreg(rd), xreg(rs1))
        }
        f_op::OP_LOAD_FP => {
            let mn = if f3 == i_f3::LW { "flw" } else { "fld" };
            format!("{mn} {}, {imm_i}({})", freg(rd), xreg(rs1))
        }

        // ── Stores ────────────────────────────────────────
        i_op::OP_STORE => {
            let mn = match f3 {
                i_f3::SB => "sb",
                i_f3::SH => "sh",
                i_f3::SW => "sw",
                i_f3::SD => "sd",
                _ => "s??",
            };
            format!("{mn} {}, {imm_s}({})", xreg(rs2), xreg(rs1))
        }
        f_op::OP_STORE_FP => {
            let mn = if f3 == i_f3::SW { "fsw" } else { "fsd" };
            format!("{mn} {}, {imm_s}({})", freg(rs2), xreg(rs1))
        }

        // ── Branches ──────────────────────────────────────
        i_op::OP_BRANCH => {
            let mn = match f3 {
                i_f3::BEQ => "beq",
                i_f3::BNE => "bne",
                i_f3::BLT => "blt",
                i_f3::BGE => "bge",
                i_f3::BLTU => "bltu",
                i_f3::BGEU => "bgeu",
                _ => "b??",
            };
            // Decode B-type immediate
            let imm_b = {
                let bit11 = (inst >> 7) & 1;
                let bits4_1 = (inst >> 8) & 0xF;
                let bits10_5 = (inst >> 25) & 0x3F;
                let bit12 = (inst >> 31) & 1;
                let v = (bit12 << 12) | (bit11 << 11) | (bits10_5 << 5) | (bits4_1 << 1);
                ((v as i32) << 19 >> 19) as i64
            };
            format!("{mn} {}, {}, {imm_b}", xreg(rs1), xreg(rs2))
        }

        // ── U-type ────────────────────────────────────────
        i_op::OP_LUI => {
            let imm = ((inst & 0xFFFFF000) as i32) as i64;
            format!("lui {}, {:#x}", xreg(rd), (imm >> 12) & 0xFFFFF)
        }
        i_op::OP_AUIPC => {
            let imm = ((inst & 0xFFFFF000) as i32) as i64;
            format!("auipc {}, {:#x}", xreg(rd), (imm >> 12) & 0xFFFFF)
        }

        // ── JAL ───────────────────────────────────────────
        i_op::OP_JAL => {
            let bits19_12 = (inst >> 12) & 0xFF;
            let bit11 = (inst >> 20) & 1;
            let bits10_1 = (inst >> 21) & 0x3FF;
            let bit20 = (inst >> 31) & 1;
            let v = (bit20 << 20) | (bits19_12 << 12) | (bit11 << 11) | (bits10_1 << 1);
            let imm = ((v as i32) << 11 >> 11) as i64;
            format!("jal {}, {imm}", xreg(rd))
        }

        // ── JALR ──────────────────────────────────────────
        i_op::OP_JALR => {
            format!("jalr {}, {imm_i}({})", xreg(rd), xreg(rs1))
        }

        // ── Floating-point arithmetic ─────────────────────
        f_op::OP_FP => disasm_op_fp(inst, rd, rs1, rs2, f3, f7),

        // ── FMA ───────────────────────────────────────────
        f_op::OP_FMADD => {
            let p = fp_precision(f7);
            format!(
                "fmadd.{p} {}, {}, {}, {}",
                freg(rd),
                freg(rs1),
                freg(rs2),
                freg(inst.rs3())
            )
        }
        f_op::OP_FMSUB => {
            let p = fp_precision(f7);
            format!(
                "fmsub.{p} {}, {}, {}, {}",
                freg(rd),
                freg(rs1),
                freg(rs2),
                freg(inst.rs3())
            )
        }
        f_op::OP_FNMSUB => {
            let p = fp_precision(f7);
            format!(
                "fnmsub.{p} {}, {}, {}, {}",
                freg(rd),
                freg(rs1),
                freg(rs2),
                freg(inst.rs3())
            )
        }
        f_op::OP_FNMADD => {
            let p = fp_precision(f7);
            format!(
                "fnmadd.{p} {}, {}, {}, {}",
                freg(rd),
                freg(rs1),
                freg(rs2),
                freg(inst.rs3())
            )
        }

        // ── Atomic ────────────────────────────────────────
        a_op::OP_AMO => disasm_amo(rd, rs1, rs2, f3, f7),

        // ── FENCE / System ────────────────────────────────
        i_op::OP_MISC_MEM => {
            if f3 == i_f3::FENCE_I {
                "fence.i".to_string()
            } else {
                "fence".to_string()
            }
        }

        sys_op::OP_SYSTEM => disasm_system(inst, rd, rs1, f3),

        _ => format!("unknown ({inst:#010x})"),
    }
}

/// Disassemble OP_REG / OP_REG_32 (R-type register-register).
fn disasm_op_reg(rd: usize, rs1: usize, rs2: usize, f3: u32, f7: u32, is_w: bool) -> String {
    let suffix = if is_w { "w" } else { "" };

    // M-extension
    if f7 == m_op::M_EXTENSION {
        let mn = match f3 {
            m_f3::MUL => "mul",
            m_f3::MULH => "mulh",
            m_f3::MULHSU => "mulhsu",
            m_f3::MULHU => "mulhu",
            m_f3::DIV => "div",
            m_f3::DIVU => "divu",
            m_f3::REM => "rem",
            m_f3::REMU => "remu",
            _ => "m??",
        };
        return format!("{mn}{suffix} {}, {}, {}", xreg(rd), xreg(rs1), xreg(rs2));
    }

    let mn = match (f3, f7) {
        (i_f3::ADD_SUB, i_f7::DEFAULT) => "add",
        (i_f3::ADD_SUB, i_f7::SUB) => "sub",
        (i_f3::SLL, _) => "sll",
        (i_f3::SLT, _) => "slt",
        (i_f3::SLTU, _) => "sltu",
        (i_f3::XOR, _) => "xor",
        (i_f3::SRL_SRA, i_f7::DEFAULT) => "srl",
        (i_f3::SRL_SRA, i_f7::SRA) => "sra",
        (i_f3::OR, _) => "or",
        (i_f3::AND, _) => "and",
        _ => "r??",
    };
    format!("{mn}{suffix} {}, {}, {}", xreg(rd), xreg(rs1), xreg(rs2))
}

/// Disassemble OP_IMM / OP_IMM_32 (I-type immediate arithmetic).
fn disasm_op_imm(rd: usize, rs1: usize, f3: u32, imm: i64, is_w: bool) -> String {
    let suffix = if is_w { "w" } else { "" };
    let shamt = imm & 0x3F;
    let mn = match f3 {
        i_f3::ADD_SUB => "addi",
        i_f3::SLT => "slti",
        i_f3::SLTU => "sltiu",
        i_f3::XOR => "xori",
        i_f3::OR => "ori",
        i_f3::AND => "andi",
        i_f3::SLL => return format!("slli{suffix} {}, {}, {shamt}", xreg(rd), xreg(rs1)),
        i_f3::SRL_SRA => {
            let mn = if (imm >> 10) & 1 != 0 { "srai" } else { "srli" };
            return format!("{mn}{suffix} {}, {}, {shamt}", xreg(rd), xreg(rs1));
        }
        _ => "i??",
    };
    format!("{mn}{suffix} {}, {}, {imm}", xreg(rd), xreg(rs1))
}

/// Disassemble OP_FP (floating-point arithmetic).
fn disasm_op_fp(inst: u32, rd: usize, rs1: usize, rs2: usize, f3: u32, f7: u32) -> String {
    // Determine precision from format bits (bits 26:25 of funct7)
    let is_double = (f7 & 1) != 0;
    let p = if is_double { "d" } else { "s" };

    match f7 {
        f_f7::FADD | d_f7::FADD_D => format!("fadd.{p} {}, {}, {}", freg(rd), freg(rs1), freg(rs2)),
        f_f7::FSUB | d_f7::FSUB_D => format!("fsub.{p} {}, {}, {}", freg(rd), freg(rs1), freg(rs2)),
        f_f7::FMUL | d_f7::FMUL_D => format!("fmul.{p} {}, {}, {}", freg(rd), freg(rs1), freg(rs2)),
        f_f7::FDIV | d_f7::FDIV_D => format!("fdiv.{p} {}, {}, {}", freg(rd), freg(rs1), freg(rs2)),
        f_f7::FSQRT | d_f7::FSQRT_D => format!("fsqrt.{p} {}, {}", freg(rd), freg(rs1)),
        f_f7::FSGNJ | d_f7::FSGNJ_D => {
            let mn = match f3 {
                f_f3::FSGNJ => "fsgnj",
                f_f3::FSGNJN => "fsgnjn",
                f_f3::FSGNJX => "fsgnjx",
                _ => "fsgnj?",
            };
            format!("{mn}.{p} {}, {}, {}", freg(rd), freg(rs1), freg(rs2))
        }
        f_f7::FMIN_MAX | d_f7::FMIN_MAX_D => {
            let mn = if f3 == f_f3::FMIN { "fmin" } else { "fmax" };
            format!("{mn}.{p} {}, {}, {}", freg(rd), freg(rs1), freg(rs2))
        }
        f_f7::FCMP | d_f7::FCMP_D => {
            let mn = match f3 {
                f_f3::FEQ => "feq",
                f_f3::FLT => "flt",
                f_f3::FLE => "fle",
                _ => "fcmp?",
            };
            format!("{mn}.{p} {}, {}, {}", xreg(rd), freg(rs1), freg(rs2))
        }
        f_f7::FCLASS_MV_X_F | d_f7::FCLASS_MV_X_D => {
            if f3 == f_f3::FCLASS {
                format!("fclass.{p} {}, {}", xreg(rd), freg(rs1))
            } else {
                format!(
                    "fmv.x.{} {}, {}",
                    if is_double { "d" } else { "w" },
                    xreg(rd),
                    freg(rs1)
                )
            }
        }
        f_f7::FCVT_W_F | d_f7::FCVT_W_D => {
            let variant = if rs2 == 0 {
                "w"
            } else if rs2 == 1 {
                "wu"
            } else if rs2 == 2 {
                "l"
            } else {
                "lu"
            };
            format!("fcvt.{variant}.{p} {}, {}", xreg(rd), freg(rs1))
        }
        f_f7::FCVT_F_W | d_f7::FCVT_D_W => {
            let variant = if rs2 == 0 {
                "w"
            } else if rs2 == 1 {
                "wu"
            } else if rs2 == 2 {
                "l"
            } else {
                "lu"
            };
            format!("fcvt.{p}.{variant} {}, {}", freg(rd), xreg(rs1))
        }
        f_f7::FMV_F_X | d_f7::FMV_D_X => {
            format!(
                "fmv.{}.x {}, {}",
                if is_double { "d" } else { "w" },
                freg(rd),
                xreg(rs1)
            )
        }
        f_f7::FCVT_DS => format!("fcvt.d.s {}, {}", freg(rd), freg(rs1)),
        d_f7::FCVT_S_D => format!("fcvt.s.d {}, {}", freg(rd), freg(rs1)),
        _ => {
            let _ = inst;
            format!("fp?? (funct7={f7:#04x})")
        }
    }
}

/// Disassemble AMO instruction.
fn disasm_amo(rd: usize, rs1: usize, rs2: usize, f3: u32, f7: u32) -> String {
    let suffix = if f3 == 0b011 { ".d" } else { ".w" };
    let funct5 = f7 >> 2;
    let aq = (f7 >> 1) & 1 != 0;
    let rl = f7 & 1 != 0;
    let ordering = match (aq, rl) {
        (true, true) => ".aqrl",
        (true, false) => ".aq",
        (false, true) => ".rl",
        (false, false) => "",
    };
    let mn = match funct5 {
        a_f5::LR => return format!("lr{suffix}{ordering} {}, ({})", xreg(rd), xreg(rs1)),
        a_f5::SC => "sc",
        a_f5::AMOSWAP => "amoswap",
        a_f5::AMOADD => "amoadd",
        a_f5::AMOXOR => "amoxor",
        a_f5::AMOAND => "amoand",
        a_f5::AMOOR => "amoor",
        a_f5::AMOMIN => "amomin",
        a_f5::AMOMAX => "amomax",
        a_f5::AMOMINU => "amominu",
        a_f5::AMOMAXU => "amomaxu",
        _ => "amo??",
    };
    format!(
        "{mn}{suffix}{ordering} {}, {}, ({})",
        xreg(rd),
        xreg(rs2),
        xreg(rs1)
    )
}

/// Disassemble system instructions.
fn disasm_system(inst: u32, rd: usize, rs1: usize, f3: u32) -> String {
    // Fixed-encoding system instructions
    match inst {
        sys_op::ECALL => return "ecall".to_string(),
        sys_op::EBREAK => return "ebreak".to_string(),
        sys_op::MRET => return "mret".to_string(),
        sys_op::SRET => return "sret".to_string(),
        sys_op::WFI => return "wfi".to_string(),
        _ => {}
    }

    if (inst & 0xFE007FFF) == sys_op::SFENCE_VMA {
        return format!("sfence.vma {}, {}", xreg(rs1), xreg(inst.rs2()));
    }

    // CSR instructions
    let csr = inst.csr();
    let mn = match f3 {
        sys_op::CSRRW => "csrrw",
        sys_op::CSRRS => "csrrs",
        sys_op::CSRRC => "csrrc",
        sys_op::CSRRWI => return format!("csrrwi {}, {csr:#05x}, {rs1}", xreg(rd)),
        sys_op::CSRRSI => return format!("csrrsi {}, {csr:#05x}, {rs1}", xreg(rd)),
        sys_op::CSRRCI => return format!("csrrci {}, {csr:#05x}, {rs1}", xreg(rd)),
        _ => return format!("system?? ({inst:#010x})"),
    };
    format!("{mn} {}, {csr:#05x}, {}", xreg(rd), xreg(rs1))
}

/// Determine FMA precision suffix from the format field (bits 26:25).
fn fp_precision(f7: u32) -> &'static str {
    if (f7 >> 2) & 0x3 == 0 {
        // Format bits in R4-type are bits 26:25 which map to funct7 bits 1:0
        // after the rs3 field is separated. We check bit 0 of what remains.
    }
    // For R4-type, fmt is in bits 26:25 of the instruction, which is
    // funct7 bits 1:0 once rs3 (bits 31:27) is removed.
    // Since we receive the full funct7, and rs3 occupies bits 31:27,
    // the fmt is bits 26:25. funct7 = bits 31:25, so fmt = funct7 & 0x3.
    let fmt = f7 & 0x3;
    if fmt == 1 { "d" } else { "s" }
}
