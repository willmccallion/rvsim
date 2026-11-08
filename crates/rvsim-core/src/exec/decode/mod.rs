//! Instruction decoding: an instruction word to the control signals that
//! say what it does.

#![expect(
    clippy::cast_possible_truncation,
    reason = "decode slices fields and immediates out of a 32-bit instruction word"
)]

pub mod requirement;
mod scalar;
mod vector;

use crate::common::VirtAddr;
use crate::exec::compute::vector::mem::{vec_mem_dst_count, vec_mem_emul_regs};
use crate::exec::signals::{ControlSignals, OpASrc, OpBSrc};
use crate::isa::encoding::rvv::opcodes as v_opcodes;
use crate::isa::instruction::{Decoded, decode as instruction_decode};
use crate::isa::op::{AluOp, ArithOp, IntAluOp, SystemOp, VecLoadOp, VecStoreOp, VectorOp};
use crate::isa::privileged::Trap;
use crate::isa::reg;
use crate::isa::rvv::{VtypeFields, parse_vtype};

/// Vector load/store width encoding for EEW=8.
pub(super) const VEC_WIDTH_8: u32 = 0b000;

/// Vector load/store width encoding for EEW=16.
pub(super) const VEC_WIDTH_16: u32 = 0b101;

/// Vector load/store width encoding for EEW=32.
pub(super) const VEC_WIDTH_32: u32 = 0b110;

/// Vector load/store width encoding for EEW=64.
pub(super) const VEC_WIDTH_64: u32 = 0b111;

/// Why an instruction decodes to a trap instead of control signals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeFault {
    /// The opcode and function fields name no instruction the simulator
    /// implements.
    UnknownEncoding,
    /// A field holds a value its encoding reserves: a rounding mode, a mask
    /// bit or vs2 the operation fixes, a memory width or field count.
    ReservedField,
    /// A vector register group is misaligned for its size or runs past v31.
    MisplacedRegisterGroup,
    /// `vtype.vill` is set and the instruction depends on `vtype`.
    IllegalVtype,
    /// The hart is configured without the instruction's extension.
    ExtensionNotImplemented,
    /// `ebreak`.
    Breakpoint,
}

impl DecodeFault {
    /// The trap the instruction `inst` at `pc` takes: a breakpoint for
    /// `ebreak`, an illegal-instruction exception otherwise.
    #[must_use]
    pub const fn trap(self, inst: u32, pc: VirtAddr) -> Trap {
        match self {
            Self::Breakpoint => Trap::Breakpoint(pc.val()),
            Self::UnknownEncoding
            | Self::ReservedField
            | Self::MisplacedRegisterGroup
            | Self::IllegalVtype
            | Self::ExtensionNotImplemented => Trap::IllegalInstruction(inst),
        }
    }
}

/// Decodes a single instruction into control signals.
fn decode_instruction(inst: u32, d: &Decoded) -> Result<ControlSignals, DecodeFault> {
    let mut c = ControlSignals {
        a_src: OpASrc::Reg1,
        b_src: OpBSrc::Imm,
        alu: AluOp::Int(IntAluOp::Arith(ArithOp::Add)),
        ..Default::default()
    };

    match d.opcode {
        v_opcodes::OP_V | v_opcodes::OP_V_CRYPTO => vector::decode(&mut c, inst, d)?,
        _ => scalar::decode(&mut c, inst, d)?,
    }
    if let Some(op) = c.vec_op
        && vector::sets_fixed_field(op, inst)
    {
        return Err(DecodeFault::ReservedField);
    }
    Ok(c)
}

/// An instruction decoded against the vector configuration in effect.
#[derive(Clone, Debug)]
pub struct DecodedInst {
    /// Its register and immediate fields.
    pub fields: Decoded,
    /// What it does.
    pub ctrl: ControlSignals,
    /// Why it decodes to a trap, if it does.
    pub fault: Option<DecodeFault>,
}

/// Decodes the (expanded) instruction `inst` while `vtype` holds
/// `vtype_bits`.
///
/// A vector instruction takes its register-group size from `vtype`, and is
/// illegal when its register groups are misaligned for it.
#[must_use]
pub fn decode_inst(inst: u32, vtype_bits: u64) -> DecodedInst {
    let mut fields = instruction_decode(inst);
    let (mut ctrl, mut fault) = match decode_instruction(inst, &fields) {
        Ok(ctrl) => (ctrl, None),
        Err(fault) => (ControlSignals::default(), Some(fault)),
    };
    // A pseudo-operation that returns a value leaves it in a0, which is
    // its destination so that dependents wait for it.
    if let SystemOp::Pseudo(op) = ctrl.system_op
        && op.returns_value()
    {
        fields.rd = reg::REG_A0;
        ctrl.reg_write = true;
    }
    let vtype = parse_vtype(vtype_bits);
    if fault.is_none() && vtype.vill && ctrl.vec_op.is_some_and(VectorOp::depends_on_vtype) {
        ctrl = ControlSignals::default();
        fault = Some(DecodeFault::IllegalVtype);
    }
    if let Some(op) = ctrl.vec_op
        && !op.is_config()
    {
        ctrl.vec_lmul_regs = vtype.vlmul.group_regs().regs();
        ctrl.vec_lmul_is_fractional = vtype.vlmul.is_fractional();
        if fault.is_none() && vector_groups_misaligned(op, &ctrl, vtype) {
            ctrl = ControlSignals::default();
            fault = Some(DecodeFault::MisplacedRegisterGroup);
        }
    }
    DecodedInst { fields, ctrl, fault }
}

const fn is_whole_register_access(op: VectorOp) -> bool {
    matches!(
        op,
        VectorOp::Load(VecLoadOp::WholeRegister) | VectorOp::Store(VecStoreOp::WholeRegister)
    )
}

/// Whether vector instruction `op`'s register groups break RVV 1.0 §3.4.2
/// under `vtype`: each group must start at a multiple of its size and fit
/// within v0..v31. Memory operations align each field's group and must fit
/// all `nf` fields.
fn vector_groups_misaligned(op: VectorOp, ctrl: &ControlSignals, vtype: VtypeFields) -> bool {
    let misaligned = |reg: u8, group: u8| {
        group > 1 && (!reg.is_multiple_of(group) || reg.saturating_add(group) > 32)
    };
    let vd = ctrl.vd.as_u8();
    let vs2 = ctrl.vs2.as_u8();
    if matches!(op, VectorOp::Load(_) | VectorOp::Store(_)) {
        let total = vec_mem_dst_count(op, ctrl.vec_eew, vtype.vsew, vtype.vlmul, ctrl.vec_nf);
        let (data_emul, index_emul) = vec_mem_emul_regs(op, ctrl.vec_eew, vtype.vsew, vtype.vlmul);
        // A whole-register access's registers must start at a multiple of
        // their count (RVV 1.0 §7.9).
        let alignment = if is_whole_register_access(op) { total } else { data_emul };
        let vd_misaligned = alignment > 1 && !vd.is_multiple_of(alignment);
        return vd_misaligned || vd.saturating_add(total) > 32 || misaligned(vs2, index_emul);
    }
    let groups = op.operand_groups(
        ctrl.vec_lmul_regs,
        ctrl.vec_lmul_is_fractional,
        ctrl.vec_src_encoding,
        ctrl.vec_nf,
        ctrl.vec_broadcast_vs2,
    );
    misaligned(vd, groups.vd)
        || misaligned(vs2, groups.vs2)
        || misaligned(ctrl.vs1.as_u8(), groups.vs1)
}
