//! Instruction decoding: an instruction word to the control signals that
//! say what it does.

mod scalar;
mod vector;

use crate::common::error::Trap;
use crate::core::exec::signals::{AluOp, ControlSignals, OpASrc, OpBSrc, VectorOp};
use crate::core::units::vpu::mem::{
    is_vec_load, is_vec_store, vec_mem_dst_count, vec_mem_emul_regs,
};
use crate::isa::decode::decode as instruction_decode;
use crate::isa::instruction::Decoded;
use crate::isa::rvv::opcodes as v_opcodes;
use crate::isa::vector::{VtypeFields, parse_vtype};

/// Vector load/store width encoding for EEW=8.
pub(super) const VEC_WIDTH_8: u32 = 0b000;

/// Vector load/store width encoding for EEW=16.
pub(super) const VEC_WIDTH_16: u32 = 0b101;

/// Vector load/store width encoding for EEW=32.
pub(super) const VEC_WIDTH_32: u32 = 0b110;

/// Vector load/store width encoding for EEW=64.
pub(super) const VEC_WIDTH_64: u32 = 0b111;

/// Decodes a single instruction into control signals.
fn decode_instruction(inst: u32, pc: u64, d: &Decoded) -> Result<ControlSignals, Trap> {
    let mut c = ControlSignals {
        a_src: OpASrc::Reg1,
        b_src: OpBSrc::Imm,
        alu: AluOp::Add,
        ..Default::default()
    };

    match d.opcode {
        v_opcodes::OP_V | v_opcodes::OP_V_CRYPTO => vector::decode(&mut c, inst, d)?,
        _ => scalar::decode(&mut c, inst, pc, d)?,
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
    /// The illegal-instruction trap decoding found, if any.
    pub trap: Option<Trap>,
}

/// Decodes the (expanded) instruction `inst` at `pc` while `vtype` holds
/// `vtype_bits`.
///
/// A vector instruction takes its register-group size from `vtype`, and is
/// illegal when its register groups are misaligned for it.
#[must_use]
pub fn decode_inst(inst: u32, pc: u64, vtype_bits: u64) -> DecodedInst {
    let fields = instruction_decode(inst);
    let (mut ctrl, mut trap) = match decode_instruction(inst, pc, &fields) {
        Ok(ctrl) => (ctrl, None),
        Err(trap) => (ControlSignals::default(), Some(trap)),
    };
    let vtype = parse_vtype(vtype_bits);
    if ctrl.vec_op != VectorOp::None && !ctrl.vec_op.is_config() && !vtype.vill {
        ctrl.vec_lmul_regs = vtype.vlmul.group_regs().regs();
        ctrl.vec_lmul_is_fractional = vtype.vlmul.is_fractional();
        if trap.is_none() && vector_groups_misaligned(&ctrl, vtype) {
            ctrl = ControlSignals::default();
            trap = Some(Trap::IllegalInstruction(inst));
        }
    }
    DecodedInst { fields, ctrl, trap }
}

/// Whether a vector instruction's register groups break RVV 1.0 §3.4.2
/// under `vtype`: each group must start at a multiple of its size and fit
/// within v0..v31. Memory operations align each field's group and must fit
/// all `nf` fields.
fn vector_groups_misaligned(ctrl: &ControlSignals, vtype: VtypeFields) -> bool {
    let misaligned = |reg: u8, group: u8| {
        group > 1 && (!reg.is_multiple_of(group) || reg.saturating_add(group) > 32)
    };
    let vd = ctrl.vd.as_u8();
    let vs2 = ctrl.vs2.as_u8();
    if is_vec_load(ctrl.vec_op) || is_vec_store(ctrl.vec_op) {
        let total =
            vec_mem_dst_count(ctrl.vec_op, ctrl.vec_eew, vtype.vsew, vtype.vlmul, ctrl.vec_nf);
        let (data_emul, index_emul) =
            vec_mem_emul_regs(ctrl.vec_op, ctrl.vec_eew, vtype.vsew, vtype.vlmul);
        let vd_misaligned = data_emul > 1 && !vd.is_multiple_of(data_emul);
        return vd_misaligned || vd.saturating_add(total) > 32 || misaligned(vs2, index_emul);
    }
    let groups = ctrl.vec_op.operand_groups(
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
