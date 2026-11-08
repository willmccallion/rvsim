//! Disassembly of the scalar instructions.

use super::{freg, reg, xreg};
use crate::exec::signals::{BranchCond, ControlFlow, ControlSignals, JumpKind, OpASrc, OpBSrc};
use crate::isa::fence::{Fence, FenceSet};
use crate::isa::fp::FpFormat;
use crate::isa::instruction::{Decoded, InstructionBits};
use crate::isa::op::pseudo::PseudoOp;
use crate::isa::op::{
    AluOp, AmoOp, ArithOp, AtomicOp, BitOp, ClmulOp, CountOp, CsrOp, DivOp, FmaSigns, FpArithOp,
    FpCompareOp, FpOp, IntAluOp, IntWidth, LogicOp, MemWidth, MinMaxOp, MulOp, OperandFormat,
    PackOp, RotateOp, ShiftAddOp, ShiftOp, SignInjectOp, Signedness, SingleBitOp, SystemOp,
};

/// The low six bits of an immediate shift or bit-index: the shift amount.
const SHAMT_MASK: i64 = 0x3F;

/// The bits of a U-type immediate the instruction encodes, `imm[31:12]`.
const UPPER_IMMEDIATE_MASK: i64 = 0xF_FFFF;

/// Disassembles a scalar instruction decode turned into `ctrl`, or `None`
/// for control signals no instruction decodes to.
pub(super) fn disassemble(ctrl: &ControlSignals, fields: &Decoded) -> Option<String> {
    if let Some(op) = ctrl.atomic_op {
        return Some(atomic(op, ctrl, fields));
    }
    if ctrl.system_op != SystemOp::None {
        return system(ctrl, fields);
    }
    match ctrl.control_flow {
        ControlFlow::Branch(cond) => return Some(branch(cond, fields)),
        ControlFlow::Jump(kind) => return Some(jump(kind, fields)),
        ControlFlow::Sequential => {}
    }
    if ctrl.mem_read {
        return Some(load(ctrl, fields));
    }
    if ctrl.mem_write {
        return Some(store(ctrl, fields));
    }
    match ctrl.alu {
        AluOp::Int(op) => Some(integer(op, ctrl, fields)),
        AluOp::Fp(op) => floating_point(op, ctrl, fields),
    }
}

/// `beq rs1, rs2, offset` and the other branches.
fn branch(cond: BranchCond, fields: &Decoded) -> String {
    let name = match cond {
        BranchCond::Eq => "beq",
        BranchCond::Ne => "bne",
        BranchCond::Lt => "blt",
        BranchCond::Ge => "bge",
        BranchCond::Ltu => "bltu",
        BranchCond::Geu => "bgeu",
    };
    format!("{name} {}, {}, {}", xreg(fields.rs1), xreg(fields.rs2), fields.imm)
}

/// `jal rd, offset` or `jalr rd, offset(rs1)`.
fn jump(kind: JumpKind, fields: &Decoded) -> String {
    match kind {
        JumpKind::Jal => format!("jal {}, {}", xreg(fields.rd), fields.imm),
        JumpKind::Jalr => {
            format!("jalr {}, {}({})", xreg(fields.rd), fields.imm, xreg(fields.rs1))
        }
    }
}

/// An integer or floating-point load: `ld rd, offset(rs1)`.
fn load(ctrl: &ControlSignals, fields: &Decoded) -> String {
    let (name, rd) = if ctrl.fp_reg_write {
        (fp_load_name(ctrl.width), freg(fields.rd))
    } else {
        (int_load_name(ctrl.width, ctrl.signed_load), xreg(fields.rd))
    };
    format!("{name} {rd}, {}({})", fields.imm, xreg(fields.rs1))
}

/// An integer or floating-point store: `sd rs2, offset(rs1)`.
fn store(ctrl: &ControlSignals, fields: &Decoded) -> String {
    let (name, rs2) = if ctrl.rs2_fp {
        (fp_store_name(ctrl.width), freg(fields.rs2))
    } else {
        (int_store_name(ctrl.width), xreg(fields.rs2))
    };
    format!("{name} {rs2}, {}({})", fields.imm, xreg(fields.rs1))
}

const fn int_load_name(width: MemWidth, signed: bool) -> &'static str {
    match (width, signed) {
        (MemWidth::Byte, true) => "lb",
        (MemWidth::Byte, false) => "lbu",
        (MemWidth::Half, true) => "lh",
        (MemWidth::Half, false) => "lhu",
        (MemWidth::Word, true) => "lw",
        (MemWidth::Word, false) => "lwu",
        (MemWidth::Double | MemWidth::Nop, _) => "ld",
    }
}

const fn int_store_name(width: MemWidth) -> &'static str {
    match width {
        MemWidth::Byte => "sb",
        MemWidth::Half => "sh",
        MemWidth::Word => "sw",
        MemWidth::Double | MemWidth::Nop => "sd",
    }
}

const fn fp_load_name(width: MemWidth) -> &'static str {
    match width {
        MemWidth::Half => "flh",
        MemWidth::Word => "flw",
        MemWidth::Byte | MemWidth::Double | MemWidth::Nop => "fld",
    }
}

const fn fp_store_name(width: MemWidth) -> &'static str {
    match width {
        MemWidth::Half => "fsh",
        MemWidth::Word => "fsw",
        MemWidth::Byte | MemWidth::Double | MemWidth::Nop => "fsd",
    }
}

/// `lr.w rd, (rs1)`, or `sc`/`amo*` with rs2, and the ordering suffix.
fn atomic(op: AtomicOp, ctrl: &ControlSignals, fields: &Decoded) -> String {
    let width = if ctrl.width == MemWidth::Double { "d" } else { "w" };
    let ordering = match (ctrl.acquire, ctrl.release) {
        (true, true) => ".aqrl",
        (true, false) => ".aq",
        (false, true) => ".rl",
        (false, false) => "",
    };
    let (rd, rs1) = (xreg(fields.rd), xreg(fields.rs1));
    let name = match op {
        AtomicOp::Lr => return format!("lr.{width}{ordering} {rd}, ({rs1})"),
        AtomicOp::Sc => "sc",
        AtomicOp::Amo(amo) => amo_name(amo),
    };
    format!("{name}.{width}{ordering} {rd}, {}, ({rs1})", xreg(fields.rs2))
}

const fn amo_name(op: AmoOp) -> &'static str {
    match op {
        AmoOp::Swap => "amoswap",
        AmoOp::Add => "amoadd",
        AmoOp::Xor => "amoxor",
        AmoOp::And => "amoand",
        AmoOp::Or => "amoor",
        AmoOp::Min => "amomin",
        AmoOp::Max => "amomax",
        AmoOp::MinU => "amominu",
        AmoOp::MaxU => "amomaxu",
    }
}

/// The privileged, fence, cache-block, CSR and pseudo instructions.
fn system(ctrl: &ControlSignals, fields: &Decoded) -> Option<String> {
    let rs1 = xreg(fields.rs1);
    Some(match ctrl.system_op {
        SystemOp::Csr => return csr(ctrl, fields),
        SystemOp::Ecall => "ecall".to_owned(),
        SystemOp::Mret => "mret".to_owned(),
        SystemOp::Sret => "sret".to_owned(),
        SystemOp::Wfi => "wfi".to_owned(),
        SystemOp::FenceI => "fence.i".to_owned(),
        SystemOp::Fence(fence) => fence_text(fence),
        SystemOp::SfenceVma => format!("sfence.vma {rs1}, {}", xreg(fields.rs2)),
        SystemOp::CboZero => format!("cbo.zero ({rs1})"),
        SystemOp::CboInval => format!("cbo.inval ({rs1})"),
        SystemOp::CboClean => format!("cbo.clean ({rs1})"),
        SystemOp::CboFlush => format!("cbo.flush ({rs1})"),
        SystemOp::Pseudo(op) => pseudo_text(op),
        SystemOp::None => return None,
    })
}

/// `csrrw rd, csr, rs1`, or an immediate form with its 5-bit `uimm`.
fn csr(ctrl: &ControlSignals, fields: &Decoded) -> Option<String> {
    let (name, source) = match ctrl.csr_op {
        CsrOp::Rw => ("csrrw", xreg(fields.rs1).to_owned()),
        CsrOp::Rs => ("csrrs", xreg(fields.rs1).to_owned()),
        CsrOp::Rc => ("csrrc", xreg(fields.rs1).to_owned()),
        CsrOp::Rwi => ("csrrwi", fields.rs1.as_u8().to_string()),
        CsrOp::Rsi => ("csrrsi", fields.rs1.as_u8().to_string()),
        CsrOp::Rci => ("csrrci", fields.rs1.as_u8().to_string()),
        CsrOp::None => return None,
    };
    Some(format!("{name} {}, {:#05x}, {source}", xreg(fields.rd), ctrl.csr_addr.as_u32()))
}

/// `fence pred,succ`, each set as the letters of `iorw` it orders.
fn fence_text(fence: Fence) -> String {
    format!("fence {},{}", fence_set_text(fence.pred), fence_set_text(fence.succ))
}

/// The letters of `iorw` a set orders, or `0` for an empty set.
fn fence_set_text(set: FenceSet) -> String {
    let letters: String = [(set.i, 'i'), (set.o, 'o'), (set.r, 'r'), (set.w, 'w')]
        .into_iter()
        .filter_map(|(ordered, letter)| ordered.then_some(letter))
        .collect();
    if letters.is_empty() { "0".to_owned() } else { letters }
}

/// A gem5 m5op, by the function its immediate selects.
fn pseudo_text(op: PseudoOp) -> String {
    let name = match op {
        PseudoOp::Rpns => "rpns",
        PseudoOp::Exit => "exit",
        PseudoOp::Fail => "fail",
        PseudoOp::Sum => "sum",
        PseudoOp::ResetStats => "reset_stats",
        PseudoOp::DumpStats => "dump_stats",
        PseudoOp::DumpResetStats => "dump_reset_stats",
        PseudoOp::Checkpoint => "checkpoint",
        PseudoOp::WorkBegin => "work_begin",
        PseudoOp::WorkEnd => "work_end",
        PseudoOp::WriteFile => "write_file",
        PseudoOp::ReadFile => "read_file",
        PseudoOp::DebugBreak => "debug_break",
        PseudoOp::SwitchCpu => "switch_cpu",
        PseudoOp::Panic => "panic",
        PseudoOp::Unknown(function) => return format!("m5op {function:#04x}"),
    };
    format!("m5_{name}")
}

/// An integer ALU instruction in its register, immediate, unary or upper-
/// immediate form.
fn integer(op: IntAluOp, ctrl: &ControlSignals, fields: &Decoded) -> String {
    let (rd, rs1) = (xreg(fields.rd), xreg(fields.rs1));
    match (ctrl.a_src, ctrl.b_src) {
        (OpASrc::Zero, _) => {
            format!("lui {rd}, {:#x}", (fields.imm >> 12) & UPPER_IMMEDIATE_MASK)
        }
        (OpASrc::Pc, _) => {
            format!("auipc {rd}, {:#x}", (fields.imm >> 12) & UPPER_IMMEDIATE_MASK)
        }
        (OpASrc::Reg1, _) if is_unary(op) => format!("{} {rd}, {rs1}", int_name(op, ctrl)),
        (OpASrc::Reg1, OpBSrc::Imm) => {
            let imm = if takes_shift_amount(op) { fields.imm & SHAMT_MASK } else { fields.imm };
            format!("{} {rd}, {rs1}, {imm}", int_immediate_name(op, ctrl))
        }
        (OpASrc::Reg1, OpBSrc::Reg2 | OpBSrc::Zero) => {
            format!("{} {rd}, {rs1}, {}", int_name(op, ctrl), xreg(fields.rs2))
        }
    }
}

/// True for the Zbb/Zbkb ops with one source, whose immediate field
/// selects the operation instead of holding an operand.
const fn is_unary(op: IntAluOp) -> bool {
    matches!(
        op,
        IntAluOp::Bit(
            BitOp::Count(_)
                | BitOp::SextB
                | BitOp::SextH
                | BitOp::OrcB
                | BitOp::Rev8
                | BitOp::Brev8
        )
    )
}

/// True for the immediate forms whose immediate is a shift amount or bit
/// index, with the operation's selector in the bits above it.
const fn takes_shift_amount(op: IntAluOp) -> bool {
    matches!(
        op,
        IntAluOp::Shift(_)
            | IntAluOp::Bit(
                BitOp::Rotate(_) | BitOp::SingleBit(_) | BitOp::ShiftAdd(ShiftAddOp::SlliUw)
            )
    )
}

/// The register form's name, with `w` for a word operation.
fn int_name(op: IntAluOp, ctrl: &ControlSignals) -> String {
    with_word_suffix(int_op_name(op), op, ctrl)
}

/// The immediate form's name: `addi`, `sltiu`, `slli.uw`, `roriw`.
fn int_immediate_name(op: IntAluOp, ctrl: &ControlSignals) -> String {
    let name = match op {
        IntAluOp::Logic(LogicOp::Sltu) => "sltiu".to_owned(),
        IntAluOp::Bit(BitOp::ShiftAdd(ShiftAddOp::SlliUw)) => "slli.uw".to_owned(),
        _ => format!("{}i", int_op_name(op)),
    };
    with_word_suffix(&name, op, ctrl)
}

/// `name` with the `w` of an OP-32/OP-IMM-32 operation. `packw` is an
/// operation of its own, not a word form of `pack`.
fn with_word_suffix(name: &str, op: IntAluOp, ctrl: &ControlSignals) -> String {
    let is_word = ctrl.format == OperandFormat::Int(IntWidth::Word)
        && !matches!(op, IntAluOp::Bit(BitOp::Pack(_)));
    if is_word { format!("{name}w") } else { name.to_owned() }
}

/// The name of an integer operation's register form.
const fn int_op_name(op: IntAluOp) -> &'static str {
    match op {
        IntAluOp::Arith(ArithOp::Add) => "add",
        IntAluOp::Arith(ArithOp::Sub) => "sub",
        IntAluOp::Logic(op) => match op {
            LogicOp::And => "and",
            LogicOp::Or => "or",
            LogicOp::Xor => "xor",
            LogicOp::Slt => "slt",
            LogicOp::Sltu => "sltu",
        },
        IntAluOp::Shift(op) => match op {
            ShiftOp::Sll => "sll",
            ShiftOp::Srl => "srl",
            ShiftOp::Sra => "sra",
        },
        IntAluOp::Mul(op) => match op {
            MulOp::Mul => "mul",
            MulOp::Mulh => "mulh",
            MulOp::Mulhsu => "mulhsu",
            MulOp::Mulhu => "mulhu",
        },
        IntAluOp::Div(op) => match op {
            DivOp::Div => "div",
            DivOp::Divu => "divu",
            DivOp::Rem => "rem",
            DivOp::Remu => "remu",
        },
        IntAluOp::Bit(op) => bit_op_name(op),
    }
}

const fn bit_op_name(op: BitOp) -> &'static str {
    match op {
        BitOp::ShiftAdd(op) => match op {
            ShiftAddOp::Sh1Add => "sh1add",
            ShiftAddOp::Sh2Add => "sh2add",
            ShiftAddOp::Sh3Add => "sh3add",
            ShiftAddOp::AddUw => "add.uw",
            ShiftAddOp::Sh1AddUw => "sh1add.uw",
            ShiftAddOp::Sh2AddUw => "sh2add.uw",
            ShiftAddOp::Sh3AddUw => "sh3add.uw",
            ShiftAddOp::SlliUw => "slli.uw",
        },
        BitOp::Andn => "andn",
        BitOp::Orn => "orn",
        BitOp::Xnor => "xnor",
        BitOp::Count(op) => match op {
            CountOp::Clz => "clz",
            CountOp::Ctz => "ctz",
            CountOp::Cpop => "cpop",
        },
        BitOp::MinMax(op) => match op {
            MinMaxOp::Max => "max",
            MinMaxOp::Maxu => "maxu",
            MinMaxOp::Min => "min",
            MinMaxOp::Minu => "minu",
        },
        BitOp::SextB => "sext.b",
        BitOp::SextH => "sext.h",
        BitOp::Rotate(RotateOp::Rol) => "rol",
        BitOp::Rotate(RotateOp::Ror) => "ror",
        BitOp::OrcB => "orc.b",
        BitOp::Rev8 => "rev8",
        BitOp::Brev8 => "brev8",
        BitOp::Clmul(op) => match op {
            ClmulOp::Clmul => "clmul",
            ClmulOp::Clmulh => "clmulh",
            ClmulOp::Clmulr => "clmulr",
        },
        BitOp::SingleBit(op) => match op {
            SingleBitOp::Bclr => "bclr",
            SingleBitOp::Bext => "bext",
            SingleBitOp::Binv => "binv",
            SingleBitOp::Bset => "bset",
        },
        BitOp::Pack(op) => match op {
            PackOp::Pack => "pack",
            PackOp::Packh => "packh",
            PackOp::Packw => "packw",
        },
        BitOp::Xperm4 => "xperm4",
        BitOp::Xperm8 => "xperm8",
    }
}

/// A floating-point instruction, named with its format's suffix.
fn floating_point(op: FpOp, ctrl: &ControlSignals, fields: &Decoded) -> Option<String> {
    let OperandFormat::Fp(format) = ctrl.format else {
        return None;
    };
    let p = format_suffix(format);
    let rd = if ctrl.fp_reg_write { freg(fields.rd) } else { xreg(fields.rd) };
    let rs1 = reg(ctrl.src1(fields.rs1));
    let rs2 = reg(ctrl.src2(fields.rs2));
    Some(match op {
        FpOp::Arith(op) => format!("{}.{p} {rd}, {rs1}, {rs2}", fp_arith_name(op)),
        FpOp::Sqrt => format!("fsqrt.{p} {rd}, {rs1}"),
        FpOp::Fma(signs) => {
            let rs3 = freg(fields.raw.rs3());
            format!("{}.{p} {rd}, {rs1}, {rs2}, {rs3}", fma_name(signs))
        }
        FpOp::Min => format!("fmin.{p} {rd}, {rs1}, {rs2}"),
        FpOp::Max => format!("fmax.{p} {rd}, {rs1}, {rs2}"),
        FpOp::SignInject(op) => format!("{}.{p} {rd}, {rs1}, {rs2}", sign_inject_name(op)),
        FpOp::Compare(op) => format!("{}.{p} {rd}, {rs1}, {rs2}", fp_compare_name(op)?),
        FpOp::Class => format!("fclass.{p} {rd}, {rs1}"),
        FpOp::MoveToInt => format!("fmv.x.{} {rd}, {rs1}", move_suffix(format)),
        FpOp::MoveFromInt => format!("fmv.{}.x {rd}, {rs1}", move_suffix(format)),
        FpOp::ToInt(width, signedness) => {
            format!("fcvt.{}.{p} {rd}, {rs1}", int_suffix(width, signedness))
        }
        FpOp::FromInt(width, signedness) => {
            format!("fcvt.{p}.{} {rd}, {rs1}", int_suffix(width, signedness))
        }
        FpOp::FromFloat(source) => format!("fcvt.{p}.{} {rd}, {rs1}", format_suffix(source)),
    })
}

const fn format_suffix(format: FpFormat) -> &'static str {
    match format {
        FpFormat::Half => "h",
        FpFormat::Single => "s",
        FpFormat::Double => "d",
    }
}

/// `fmv`'s name for a format: single precision is `w`.
const fn move_suffix(format: FpFormat) -> &'static str {
    match format {
        FpFormat::Half => "h",
        FpFormat::Single => "w",
        FpFormat::Double => "d",
    }
}

/// `w`, `wu`, `l` or `lu`: the integer side of a conversion.
const fn int_suffix(width: IntWidth, signedness: Signedness) -> &'static str {
    match (width, signedness) {
        (IntWidth::Word, Signedness::Signed) => "w",
        (IntWidth::Word, Signedness::Unsigned) => "wu",
        (IntWidth::Double, Signedness::Signed) => "l",
        (IntWidth::Double, Signedness::Unsigned) => "lu",
    }
}

const fn fp_arith_name(op: FpArithOp) -> &'static str {
    match op {
        FpArithOp::Add => "fadd",
        FpArithOp::Sub => "fsub",
        FpArithOp::Mul => "fmul",
        FpArithOp::Div => "fdiv",
    }
}

const fn fma_name(signs: FmaSigns) -> &'static str {
    match signs {
        FmaSigns::Macc => "fmadd",
        FmaSigns::MSac => "fmsub",
        FmaSigns::NMSac => "fnmsub",
        FmaSigns::NMacc => "fnmadd",
    }
}

const fn sign_inject_name(op: SignInjectOp) -> &'static str {
    match op {
        SignInjectOp::Copy => "fsgnj",
        SignInjectOp::Negate => "fsgnjn",
        SignInjectOp::Xor => "fsgnjx",
    }
}

/// The scalar compare's name; only the vector unit has `Ne`, `Gt` and `Ge`.
const fn fp_compare_name(op: FpCompareOp) -> Option<&'static str> {
    match op {
        FpCompareOp::Eq => Some("feq"),
        FpCompareOp::Lt => Some("flt"),
        FpCompareOp::Le => Some("fle"),
        FpCompareOp::Ne | FpCompareOp::Gt | FpCompareOp::Ge => None,
    }
}
