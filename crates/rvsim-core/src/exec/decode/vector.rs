//! Vector instruction decoding.

use super::{VEC_WIDTH_8, VEC_WIDTH_16, VEC_WIDTH_32};
use crate::exec::signals::{ControlSignals, OpBSrc};
use crate::isa::encoding::rvv::{
    encoding as v_enc, funct3 as v_funct3, funct6 as v_f6, opcodes as v_opcodes,
};
use crate::isa::instruction::Decoded;
use crate::isa::op::{
    CarryOp, CompareOp, CryptoOp, ExtendFactor, ExtendOp, FmaAddend, FmaSigns, FpBinaryOp,
    FpCompareOp, FpNarrowConvertOp, FpReduceOp, FpUnaryOp, FpWidenConvertOp, FpWidenOp,
    FpWidenReduceOp, IntConvert, IntOp, IntReduceOp, IntRounding, MaccOp, MaskLogicalOp, MaskOp,
    MaskSetOp, NarrowOp, PermuteOp, ReduceOp, Signedness, SlideOffset, VecAluOp, VecConfigOp,
    VecFpOp, VecLoadOp, VecSrcEncoding, VecStoreOp, VectorOp, WholeRegisters, WidenIntReduceOp,
    WidenMaccOp, WidenOp,
};
use crate::isa::privileged::Trap;
use crate::isa::rvv::{Sew, VRegIdx};

/// The instruction bit holding the top bit of `vror.vi`'s 6-bit rotate.
const VROR_ZIMM_HIGH_BIT: u32 = 26;

/// Unit-stride lumop: normal unit-stride load.
const LUMOP_UNIT: u8 = 0b00000;

/// Unit-stride lumop: whole-register load.
const LUMOP_WHOLE_REG: u8 = 0b01000;

/// Unit-stride lumop: mask load.
const LUMOP_MASK: u8 = 0b01011;

/// Unit-stride lumop: fault-only-first.
const LUMOP_FAULT_FIRST: u8 = 0b10000;

/// Unit-stride sumop: normal unit-stride store.
const SUMOP_UNIT: u8 = 0b00000;

/// Unit-stride sumop: whole-register store.
const SUMOP_WHOLE_REG: u8 = 0b01000;

/// Unit-stride sumop: mask store.
const SUMOP_MASK: u8 = 0b01011;

/// Memory addressing mode (mop): unit-stride.
const MOP_UNIT: u8 = 0b00;

/// Memory addressing mode (mop): indexed unordered.
const MOP_INDEXED_UNORD: u8 = 0b01;

/// Memory addressing mode (mop): strided.
const MOP_STRIDED: u8 = 0b10;

/// Memory addressing mode (mop): indexed ordered.
const MOP_INDEXED_ORD: u8 = 0b11;

/// Decodes a vector instruction (RVV 1.0 and the vector crypto
/// extensions) into `c`.
pub(super) fn decode(c: &mut ControlSignals, inst: u32, d: &Decoded) -> Result<(), Trap> {
    match d.opcode {
        v_opcodes::OP_V => {
            if d.funct3 == v_funct3::OPCFG {
                // vsetvl family: all write scalar rd.
                let bit31 = (inst >> 31) & 1;
                let bit30 = (inst >> 30) & 1;

                if bit31 == 0 {
                    // vsetvli: zimm[10:0] from bits 30:20
                    c.vec_op = Some(VectorOp::Config(VecConfigOp::Vsetvli {
                        vtype: v_enc::zimm_vsetvli(inst),
                    }));
                    c.reg_write = true;
                } else if bit30 == 1 {
                    // vsetivli: zimm[9:0] from bits 29:20, uimm[4:0] from bits 19:15
                    c.vec_op = Some(VectorOp::Config(VecConfigOp::Vsetivli {
                        avl: v_enc::uimm_vsetivli(inst),
                        vtype: v_enc::zimm_vsetivli(inst),
                    }));
                    c.reg_write = true;
                } else {
                    // vsetvl: vtype from rs2
                    c.vec_op = Some(VectorOp::Config(VecConfigOp::Vsetvl));
                    c.reg_write = true;
                    c.b_src = OpBSrc::Reg2;
                }

                // Store decoded vector register indices for downstream
                c.vd = VRegIdx::new(v_enc::vd(inst));
                c.vs1 = VRegIdx::new(v_enc::vs1(inst));
                c.vs2 = VRegIdx::new(v_enc::vs2(inst));
            } else {
                // Vector arithmetic: funct6 is bits 31:26
                let f6 = v_enc::funct6(inst);
                c.vm = v_enc::vm(inst);
                c.vd = VRegIdx::new(v_enc::vd(inst));
                c.vs1 = VRegIdx::new(v_enc::vs1(inst));
                c.vs2 = VRegIdx::new(v_enc::vs2(inst));

                match d.funct3 {
                    v_funct3::OPIVV => {
                        c.vec_reg_write = true;
                        c.vec_src_encoding = VecSrcEncoding::VV;
                        c.vec_op = Some(decode_opivv(f6, inst)?);
                    }
                    v_funct3::OPIVX => {
                        c.vec_reg_write = true;
                        c.vec_src_encoding = VecSrcEncoding::VX;
                        c.vec_op = Some(decode_opivx(f6, inst)?);
                    }
                    v_funct3::OPIVI => {
                        c.vec_reg_write = true;
                        let op = decode_opivi(f6, inst)?;
                        c.vec_op = Some(op);
                        c.vec_src_encoding = VecSrcEncoding::VI(vi_immediate(op, inst));
                        // Whole-register moves: validate vd+nregs and vs2+nregs <= 32
                        if let VectorOp::Permute(PermuteOp::WholeMove(registers)) = op {
                            let nregs = registers.count();
                            let vd_raw = v_enc::vd(inst);
                            let vs2_raw = v_enc::vs2(inst);
                            if vd_raw + nregs > 32 || vs2_raw + nregs > 32 {
                                return Err(Trap::IllegalInstruction(inst));
                            }
                        }
                    }
                    v_funct3::OPMVV => {
                        c.vec_src_encoding = VecSrcEncoding::VV;
                        let (op, writes_vec) = decode_opmvv(f6, inst)?;
                        c.vec_op = Some(op);
                        c.vec_reg_write = writes_vec;
                        // Some OPMVV ops write scalar rd instead of vec
                        if matches!(
                            op,
                            VectorOp::Permute(PermuteOp::MvXS)
                                | VectorOp::Mask(MaskOp::CPop | MaskOp::First)
                        ) {
                            c.reg_write = true;
                        }
                    }
                    v_funct3::OPMVX => {
                        c.vec_src_encoding = VecSrcEncoding::VX;
                        let (op, writes_vec) = decode_opmvx(f6, inst)?;
                        c.vec_op = Some(op);
                        c.vec_reg_write = writes_vec;
                    }
                    v_funct3::OPFVV => {
                        c.vec_src_encoding = VecSrcEncoding::VV;
                        let (op, writes_vec) = decode_opfvv(f6, inst)?;
                        c.vec_op = Some(op);
                        c.vec_reg_write = writes_vec;
                        // Scalar-to-FP move (vfmv.f.s) writes FP rd, not vec
                        if matches!(op, VectorOp::Fp(VecFpOp::MoveToScalar)) {
                            c.fp_reg_write = true;
                        }
                    }
                    v_funct3::OPFVF => {
                        c.vec_src_encoding = VecSrcEncoding::VF;
                        c.rs1_fp = true;
                        let (op, writes_vec) = decode_opfvf(f6, inst)?;
                        c.vec_op = Some(op);
                        c.vec_reg_write = writes_vec;
                    }
                    _ => {
                        return Err(Trap::IllegalInstruction(inst));
                    }
                }
            }
        }
        v_opcodes::OP_V_CRYPTO => {
            // RVV crypto extensions (Zvkned/Zvknha/Zvksed/Zvksh/Zvkg).
            // All Zvk* ops use opcode 0x77 with funct3=0b010 (OPMVV) and
            // bit25=1 (vm always set; the masked encoding is reserved).
            if d.funct3 != v_funct3::OPMVV {
                return Err(Trap::IllegalInstruction(inst));
            }
            let f6 = v_enc::funct6(inst);
            c.vm = v_enc::vm(inst);
            c.vd = VRegIdx::new(v_enc::vd(inst));
            c.vs1 = VRegIdx::new(v_enc::vs1(inst));
            c.vs2 = VRegIdx::new(v_enc::vs2(inst));
            c.vec_src_encoding = VecSrcEncoding::VV;
            c.vec_reg_write = true;
            // VCRYPTO_VS = .vs form: broadcast vs2[0]; VCRYPTO_VV = .vv form: per-group key.
            c.vec_broadcast_vs2 = f6 == v_f6::VCRYPTO_VS;
            c.vec_op = Some(decode_opcrypto(f6, inst)?);
        }
        _ => return Err(Trap::IllegalInstruction(inst)),
    }
    Ok(())
}

/// Decode opcode 0x77 (Zvkn*/Zvks*/Zvkg crypto extensions).
///
/// All crypto ops use funct3=OPMVV; funct6 selects the family. The .vv
/// (per-group key) and .vs (broadcast vs2[0]) variants of the AES/SM4
/// rounds use distinct funct6 values and a shared vs1 sub-opcode set —
/// the caller must inspect the original funct6 to set
/// `vec_broadcast_vs2` accordingly.
const fn decode_opcrypto(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    let vs1 = v_enc::vs1(inst);
    Ok(match f6 {
        v_f6::VSM3_ME => VectorOp::Crypto(CryptoOp::Sm3Me),
        v_f6::VSM4_K => VectorOp::Crypto(CryptoOp::Sm4K),
        v_f6::VAES_KF1 => VectorOp::Crypto(CryptoOp::AesKf1),
        v_f6::VCRYPTO_VV => match vs1 {
            v_f6::VAES_VS1_DM => VectorOp::Crypto(CryptoOp::AesDm),
            v_f6::VAES_VS1_DF => VectorOp::Crypto(CryptoOp::AesDf),
            v_f6::VAES_VS1_EM => VectorOp::Crypto(CryptoOp::AesEm),
            v_f6::VAES_VS1_EF => VectorOp::Crypto(CryptoOp::AesEf),
            v_f6::VSM4_VS1_R => VectorOp::Crypto(CryptoOp::Sm4R),
            v_f6::VGMUL_VS1 => VectorOp::Crypto(CryptoOp::Gmul),
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        v_f6::VCRYPTO_VS => match vs1 {
            v_f6::VAES_VS1_DM => VectorOp::Crypto(CryptoOp::AesDm),
            v_f6::VAES_VS1_DF => VectorOp::Crypto(CryptoOp::AesDf),
            v_f6::VAES_VS1_EM => VectorOp::Crypto(CryptoOp::AesEm),
            v_f6::VAES_VS1_EF => VectorOp::Crypto(CryptoOp::AesEf),
            v_f6::VAES_VS1_Z => VectorOp::Crypto(CryptoOp::AesZ),
            v_f6::VSM4_VS1_R => VectorOp::Crypto(CryptoOp::Sm4R),
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        v_f6::VAES_KF2 => VectorOp::Crypto(CryptoOp::AesKf2),
        v_f6::VSM3_C => VectorOp::Crypto(CryptoOp::Sm3C),
        v_f6::VGHSH => VectorOp::Crypto(CryptoOp::Ghsh),
        v_f6::VSHA2_MS => VectorOp::Crypto(CryptoOp::Sha2Ms),
        v_f6::VSHA2_CH => VectorOp::Crypto(CryptoOp::Sha2Ch),
        v_f6::VSHA2_CL => VectorOp::Crypto(CryptoOp::Sha2Cl),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPIVV funct3 (integer vector-vector) operations.
const fn decode_opivv(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VSUB => VectorOp::Alu(VecAluOp::Int(IntOp::Sub)),
        v_f6::VMINU => VectorOp::Alu(VecAluOp::Int(IntOp::MinU)),
        v_f6::VMIN => VectorOp::Alu(VecAluOp::Int(IntOp::Min)),
        v_f6::VMAXU => VectorOp::Alu(VecAluOp::Int(IntOp::MaxU)),
        v_f6::VMAX => VectorOp::Alu(VecAluOp::Int(IntOp::Max)),
        v_f6::VAND => VectorOp::Alu(VecAluOp::Int(IntOp::And)),
        v_f6::VOR => VectorOp::Alu(VecAluOp::Int(IntOp::Or)),
        v_f6::VXOR => VectorOp::Alu(VecAluOp::Int(IntOp::Xor)),
        v_f6::VADD => VectorOp::Alu(VecAluOp::Int(IntOp::Add)),
        v_f6::VRGATHER => VectorOp::Permute(PermuteOp::Rgather),
        v_f6::VRGATHEREI16 => VectorOp::Permute(PermuteOp::RgatherEi16),
        v_f6::VADC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Adc)),
        v_f6::VMADC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Madc)),
        v_f6::VSBC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Sbc)),
        v_f6::VMSBC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Msbc)),
        v_f6::VMERGE_VMV => VectorOp::Alu(VecAluOp::Merge),
        v_f6::VMSEQ => VectorOp::Alu(VecAluOp::Compare(CompareOp::Eq)),
        v_f6::VMSNE => VectorOp::Alu(VecAluOp::Compare(CompareOp::Ne)),
        v_f6::VMSLTU => VectorOp::Alu(VecAluOp::Compare(CompareOp::LtU)),
        v_f6::VMSLT => VectorOp::Alu(VecAluOp::Compare(CompareOp::Lt)),
        v_f6::VMSLEU => VectorOp::Alu(VecAluOp::Compare(CompareOp::LeU)),
        v_f6::VMSLE => VectorOp::Alu(VecAluOp::Compare(CompareOp::Le)),
        v_f6::VSADDU => VectorOp::Alu(VecAluOp::Int(IntOp::SAddU)),
        v_f6::VSADD => VectorOp::Alu(VecAluOp::Int(IntOp::SAdd)),
        v_f6::VSSUBU => VectorOp::Alu(VecAluOp::Int(IntOp::SSubU)),
        v_f6::VSSUB => VectorOp::Alu(VecAluOp::Int(IntOp::SSub)),
        v_f6::VSLL => VectorOp::Alu(VecAluOp::Int(IntOp::Sll)),
        v_f6::VSMUL => VectorOp::Alu(VecAluOp::Int(IntOp::Smul)),
        v_f6::VSRL => VectorOp::Alu(VecAluOp::Int(IntOp::Srl)),
        v_f6::VSRA => VectorOp::Alu(VecAluOp::Int(IntOp::Sra)),
        v_f6::VSSRL => VectorOp::Alu(VecAluOp::Int(IntOp::SSrl)),
        v_f6::VSSRA => VectorOp::Alu(VecAluOp::Int(IntOp::SSra)),
        v_f6::VNSRL => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Srl)),
        v_f6::VNSRA => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Sra)),
        v_f6::VNCLIPU => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::ClipU)),
        v_f6::VNCLIP => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Clip)),
        // VWREDSUMU/VWREDSUM in OPIVV vs vwaddu/vwadd in OPMVV share funct6.
        v_f6::VWREDSUMU => VectorOp::Reduce(ReduceOp::WidenInt(WidenIntReduceOp::SumU)),
        v_f6::VWREDSUM => VectorOp::Reduce(ReduceOp::WidenInt(WidenIntReduceOp::Sum)),
        v_f6::VANDN => VectorOp::Alu(VecAluOp::Int(IntOp::AndN)),
        v_f6::VROL => VectorOp::Alu(VecAluOp::Int(IntOp::Rol)),
        v_f6::VROR => VectorOp::Alu(VecAluOp::Int(IntOp::Ror)),
        v_f6::VWSLL => VectorOp::Alu(VecAluOp::Widen(WidenOp::Sll)),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPIVX funct3 (integer vector-scalar) operations.
const fn decode_opivx(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VSUB => VectorOp::Alu(VecAluOp::Int(IntOp::Sub)),
        v_f6::VRSUB => VectorOp::Alu(VecAluOp::Int(IntOp::Rsub)),
        v_f6::VMINU => VectorOp::Alu(VecAluOp::Int(IntOp::MinU)),
        v_f6::VMIN => VectorOp::Alu(VecAluOp::Int(IntOp::Min)),
        v_f6::VMAXU => VectorOp::Alu(VecAluOp::Int(IntOp::MaxU)),
        v_f6::VMAX => VectorOp::Alu(VecAluOp::Int(IntOp::Max)),
        v_f6::VAND => VectorOp::Alu(VecAluOp::Int(IntOp::And)),
        v_f6::VOR => VectorOp::Alu(VecAluOp::Int(IntOp::Or)),
        v_f6::VXOR => VectorOp::Alu(VecAluOp::Int(IntOp::Xor)),
        v_f6::VADD => VectorOp::Alu(VecAluOp::Int(IntOp::Add)),
        v_f6::VRGATHER => VectorOp::Permute(PermuteOp::Rgather),
        v_f6::VSLIDEUP => VectorOp::Permute(PermuteOp::SlideUp(SlideOffset::Rs1)),
        v_f6::VSLIDEDOWN => VectorOp::Permute(PermuteOp::SlideDown(SlideOffset::Rs1)),
        v_f6::VADC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Adc)),
        v_f6::VMADC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Madc)),
        v_f6::VSBC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Sbc)),
        v_f6::VMSBC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Msbc)),
        v_f6::VMERGE_VMV => VectorOp::Alu(VecAluOp::Merge),
        v_f6::VMSEQ => VectorOp::Alu(VecAluOp::Compare(CompareOp::Eq)),
        v_f6::VMSNE => VectorOp::Alu(VecAluOp::Compare(CompareOp::Ne)),
        v_f6::VMSLTU => VectorOp::Alu(VecAluOp::Compare(CompareOp::LtU)),
        v_f6::VMSLT => VectorOp::Alu(VecAluOp::Compare(CompareOp::Lt)),
        v_f6::VMSLEU => VectorOp::Alu(VecAluOp::Compare(CompareOp::LeU)),
        v_f6::VMSLE => VectorOp::Alu(VecAluOp::Compare(CompareOp::Le)),
        v_f6::VMSGTU => VectorOp::Alu(VecAluOp::Compare(CompareOp::GtU)),
        v_f6::VMSGT => VectorOp::Alu(VecAluOp::Compare(CompareOp::Gt)),
        v_f6::VSADDU => VectorOp::Alu(VecAluOp::Int(IntOp::SAddU)),
        v_f6::VSADD => VectorOp::Alu(VecAluOp::Int(IntOp::SAdd)),
        v_f6::VSSUBU => VectorOp::Alu(VecAluOp::Int(IntOp::SSubU)),
        v_f6::VSSUB => VectorOp::Alu(VecAluOp::Int(IntOp::SSub)),
        v_f6::VSLL => VectorOp::Alu(VecAluOp::Int(IntOp::Sll)),
        v_f6::VSMUL => VectorOp::Alu(VecAluOp::Int(IntOp::Smul)),
        v_f6::VSRL => VectorOp::Alu(VecAluOp::Int(IntOp::Srl)),
        v_f6::VSRA => VectorOp::Alu(VecAluOp::Int(IntOp::Sra)),
        v_f6::VSSRL => VectorOp::Alu(VecAluOp::Int(IntOp::SSrl)),
        v_f6::VSSRA => VectorOp::Alu(VecAluOp::Int(IntOp::SSra)),
        v_f6::VNSRL => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Srl)),
        v_f6::VNSRA => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Sra)),
        v_f6::VNCLIPU => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::ClipU)),
        v_f6::VNCLIP => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Clip)),
        v_f6::VANDN => VectorOp::Alu(VecAluOp::Int(IntOp::AndN)),
        v_f6::VROL => VectorOp::Alu(VecAluOp::Int(IntOp::Rol)),
        v_f6::VROR => VectorOp::Alu(VecAluOp::Int(IntOp::Ror)),
        v_f6::VWSLL => VectorOp::Alu(VecAluOp::Widen(WidenOp::Sll)),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPIVI funct3 (integer vector-immediate) operations.
/// The immediate an OPIVI `op` reads: shifts, gathers and the Zvbb
/// shifts take `uimm5` (RVV 1.0 §11.7), `vror` a 6-bit `zimm` whose top
/// bit is instruction bit 26, and the rest a sign-extended `simm5`.
const fn vi_immediate(op: VectorOp, inst: u32) -> i8 {
    match op {
        VectorOp::Alu(VecAluOp::Int(IntOp::Ror)) => {
            ((((inst >> VROR_ZIMM_HIGH_BIT) & 1) << 5) as u64 | v_enc::uimm5(inst)) as i8
        }
        VectorOp::Alu(
            VecAluOp::Int(IntOp::Sll | IntOp::Srl | IntOp::Sra | IntOp::SSrl | IntOp::SSra)
            | VecAluOp::Narrow(_)
            | VecAluOp::Widen(WidenOp::Sll),
        )
        | VectorOp::Permute(PermuteOp::Rgather) => v_enc::uimm5(inst) as i8,
        _ => v_enc::simm5(inst) as i8,
    }
}

const fn decode_opivi(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VRSUB => VectorOp::Alu(VecAluOp::Int(IntOp::Rsub)),
        v_f6::VAND => VectorOp::Alu(VecAluOp::Int(IntOp::And)),
        v_f6::VOR => VectorOp::Alu(VecAluOp::Int(IntOp::Or)),
        v_f6::VXOR => VectorOp::Alu(VecAluOp::Int(IntOp::Xor)),
        v_f6::VADD => VectorOp::Alu(VecAluOp::Int(IntOp::Add)),
        v_f6::VRGATHER => VectorOp::Permute(PermuteOp::Rgather),
        v_f6::VSLIDEUP => {
            VectorOp::Permute(PermuteOp::SlideUp(SlideOffset::Imm(v_enc::uimm5(inst) as u8)))
        }
        v_f6::VSLIDEDOWN => {
            VectorOp::Permute(PermuteOp::SlideDown(SlideOffset::Imm(v_enc::uimm5(inst) as u8)))
        }
        // VSMUL funct6 in OPIVI is whole-register move; simm5 encodes nregs.
        v_f6::VSMUL => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                0b00000 => VectorOp::Permute(PermuteOp::WholeMove(WholeRegisters::One)),
                0b00001 => VectorOp::Permute(PermuteOp::WholeMove(WholeRegisters::Two)),
                0b00011 => VectorOp::Permute(PermuteOp::WholeMove(WholeRegisters::Four)),
                0b00111 => VectorOp::Permute(PermuteOp::WholeMove(WholeRegisters::Eight)),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        v_f6::VADC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Adc)),
        v_f6::VMADC => VectorOp::Alu(VecAluOp::Carry(CarryOp::Madc)),
        v_f6::VMERGE_VMV => VectorOp::Alu(VecAluOp::Merge),
        v_f6::VMSEQ => VectorOp::Alu(VecAluOp::Compare(CompareOp::Eq)),
        v_f6::VMSNE => VectorOp::Alu(VecAluOp::Compare(CompareOp::Ne)),
        v_f6::VMSLEU => VectorOp::Alu(VecAluOp::Compare(CompareOp::LeU)),
        v_f6::VMSLE => VectorOp::Alu(VecAluOp::Compare(CompareOp::Le)),
        v_f6::VMSGTU => VectorOp::Alu(VecAluOp::Compare(CompareOp::GtU)),
        v_f6::VMSGT => VectorOp::Alu(VecAluOp::Compare(CompareOp::Gt)),
        v_f6::VSADDU => VectorOp::Alu(VecAluOp::Int(IntOp::SAddU)),
        v_f6::VSADD => VectorOp::Alu(VecAluOp::Int(IntOp::SAdd)),
        v_f6::VSLL => VectorOp::Alu(VecAluOp::Int(IntOp::Sll)),
        v_f6::VSRL => VectorOp::Alu(VecAluOp::Int(IntOp::Srl)),
        v_f6::VSRA => VectorOp::Alu(VecAluOp::Int(IntOp::Sra)),
        v_f6::VSSRL => VectorOp::Alu(VecAluOp::Int(IntOp::SSrl)),
        v_f6::VSSRA => VectorOp::Alu(VecAluOp::Int(IntOp::SSra)),
        v_f6::VNSRL => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Srl)),
        v_f6::VNSRA => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Sra)),
        v_f6::VNCLIPU => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::ClipU)),
        v_f6::VNCLIP => VectorOp::Alu(VecAluOp::Narrow(NarrowOp::Clip)),
        v_f6::VROR | v_f6::VROR_VI_ROTATE_HIGH => VectorOp::Alu(VecAluOp::Int(IntOp::Ror)),
        v_f6::VWSLL => VectorOp::Alu(VecAluOp::Widen(WidenOp::Sll)),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPMVV funct3 (mask/reduction vector-vector) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opmvv(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        v_f6::VREDSUM => (VectorOp::Reduce(ReduceOp::Int(IntReduceOp::Sum)), true),
        v_f6::VREDAND => (VectorOp::Reduce(ReduceOp::Int(IntReduceOp::And)), true),
        v_f6::VREDOR => (VectorOp::Reduce(ReduceOp::Int(IntReduceOp::Or)), true),
        v_f6::VREDXOR => (VectorOp::Reduce(ReduceOp::Int(IntReduceOp::Xor)), true),
        v_f6::VREDMINU => (VectorOp::Reduce(ReduceOp::Int(IntReduceOp::MinU)), true),
        v_f6::VREDMIN => (VectorOp::Reduce(ReduceOp::Int(IntReduceOp::Min)), true),
        v_f6::VREDMAXU => (VectorOp::Reduce(ReduceOp::Int(IntReduceOp::MaxU)), true),
        v_f6::VREDMAX => (VectorOp::Reduce(ReduceOp::Int(IntReduceOp::Max)), true),
        v_f6::VAADDU => (VectorOp::Alu(VecAluOp::Int(IntOp::AAddU)), true),
        v_f6::VAADD => (VectorOp::Alu(VecAluOp::Int(IntOp::AAdd)), true),
        v_f6::VASUBU => (VectorOp::Alu(VecAluOp::Int(IntOp::ASubU)), true),
        v_f6::VASUB => (VectorOp::Alu(VecAluOp::Int(IntOp::ASub)), true),
        v_f6::VWXUNARY0 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                v_f6::VWXUNARY0_VMV_X_S => (VectorOp::Permute(PermuteOp::MvXS), false),
                v_f6::VWXUNARY0_VCPOP_M => (VectorOp::Mask(MaskOp::CPop), false),
                v_f6::VWXUNARY0_VFIRST_M => (VectorOp::Mask(MaskOp::First), false),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        v_f6::VXUNARY0 => match decode_vxunary0(inst) {
            Ok(op) => (VectorOp::Alu(op), true),
            Err(e) => return Err(e),
        },
        v_f6::VMUNARY0 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                v_f6::VMUNARY0_VMSBF_M => {
                    (VectorOp::Mask(MaskOp::Set(MaskSetOp::BeforeFirst)), true)
                }
                v_f6::VMUNARY0_VMSOF_M => (VectorOp::Mask(MaskOp::Set(MaskSetOp::OnlyFirst)), true),
                v_f6::VMUNARY0_VMSIF_M => {
                    (VectorOp::Mask(MaskOp::Set(MaskSetOp::IncludingFirst)), true)
                }
                v_f6::VMUNARY0_VIOTA_M => (VectorOp::Mask(MaskOp::Iota), true),
                v_f6::VMUNARY0_VID_V => (VectorOp::Mask(MaskOp::Id), true),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        // VMERGE_VMV in OPMVV decodes to vcompress.
        v_f6::VMERGE_VMV => (VectorOp::Permute(PermuteOp::Compress), true),
        v_f6::VMANDN => (VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::AndNot)), true),
        v_f6::VMAND => (VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::And)), true),
        v_f6::VMOR => (VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::Or)), true),
        v_f6::VMXOR => (VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::Xor)), true),
        v_f6::VMORN => (VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::OrNot)), true),
        v_f6::VMNAND => (VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::Nand)), true),
        v_f6::VMNOR => (VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::Nor)), true),
        v_f6::VMXNOR => (VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::Xnor)), true),
        v_f6::VMUL => (VectorOp::Alu(VecAluOp::Int(IntOp::Mul)), true),
        v_f6::VMULH => (VectorOp::Alu(VecAluOp::Int(IntOp::Mulh)), true),
        v_f6::VMULHU => (VectorOp::Alu(VecAluOp::Int(IntOp::Mulhu)), true),
        v_f6::VMULHSU => (VectorOp::Alu(VecAluOp::Int(IntOp::Mulhsu)), true),
        v_f6::VMACC => (VectorOp::Alu(VecAluOp::Macc(MaccOp::Macc)), true),
        v_f6::VNMSAC => (VectorOp::Alu(VecAluOp::Macc(MaccOp::NMSac)), true),
        v_f6::VMADD => (VectorOp::Alu(VecAluOp::Macc(MaccOp::Madd)), true),
        v_f6::VNMSUB => (VectorOp::Alu(VecAluOp::Macc(MaccOp::NMSub)), true),
        v_f6::VDIVU => (VectorOp::Alu(VecAluOp::Int(IntOp::DivU)), true),
        v_f6::VDIV => (VectorOp::Alu(VecAluOp::Int(IntOp::Div)), true),
        v_f6::VREMU => (VectorOp::Alu(VecAluOp::Int(IntOp::RemU)), true),
        v_f6::VREM => (VectorOp::Alu(VecAluOp::Int(IntOp::Rem)), true),
        // VWREDSUMU/VWREDSUM share funct6 with these but live in OPIVV (decode_opivv).
        v_f6::VWADDU => (VectorOp::Alu(VecAluOp::Widen(WidenOp::AddU)), true),
        v_f6::VWADD => (VectorOp::Alu(VecAluOp::Widen(WidenOp::Add)), true),
        v_f6::VWSUBU => (VectorOp::Alu(VecAluOp::Widen(WidenOp::SubU)), true),
        v_f6::VWSUB => (VectorOp::Alu(VecAluOp::Widen(WidenOp::Sub)), true),
        v_f6::VWADDU_W => (VectorOp::Alu(VecAluOp::Widen(WidenOp::AddUW)), true),
        v_f6::VWADD_W => (VectorOp::Alu(VecAluOp::Widen(WidenOp::AddW)), true),
        v_f6::VWSUBU_W => (VectorOp::Alu(VecAluOp::Widen(WidenOp::SubUW)), true),
        v_f6::VWSUB_W => (VectorOp::Alu(VecAluOp::Widen(WidenOp::SubW)), true),
        v_f6::VWMULU => (VectorOp::Alu(VecAluOp::Widen(WidenOp::MulU)), true),
        v_f6::VWMULSU => (VectorOp::Alu(VecAluOp::Widen(WidenOp::MulSU)), true),
        v_f6::VWMUL => (VectorOp::Alu(VecAluOp::Widen(WidenOp::Mul)), true),
        v_f6::VWMACCU => (VectorOp::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccU)), true),
        v_f6::VWMACC => (VectorOp::Alu(VecAluOp::WidenMacc(WidenMaccOp::Macc)), true),
        v_f6::VWMACCSU => (VectorOp::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccSU)), true),
        v_f6::VWMACCUS => (VectorOp::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccUS)), true),
        v_f6::VCLMUL => (VectorOp::Alu(VecAluOp::Int(IntOp::ClMul)), true),
        v_f6::VCLMULH => (VectorOp::Alu(VecAluOp::Int(IntOp::ClMulH)), true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPMVX funct3 (mask/move vector-scalar) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opmvx(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        v_f6::VRXUNARY0 => (VectorOp::Permute(PermuteOp::MvSX), true),
        v_f6::VSLIDEUP => (VectorOp::Permute(PermuteOp::Slide1Up), true),
        v_f6::VSLIDEDOWN => (VectorOp::Permute(PermuteOp::Slide1Down), true),
        v_f6::VMUL => (VectorOp::Alu(VecAluOp::Int(IntOp::Mul)), true),
        v_f6::VMULH => (VectorOp::Alu(VecAluOp::Int(IntOp::Mulh)), true),
        v_f6::VMULHU => (VectorOp::Alu(VecAluOp::Int(IntOp::Mulhu)), true),
        v_f6::VMULHSU => (VectorOp::Alu(VecAluOp::Int(IntOp::Mulhsu)), true),
        v_f6::VMACC => (VectorOp::Alu(VecAluOp::Macc(MaccOp::Macc)), true),
        v_f6::VNMSAC => (VectorOp::Alu(VecAluOp::Macc(MaccOp::NMSac)), true),
        v_f6::VMADD => (VectorOp::Alu(VecAluOp::Macc(MaccOp::Madd)), true),
        v_f6::VNMSUB => (VectorOp::Alu(VecAluOp::Macc(MaccOp::NMSub)), true),
        v_f6::VDIVU => (VectorOp::Alu(VecAluOp::Int(IntOp::DivU)), true),
        v_f6::VDIV => (VectorOp::Alu(VecAluOp::Int(IntOp::Div)), true),
        v_f6::VREMU => (VectorOp::Alu(VecAluOp::Int(IntOp::RemU)), true),
        v_f6::VREM => (VectorOp::Alu(VecAluOp::Int(IntOp::Rem)), true),
        v_f6::VWADDU => (VectorOp::Alu(VecAluOp::Widen(WidenOp::AddU)), true),
        v_f6::VWADD => (VectorOp::Alu(VecAluOp::Widen(WidenOp::Add)), true),
        v_f6::VWSUBU => (VectorOp::Alu(VecAluOp::Widen(WidenOp::SubU)), true),
        v_f6::VWSUB => (VectorOp::Alu(VecAluOp::Widen(WidenOp::Sub)), true),
        v_f6::VWADDU_W => (VectorOp::Alu(VecAluOp::Widen(WidenOp::AddUW)), true),
        v_f6::VWADD_W => (VectorOp::Alu(VecAluOp::Widen(WidenOp::AddW)), true),
        v_f6::VWSUBU_W => (VectorOp::Alu(VecAluOp::Widen(WidenOp::SubUW)), true),
        v_f6::VWSUB_W => (VectorOp::Alu(VecAluOp::Widen(WidenOp::SubW)), true),
        v_f6::VWMULU => (VectorOp::Alu(VecAluOp::Widen(WidenOp::MulU)), true),
        v_f6::VWMULSU => (VectorOp::Alu(VecAluOp::Widen(WidenOp::MulSU)), true),
        v_f6::VWMUL => (VectorOp::Alu(VecAluOp::Widen(WidenOp::Mul)), true),
        v_f6::VWMACCU => (VectorOp::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccU)), true),
        v_f6::VWMACC => (VectorOp::Alu(VecAluOp::WidenMacc(WidenMaccOp::Macc)), true),
        v_f6::VWMACCSU => (VectorOp::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccSU)), true),
        v_f6::VWMACCUS => (VectorOp::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccUS)), true),
        v_f6::VAADDU => (VectorOp::Alu(VecAluOp::Int(IntOp::AAddU)), true),
        v_f6::VAADD => (VectorOp::Alu(VecAluOp::Int(IntOp::AAdd)), true),
        v_f6::VASUBU => (VectorOp::Alu(VecAluOp::Int(IntOp::ASubU)), true),
        v_f6::VASUB => (VectorOp::Alu(VecAluOp::Int(IntOp::ASub)), true),
        v_f6::VCLMUL => (VectorOp::Alu(VecAluOp::Int(IntOp::ClMul)), true),
        v_f6::VCLMULH => (VectorOp::Alu(VecAluOp::Int(IntOp::ClMulH)), true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode `VXUNARY0`, the Zvbb unary ops and the integer extensions, from
/// the vs1 field.
const fn decode_vxunary0(inst: u32) -> Result<VecAluOp, Trap> {
    let vs1_field = v_enc::vs1(inst);
    Ok(match vs1_field {
        v_f6::VXUNARY0_VBREV => VecAluOp::Int(IntOp::Brev),
        v_f6::VXUNARY0_VBREV8 => VecAluOp::Int(IntOp::Brev8),
        v_f6::VXUNARY0_VREV8 => VecAluOp::Int(IntOp::Rev8),
        v_f6::VXUNARY0_VCLZ => VecAluOp::Int(IntOp::Clz),
        v_f6::VXUNARY0_VCTZ => VecAluOp::Int(IntOp::Ctz),
        v_f6::VXUNARY0_VCPOP => VecAluOp::Int(IntOp::CpopV),
        _ => match extension(vs1_field) {
            Some(op) => VecAluOp::Extend(op),
            None => return Err(Trap::IllegalInstruction(inst)),
        },
    })
}

/// The integer extension a `VXUNARY0` vs1 field selects, if any.
const fn extension(vs1_field: u8) -> Option<ExtendOp> {
    use ExtendFactor::{Vf2, Vf4, Vf8};
    use Signedness::{Signed, Unsigned};
    let (signedness, factor) = match vs1_field {
        v_f6::VXUNARY0_VZEXT_VF8 => (Unsigned, Vf8),
        v_f6::VXUNARY0_VSEXT_VF8 => (Signed, Vf8),
        v_f6::VXUNARY0_VZEXT_VF4 => (Unsigned, Vf4),
        v_f6::VXUNARY0_VSEXT_VF4 => (Signed, Vf4),
        v_f6::VXUNARY0_VZEXT_VF2 => (Unsigned, Vf2),
        v_f6::VXUNARY0_VSEXT_VF2 => (Signed, Vf2),
        _ => return None,
    };
    Some(ExtendOp { signedness, factor })
}

/// Decode `VFUNARY0` sub-operations (conversion ops) from the vs1 field.
const fn decode_vfunary0(inst: u32) -> Result<VecFpOp, Trap> {
    use IntConvert::{FromInt, ToInt};
    use IntRounding::{Dynamic, TowardZero};
    use Signedness::{Signed, Unsigned};
    let vs1_field = v_enc::vs1(inst);
    Ok(match vs1_field {
        0b00000 => VecFpOp::Unary(FpUnaryOp::Convert(ToInt(Unsigned, Dynamic))),
        0b00001 => VecFpOp::Unary(FpUnaryOp::Convert(ToInt(Signed, Dynamic))),
        0b00010 => VecFpOp::Unary(FpUnaryOp::Convert(FromInt(Unsigned))),
        0b00011 => VecFpOp::Unary(FpUnaryOp::Convert(FromInt(Signed))),
        0b00110 => VecFpOp::Unary(FpUnaryOp::Convert(ToInt(Unsigned, TowardZero))),
        0b00111 => VecFpOp::Unary(FpUnaryOp::Convert(ToInt(Signed, TowardZero))),
        0b01000 => VecFpOp::WidenConvert(FpWidenConvertOp::Int(ToInt(Unsigned, Dynamic))),
        0b01001 => VecFpOp::WidenConvert(FpWidenConvertOp::Int(ToInt(Signed, Dynamic))),
        0b01010 => VecFpOp::WidenConvert(FpWidenConvertOp::Int(FromInt(Unsigned))),
        0b01011 => VecFpOp::WidenConvert(FpWidenConvertOp::Int(FromInt(Signed))),
        0b01100 => VecFpOp::WidenConvert(FpWidenConvertOp::Float),
        0b01110 => VecFpOp::WidenConvert(FpWidenConvertOp::Int(ToInt(Unsigned, TowardZero))),
        0b01111 => VecFpOp::WidenConvert(FpWidenConvertOp::Int(ToInt(Signed, TowardZero))),
        0b10000 => VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(ToInt(Unsigned, Dynamic))),
        0b10001 => VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(ToInt(Signed, Dynamic))),
        0b10010 => VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(FromInt(Unsigned))),
        0b10011 => VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(FromInt(Signed))),
        0b10100 => VecFpOp::NarrowConvert(FpNarrowConvertOp::Float),
        0b10101 => VecFpOp::NarrowConvert(FpNarrowConvertOp::FloatRoundToOdd),
        0b10110 => VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(ToInt(Unsigned, TowardZero))),
        0b10111 => VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(ToInt(Signed, TowardZero))),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode `VFUNARY1` sub-operations (`vfsqrt`, `vfrsqrt7`, `vfrec7`, `vfclass`)
/// from the vs1 field.
const fn decode_vfunary1(inst: u32) -> Result<FpUnaryOp, Trap> {
    let vs1_field = v_enc::vs1(inst);
    Ok(match vs1_field {
        0b00000 => FpUnaryOp::Sqrt,
        0b00100 => FpUnaryOp::Rsqrt7,
        0b00101 => FpUnaryOp::Rec7,
        0b10000 => FpUnaryOp::Class,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPFVV funct3 (FP vector-vector) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opfvv(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        v_f6::VFADD => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Add)), true),
        v_f6::VFSUB => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Sub)), true),
        v_f6::VFMIN => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Min)), true),
        v_f6::VFMAX => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Max)), true),
        v_f6::VFSGNJ => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Sgnj)), true),
        v_f6::VFSGNJN => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Sgnjn)), true),
        v_f6::VFSGNJX => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Sgnjx)), true),
        v_f6::VMFEQ => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Eq)), true),
        v_f6::VMFLE => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Le)), true),
        v_f6::VMFLT => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Lt)), true),
        v_f6::VMFNE => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Ne)), true),
        v_f6::VFDIV => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Div)), true),
        v_f6::VFMUL => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Mul)), true),
        v_f6::VFMACC => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::Macc, FmaAddend::Vd)), true),
        v_f6::VFNMACC => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::NMacc, FmaAddend::Vd)), true),
        v_f6::VFMSAC => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::MSac, FmaAddend::Vd)), true),
        v_f6::VFNMSAC => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::NMSac, FmaAddend::Vd)), true),
        v_f6::VFMADD => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::Macc, FmaAddend::Vs2)), true),
        v_f6::VFNMADD => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::NMacc, FmaAddend::Vs2)), true),
        v_f6::VFMSUB => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::MSac, FmaAddend::Vs2)), true),
        v_f6::VFNMSUB => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::NMSac, FmaAddend::Vs2)), true),
        v_f6::VFWADD => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::Add)), true),
        v_f6::VFWSUB => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::Sub)), true),
        v_f6::VFWADD_W => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::AddW)), true),
        v_f6::VFWSUB_W => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::SubW)), true),
        v_f6::VFWMUL => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::Mul)), true),
        v_f6::VFWMACC => (VectorOp::Fp(VecFpOp::WidenFma(FmaSigns::Macc)), true),
        v_f6::VFWNMACC => (VectorOp::Fp(VecFpOp::WidenFma(FmaSigns::NMacc)), true),
        v_f6::VFWMSAC => (VectorOp::Fp(VecFpOp::WidenFma(FmaSigns::MSac)), true),
        v_f6::VFWNMSAC => (VectorOp::Fp(VecFpOp::WidenFma(FmaSigns::NMSac)), true),
        v_f6::VWFUNARY0 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                v_f6::VWFUNARY0_VFMV_F_S => (VectorOp::Fp(VecFpOp::MoveToScalar), false),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        v_f6::VFUNARY0 => match decode_vfunary0(inst) {
            Ok(op) => (VectorOp::Fp(op), true),
            Err(e) => return Err(e),
        },
        v_f6::VFUNARY1 => match decode_vfunary1(inst) {
            Ok(op) => (VectorOp::Fp(VecFpOp::Unary(op)), true),
            Err(e) => return Err(e),
        },
        v_f6::VFREDUSUM => (VectorOp::Reduce(ReduceOp::Fp(FpReduceOp::USum)), true),
        v_f6::VFREDOSUM => (VectorOp::Reduce(ReduceOp::Fp(FpReduceOp::OSum)), true),
        v_f6::VFREDMIN => (VectorOp::Reduce(ReduceOp::Fp(FpReduceOp::Min)), true),
        v_f6::VFREDMAX => (VectorOp::Reduce(ReduceOp::Fp(FpReduceOp::Max)), true),
        v_f6::VFWREDUSUM => (VectorOp::Reduce(ReduceOp::FpWiden(FpWidenReduceOp::USum)), true),
        v_f6::VFWREDOSUM => (VectorOp::Reduce(ReduceOp::FpWiden(FpWidenReduceOp::OSum)), true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPFVF funct3 (FP vector-scalar) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opfvf(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        v_f6::VFADD => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Add)), true),
        v_f6::VFSUB => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Sub)), true),
        v_f6::VFRSUB => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::RSub)), true),
        v_f6::VFMIN => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Min)), true),
        v_f6::VFMAX => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Max)), true),
        v_f6::VFSGNJ => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Sgnj)), true),
        v_f6::VFSGNJN => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Sgnjn)), true),
        v_f6::VFSGNJX => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Sgnjx)), true),
        v_f6::VMFEQ => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Eq)), true),
        v_f6::VMFLE => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Le)), true),
        v_f6::VMFLT => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Lt)), true),
        v_f6::VMFNE => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Ne)), true),
        v_f6::VMFGT => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Gt)), true),
        v_f6::VMFGE => (VectorOp::Fp(VecFpOp::Compare(FpCompareOp::Ge)), true),
        v_f6::VFDIV => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Div)), true),
        v_f6::VFRDIV => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::RDiv)), true),
        v_f6::VFMUL => (VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Mul)), true),
        v_f6::VFMACC => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::Macc, FmaAddend::Vd)), true),
        v_f6::VFNMACC => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::NMacc, FmaAddend::Vd)), true),
        v_f6::VFMSAC => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::MSac, FmaAddend::Vd)), true),
        v_f6::VFNMSAC => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::NMSac, FmaAddend::Vd)), true),
        v_f6::VFMADD => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::Macc, FmaAddend::Vs2)), true),
        v_f6::VFNMADD => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::NMacc, FmaAddend::Vs2)), true),
        v_f6::VFMSUB => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::MSac, FmaAddend::Vs2)), true),
        v_f6::VFNMSUB => (VectorOp::Fp(VecFpOp::Fma(FmaSigns::NMSac, FmaAddend::Vs2)), true),
        v_f6::VFWADD => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::Add)), true),
        v_f6::VFWSUB => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::Sub)), true),
        v_f6::VFWADD_W => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::AddW)), true),
        v_f6::VFWSUB_W => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::SubW)), true),
        v_f6::VFWMUL => (VectorOp::Fp(VecFpOp::Widen(FpWidenOp::Mul)), true),
        v_f6::VFWMACC => (VectorOp::Fp(VecFpOp::WidenFma(FmaSigns::Macc)), true),
        v_f6::VFWNMACC => (VectorOp::Fp(VecFpOp::WidenFma(FmaSigns::NMacc)), true),
        v_f6::VFWMSAC => (VectorOp::Fp(VecFpOp::WidenFma(FmaSigns::MSac)), true),
        v_f6::VFWNMSAC => (VectorOp::Fp(VecFpOp::WidenFma(FmaSigns::NMSac)), true),
        v_f6::VFSLIDE1UP => (VectorOp::Fp(VecFpOp::Slide1Up), true),
        v_f6::VFSLIDE1DOWN => (VectorOp::Fp(VecFpOp::Slide1Down), true),
        v_f6::VRFUNARY0 => {
            let vs2_field = v_enc::vs2(inst);
            match vs2_field {
                v_f6::VRFUNARY0_VFMV_S_F => (VectorOp::Fp(VecFpOp::MoveFromScalar), true),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        // VMERGE_VMV: vm=0 vfmerge.vfm, vm=1 vfmv.v.f.
        v_f6::VMERGE_VMV => (VectorOp::Fp(VecFpOp::Merge), true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Map funct3 width encoding to `Sew` for vector loads/stores.
const fn funct3_to_eew(funct3: u32) -> Sew {
    match funct3 {
        VEC_WIDTH_8 => Sew::E8,
        VEC_WIDTH_16 => Sew::E16,
        VEC_WIDTH_32 => Sew::E32,
        _ => Sew::E64,
    }
}

/// Decode a vector load instruction (`OP_LOAD_FP` with vector funct3).
pub(super) const fn decode_vec_load(
    inst: u32,
    funct3: u32,
    c: &mut ControlSignals,
) -> Result<(), Trap> {
    let eew = funct3_to_eew(funct3);
    let mop = v_enc::mop(inst);
    let vm = v_enc::vm(inst);
    let nf = v_enc::nf(inst);
    let mew = v_enc::mew(inst);

    // mew must be 0 for RVV 1.0
    if mew {
        return Err(Trap::IllegalInstruction(inst));
    }

    let vd_raw = v_enc::vd(inst);

    // Segment/whole-register ops access vd..vd+nf; reject if that exceeds v31.
    if vd_raw + nf >= 32 {
        return Err(Trap::IllegalInstruction(inst));
    }

    c.vec_eew = eew;
    c.vec_nf = nf;
    c.vm = vm;
    c.vd = VRegIdx::new(vd_raw);
    c.vs2 = VRegIdx::new(v_enc::vs2(inst));
    c.vec_reg_write = true;

    match mop {
        MOP_UNIT => {
            let lumop = v_enc::lumop(inst);
            c.vec_op = Some(match lumop {
                LUMOP_UNIT => VectorOp::Load(VecLoadOp::Unit),
                LUMOP_WHOLE_REG => VectorOp::Load(VecLoadOp::WholeRegister),
                LUMOP_MASK => VectorOp::Load(VecLoadOp::Mask),
                LUMOP_FAULT_FIRST => VectorOp::Load(VecLoadOp::FaultOnlyFirst),
                _ => return Err(Trap::IllegalInstruction(inst)),
            });
        }
        MOP_INDEXED_UNORD => {
            c.vec_op = Some(VectorOp::Load(VecLoadOp::IndexedUnordered));
        }
        MOP_STRIDED => {
            c.vec_op = Some(VectorOp::Load(VecLoadOp::Strided));
        }
        MOP_INDEXED_ORD => {
            c.vec_op = Some(VectorOp::Load(VecLoadOp::IndexedOrdered));
        }
        _ => return Err(Trap::IllegalInstruction(inst)),
    }

    Ok(())
}

/// Decode a vector store instruction (`OP_STORE_FP` with vector funct3).
pub(super) const fn decode_vec_store(
    inst: u32,
    funct3: u32,
    c: &mut ControlSignals,
) -> Result<(), Trap> {
    let eew = funct3_to_eew(funct3);
    let mop = v_enc::mop(inst);
    let vm = v_enc::vm(inst);
    let nf = v_enc::nf(inst);
    let mew = v_enc::mew(inst);

    // mew must be 0 for RVV 1.0
    if mew {
        return Err(Trap::IllegalInstruction(inst));
    }

    let vd_raw = v_enc::vd(inst);

    // Segment/whole-register ops access vs3..vs3+nf; reject if that exceeds v31.
    if vd_raw + nf >= 32 {
        return Err(Trap::IllegalInstruction(inst));
    }

    c.vec_eew = eew;
    c.vec_nf = nf;
    c.vm = vm;
    c.vd = VRegIdx::new(vd_raw); // vd is vs3 (store data) for stores
    c.vs2 = VRegIdx::new(v_enc::vs2(inst));
    c.vec_reg_write = false; // stores don't write vector registers

    match mop {
        MOP_UNIT => {
            let sumop = v_enc::sumop(inst);
            c.vec_op = Some(match sumop {
                SUMOP_UNIT => VectorOp::Store(VecStoreOp::Unit),
                SUMOP_WHOLE_REG => VectorOp::Store(VecStoreOp::WholeRegister),
                SUMOP_MASK => VectorOp::Store(VecStoreOp::Mask),
                _ => return Err(Trap::IllegalInstruction(inst)),
            });
        }
        MOP_INDEXED_UNORD => {
            c.vec_op = Some(VectorOp::Store(VecStoreOp::IndexedUnordered));
        }
        MOP_STRIDED => {
            c.vec_op = Some(VectorOp::Store(VecStoreOp::Strided));
        }
        MOP_INDEXED_ORD => {
            c.vec_op = Some(VectorOp::Store(VecStoreOp::IndexedOrdered));
        }
        _ => return Err(Trap::IllegalInstruction(inst)),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::common::VirtAddr;
    use crate::exec::decode::decode_inst;
    use crate::isa::encoding::rvv::{funct3 as v_funct3, funct6 as v_f6, opcodes as v_opcodes};
    use crate::isa::op::{
        ExtendFactor, ExtendOp, FpNarrowConvertOp, FpUnaryOp, FpWidenConvertOp, IntConvert, IntOp,
        IntRounding, Signedness, VecAluOp, VecFpOp, VectorOp,
    };

    /// The op the unmasked unary instruction with `funct6`, `funct3` and
    /// vs1 field `selector` decodes to, or `None` if it is illegal. Its
    /// registers are aligned for any group size LMUL=1 widens to.
    fn decode_unary(funct6: u32, funct3: u32, selector: u32) -> Option<VectorOp> {
        let (vs2, vd) = (8, 16);
        let inst = funct6 << 26
            | 1 << 25
            | vs2 << 20
            | selector << 15
            | funct3 << 12
            | vd << 7
            | v_opcodes::OP_V;
        let decoded = decode_inst(inst, VirtAddr::new(0), 0);
        if decoded.trap.is_some() {
            return None;
        }
        decoded.ctrl.vec_op
    }

    /// Asserts that every vs1 field of the unary group decodes as `listed`
    /// says, and that the fields it leaves out are illegal.
    fn assert_unary_group(funct6: u32, funct3: u32, listed: &[(u32, VectorOp)]) {
        for selector in 0..32 {
            let expected = listed.iter().find(|(s, _)| *s == selector).map(|&(_, op)| op);
            assert_eq!(decode_unary(funct6, funct3, selector), expected, "vs1 = {selector:#07b}");
        }
    }

    #[test]
    fn vfunary0_decodes_each_conversion_its_vs1_field_selects() {
        use IntRounding::{Dynamic, TowardZero};
        use Signedness::{Signed, Unsigned};
        let single = |convert| VectorOp::Fp(VecFpOp::Unary(FpUnaryOp::Convert(convert)));
        let widen = |convert| VectorOp::Fp(VecFpOp::WidenConvert(convert));
        let narrow = |convert| VectorOp::Fp(VecFpOp::NarrowConvert(convert));
        let to_int = IntConvert::ToInt;
        let from_int = IntConvert::FromInt;
        assert_unary_group(
            v_f6::VFUNARY0,
            v_funct3::OPFVV,
            &[
                (0b00000, single(to_int(Unsigned, Dynamic))),
                (0b00001, single(to_int(Signed, Dynamic))),
                (0b00010, single(from_int(Unsigned))),
                (0b00011, single(from_int(Signed))),
                (0b00110, single(to_int(Unsigned, TowardZero))),
                (0b00111, single(to_int(Signed, TowardZero))),
                (0b01000, widen(FpWidenConvertOp::Int(to_int(Unsigned, Dynamic)))),
                (0b01001, widen(FpWidenConvertOp::Int(to_int(Signed, Dynamic)))),
                (0b01010, widen(FpWidenConvertOp::Int(from_int(Unsigned)))),
                (0b01011, widen(FpWidenConvertOp::Int(from_int(Signed)))),
                (0b01100, widen(FpWidenConvertOp::Float)),
                (0b01110, widen(FpWidenConvertOp::Int(to_int(Unsigned, TowardZero)))),
                (0b01111, widen(FpWidenConvertOp::Int(to_int(Signed, TowardZero)))),
                (0b10000, narrow(FpNarrowConvertOp::Int(to_int(Unsigned, Dynamic)))),
                (0b10001, narrow(FpNarrowConvertOp::Int(to_int(Signed, Dynamic)))),
                (0b10010, narrow(FpNarrowConvertOp::Int(from_int(Unsigned)))),
                (0b10011, narrow(FpNarrowConvertOp::Int(from_int(Signed)))),
                (0b10100, narrow(FpNarrowConvertOp::Float)),
                (0b10101, narrow(FpNarrowConvertOp::FloatRoundToOdd)),
                (0b10110, narrow(FpNarrowConvertOp::Int(to_int(Unsigned, TowardZero)))),
                (0b10111, narrow(FpNarrowConvertOp::Int(to_int(Signed, TowardZero)))),
            ],
        );
    }

    #[test]
    fn vxunary0_decodes_each_extension_and_bit_op_its_vs1_field_selects() {
        use ExtendFactor::{Vf2, Vf4, Vf8};
        use Signedness::{Signed, Unsigned};
        let extend =
            |signedness, factor| VectorOp::Alu(VecAluOp::Extend(ExtendOp { signedness, factor }));
        let int = |op| VectorOp::Alu(VecAluOp::Int(op));
        assert_unary_group(
            v_f6::VXUNARY0,
            v_funct3::OPMVV,
            &[
                (0b00010, extend(Unsigned, Vf8)),
                (0b00011, extend(Signed, Vf8)),
                (0b00100, extend(Unsigned, Vf4)),
                (0b00101, extend(Signed, Vf4)),
                (0b00110, extend(Unsigned, Vf2)),
                (0b00111, extend(Signed, Vf2)),
                (0b01000, int(IntOp::Brev8)),
                (0b01001, int(IntOp::Rev8)),
                (0b01010, int(IntOp::Brev)),
                (0b01100, int(IntOp::Clz)),
                (0b01101, int(IntOp::Ctz)),
                (0b01110, int(IntOp::CpopV)),
            ],
        );
    }
}
