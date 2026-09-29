//! Vector instruction decoding.

use super::{VEC_WIDTH_8, VEC_WIDTH_16, VEC_WIDTH_32};
use crate::exec::signals::{ControlSignals, OpBSrc};
use crate::isa::encoding::rvv::{
    encoding as v_enc, funct3 as v_funct3, funct6 as v_f6, opcodes as v_opcodes,
};
use crate::isa::instruction::Decoded;
use crate::isa::op::{VecSrcEncoding, VectorOp};
use crate::isa::privileged::Trap;
use crate::isa::rvv::{Sew, VRegIdx};

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
                    c.vec_op = VectorOp::Vsetvli;
                    c.reg_write = true;
                } else if bit30 == 1 {
                    // vsetivli: zimm[9:0] from bits 29:20, uimm[4:0] from bits 19:15
                    c.vec_op = VectorOp::Vsetivli;
                    c.reg_write = true;
                } else {
                    // vsetvl: vtype from rs2
                    c.vec_op = VectorOp::Vsetvl;
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
                        c.vec_op = decode_opivv(f6, inst)?;
                    }
                    v_funct3::OPIVX => {
                        c.vec_reg_write = true;
                        c.vec_src_encoding = VecSrcEncoding::VX;
                        c.vec_op = decode_opivx(f6, inst)?;
                    }
                    v_funct3::OPIVI => {
                        c.vec_reg_write = true;
                        c.vec_src_encoding = VecSrcEncoding::VI;
                        c.vec_op = decode_opivi(f6, inst)?;
                        // Whole-register moves: validate vd+nregs and vs2+nregs <= 32
                        let nregs: u8 = match c.vec_op {
                            VectorOp::VMv1r => 1,
                            VectorOp::VMv2r => 2,
                            VectorOp::VMv4r => 4,
                            VectorOp::VMv8r => 8,
                            _ => 0,
                        };
                        if nregs > 0 {
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
                        c.vec_op = op;
                        c.vec_reg_write = writes_vec;
                        // Some OPMVV ops write scalar rd instead of vec
                        if matches!(op, VectorOp::VMvXS | VectorOp::VCPopM | VectorOp::VFirstM) {
                            c.reg_write = true;
                        }
                    }
                    v_funct3::OPMVX => {
                        c.vec_src_encoding = VecSrcEncoding::VX;
                        let (op, writes_vec) = decode_opmvx(f6, inst)?;
                        c.vec_op = op;
                        c.vec_reg_write = writes_vec;
                    }
                    v_funct3::OPFVV => {
                        c.vec_src_encoding = VecSrcEncoding::VV;
                        let (op, writes_vec) = decode_opfvv(f6, inst)?;
                        c.vec_op = op;
                        c.vec_reg_write = writes_vec;
                        // Scalar-to-FP move (vfmv.f.s) writes FP rd, not vec
                        if matches!(op, VectorOp::VFMvFS) {
                            c.fp_reg_write = true;
                        }
                    }
                    v_funct3::OPFVF => {
                        c.vec_src_encoding = VecSrcEncoding::VF;
                        c.rs1_fp = true;
                        let (op, writes_vec) = decode_opfvf(f6, inst)?;
                        c.vec_op = op;
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
            c.vec_op = decode_opcrypto(f6, inst)?;
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
        v_f6::VSM3_ME => VectorOp::VSm3Me,
        v_f6::VSM4_K => VectorOp::VSm4K,
        v_f6::VAES_KF1 => VectorOp::VAesKf1,
        v_f6::VCRYPTO_VV => match vs1 {
            v_f6::VAES_VS1_DM => VectorOp::VAesDm,
            v_f6::VAES_VS1_DF => VectorOp::VAesDf,
            v_f6::VAES_VS1_EM => VectorOp::VAesEm,
            v_f6::VAES_VS1_EF => VectorOp::VAesEf,
            v_f6::VSM4_VS1_R => VectorOp::VSm4R,
            v_f6::VGMUL_VS1 => VectorOp::VGmul,
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        v_f6::VCRYPTO_VS => match vs1 {
            v_f6::VAES_VS1_DM => VectorOp::VAesDm,
            v_f6::VAES_VS1_DF => VectorOp::VAesDf,
            v_f6::VAES_VS1_EM => VectorOp::VAesEm,
            v_f6::VAES_VS1_EF => VectorOp::VAesEf,
            v_f6::VAES_VS1_Z => VectorOp::VAesZ,
            v_f6::VSM4_VS1_R => VectorOp::VSm4R,
            _ => return Err(Trap::IllegalInstruction(inst)),
        },
        v_f6::VAES_KF2 => VectorOp::VAesKf2,
        v_f6::VSM3_C => VectorOp::VSm3C,
        v_f6::VGHSH => VectorOp::VGhsh,
        v_f6::VSHA2_MS => VectorOp::VSha2Ms,
        v_f6::VSHA2_CH => VectorOp::VSha2Ch,
        v_f6::VSHA2_CL => VectorOp::VSha2Cl,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPIVV funct3 (integer vector-vector) operations.
const fn decode_opivv(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VSUB => VectorOp::VSub,
        v_f6::VMINU => VectorOp::VMinU,
        v_f6::VMIN => VectorOp::VMin,
        v_f6::VMAXU => VectorOp::VMaxU,
        v_f6::VMAX => VectorOp::VMax,
        v_f6::VAND => VectorOp::VAnd,
        v_f6::VOR => VectorOp::VOr,
        v_f6::VXOR => VectorOp::VXor,
        v_f6::VADD => VectorOp::VAdd,
        v_f6::VRGATHER => VectorOp::VRgather,
        v_f6::VRGATHEREI16 => VectorOp::VRgatherEi16,
        v_f6::VADC => VectorOp::VAdc,
        v_f6::VMADC => VectorOp::VMadc,
        v_f6::VSBC => VectorOp::VSbc,
        v_f6::VMSBC => VectorOp::VMsbc,
        v_f6::VMERGE_VMV => VectorOp::VMerge,
        v_f6::VMSEQ => VectorOp::VMSeq,
        v_f6::VMSNE => VectorOp::VMSne,
        v_f6::VMSLTU => VectorOp::VMSltu,
        v_f6::VMSLT => VectorOp::VMSlt,
        v_f6::VMSLEU => VectorOp::VMSleu,
        v_f6::VMSLE => VectorOp::VMSle,
        v_f6::VSADDU => VectorOp::VSAddU,
        v_f6::VSADD => VectorOp::VSAdd,
        v_f6::VSSUBU => VectorOp::VSSubU,
        v_f6::VSSUB => VectorOp::VSSub,
        v_f6::VSLL => VectorOp::VSll,
        v_f6::VSMUL => VectorOp::VSmul,
        v_f6::VSRL => VectorOp::VSrl,
        v_f6::VSRA => VectorOp::VSra,
        v_f6::VSSRL => VectorOp::VSSrl,
        v_f6::VSSRA => VectorOp::VSSra,
        v_f6::VNSRL => VectorOp::VNSrl,
        v_f6::VNSRA => VectorOp::VNSra,
        v_f6::VNCLIPU => VectorOp::VNClipU,
        v_f6::VNCLIP => VectorOp::VNClip,
        // VWREDSUMU/VWREDSUM in OPIVV vs vwaddu/vwadd in OPMVV share funct6.
        v_f6::VWREDSUMU => VectorOp::VWRedSumU,
        v_f6::VWREDSUM => VectorOp::VWRedSum,
        v_f6::VANDN => VectorOp::VAndN,
        v_f6::VROL => VectorOp::VRol,
        v_f6::VROR => VectorOp::VRor,
        v_f6::VWSLL => VectorOp::VWsll,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPIVX funct3 (integer vector-scalar) operations.
const fn decode_opivx(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VSUB => VectorOp::VSub,
        v_f6::VRSUB => VectorOp::VRsub,
        v_f6::VMINU => VectorOp::VMinU,
        v_f6::VMIN => VectorOp::VMin,
        v_f6::VMAXU => VectorOp::VMaxU,
        v_f6::VMAX => VectorOp::VMax,
        v_f6::VAND => VectorOp::VAnd,
        v_f6::VOR => VectorOp::VOr,
        v_f6::VXOR => VectorOp::VXor,
        v_f6::VADD => VectorOp::VAdd,
        v_f6::VRGATHER => VectorOp::VRgather,
        v_f6::VSLIDEUP => VectorOp::VSlideUp,
        v_f6::VSLIDEDOWN => VectorOp::VSlideDown,
        v_f6::VADC => VectorOp::VAdc,
        v_f6::VMADC => VectorOp::VMadc,
        v_f6::VSBC => VectorOp::VSbc,
        v_f6::VMSBC => VectorOp::VMsbc,
        v_f6::VMERGE_VMV => VectorOp::VMerge,
        v_f6::VMSEQ => VectorOp::VMSeq,
        v_f6::VMSNE => VectorOp::VMSne,
        v_f6::VMSLTU => VectorOp::VMSltu,
        v_f6::VMSLT => VectorOp::VMSlt,
        v_f6::VMSLEU => VectorOp::VMSleu,
        v_f6::VMSLE => VectorOp::VMSle,
        v_f6::VMSGTU => VectorOp::VMSgtu,
        v_f6::VMSGT => VectorOp::VMSgt,
        v_f6::VSADDU => VectorOp::VSAddU,
        v_f6::VSADD => VectorOp::VSAdd,
        v_f6::VSSUBU => VectorOp::VSSubU,
        v_f6::VSSUB => VectorOp::VSSub,
        v_f6::VSLL => VectorOp::VSll,
        v_f6::VSMUL => VectorOp::VSmul,
        v_f6::VSRL => VectorOp::VSrl,
        v_f6::VSRA => VectorOp::VSra,
        v_f6::VSSRL => VectorOp::VSSrl,
        v_f6::VSSRA => VectorOp::VSSra,
        v_f6::VNSRL => VectorOp::VNSrl,
        v_f6::VNSRA => VectorOp::VNSra,
        v_f6::VNCLIPU => VectorOp::VNClipU,
        v_f6::VNCLIP => VectorOp::VNClip,
        v_f6::VANDN => VectorOp::VAndN,
        v_f6::VROL => VectorOp::VRol,
        v_f6::VROR => VectorOp::VRor,
        v_f6::VWSLL => VectorOp::VWsll,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPIVI funct3 (integer vector-immediate) operations.
const fn decode_opivi(f6: u32, inst: u32) -> Result<VectorOp, Trap> {
    Ok(match f6 {
        v_f6::VRSUB => VectorOp::VRsub,
        v_f6::VAND => VectorOp::VAnd,
        v_f6::VOR => VectorOp::VOr,
        v_f6::VXOR => VectorOp::VXor,
        v_f6::VADD => VectorOp::VAdd,
        v_f6::VRGATHER => VectorOp::VRgather,
        v_f6::VSLIDEUP => VectorOp::VSlideUp,
        v_f6::VSLIDEDOWN => VectorOp::VSlideDown,
        // VSMUL funct6 in OPIVI is whole-register move; simm5 encodes nregs.
        v_f6::VSMUL => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                0b00000 => VectorOp::VMv1r,
                0b00001 => VectorOp::VMv2r,
                0b00011 => VectorOp::VMv4r,
                0b00111 => VectorOp::VMv8r,
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        v_f6::VADC => VectorOp::VAdc,
        v_f6::VMADC => VectorOp::VMadc,
        v_f6::VMERGE_VMV => VectorOp::VMerge,
        v_f6::VMSEQ => VectorOp::VMSeq,
        v_f6::VMSNE => VectorOp::VMSne,
        v_f6::VMSLEU => VectorOp::VMSleu,
        v_f6::VMSLE => VectorOp::VMSle,
        v_f6::VMSGTU => VectorOp::VMSgtu,
        v_f6::VMSGT => VectorOp::VMSgt,
        v_f6::VSADDU => VectorOp::VSAddU,
        v_f6::VSADD => VectorOp::VSAdd,
        v_f6::VSLL => VectorOp::VSll,
        v_f6::VSRL => VectorOp::VSrl,
        v_f6::VSRA => VectorOp::VSra,
        v_f6::VSSRL => VectorOp::VSSrl,
        v_f6::VSSRA => VectorOp::VSSra,
        v_f6::VNSRL => VectorOp::VNSrl,
        v_f6::VNSRA => VectorOp::VNSra,
        v_f6::VNCLIPU => VectorOp::VNClipU,
        v_f6::VNCLIP => VectorOp::VNClip,
        v_f6::VROR => VectorOp::VRor,
        v_f6::VWSLL => VectorOp::VWsll,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPMVV funct3 (mask/reduction vector-vector) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opmvv(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        v_f6::VREDSUM => (VectorOp::VRedSum, true),
        v_f6::VREDAND => (VectorOp::VRedAnd, true),
        v_f6::VREDOR => (VectorOp::VRedOr, true),
        v_f6::VREDXOR => (VectorOp::VRedXor, true),
        v_f6::VREDMINU => (VectorOp::VRedMinU, true),
        v_f6::VREDMIN => (VectorOp::VRedMin, true),
        v_f6::VREDMAXU => (VectorOp::VRedMaxU, true),
        v_f6::VREDMAX => (VectorOp::VRedMax, true),
        v_f6::VAADDU => (VectorOp::VAAddU, true),
        v_f6::VAADD => (VectorOp::VAAdd, true),
        v_f6::VASUBU => (VectorOp::VASubU, true),
        v_f6::VASUB => (VectorOp::VASub, true),
        v_f6::VWXUNARY0 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                v_f6::VWXUNARY0_VMV_X_S => (VectorOp::VMvXS, false),
                v_f6::VWXUNARY0_VCPOP_M => (VectorOp::VCPopM, false),
                v_f6::VWXUNARY0_VFIRST_M => (VectorOp::VFirstM, false),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        v_f6::VXUNARY0 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                v_f6::VXUNARY0_VZEXT_VF8 => (VectorOp::VZextVf8, true),
                v_f6::VXUNARY0_VSEXT_VF8 => (VectorOp::VSextVf8, true),
                v_f6::VXUNARY0_VZEXT_VF4 => (VectorOp::VZextVf4, true),
                v_f6::VXUNARY0_VSEXT_VF4 => (VectorOp::VSextVf4, true),
                v_f6::VXUNARY0_VZEXT_VF2 => (VectorOp::VZextVf2, true),
                v_f6::VXUNARY0_VSEXT_VF2 => (VectorOp::VSextVf2, true),
                v_f6::VXUNARY0_VBREV => (VectorOp::VBrev, true),
                v_f6::VXUNARY0_VBREV8 => (VectorOp::VBrev8, true),
                v_f6::VXUNARY0_VREV8 => (VectorOp::VRev8, true),
                v_f6::VXUNARY0_VCLZ => (VectorOp::VClz, true),
                v_f6::VXUNARY0_VCTZ => (VectorOp::VCtz, true),
                v_f6::VXUNARY0_VCPOP => (VectorOp::VCpopV, true),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        v_f6::VMUNARY0 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                v_f6::VMUNARY0_VMSBF_M => (VectorOp::VMSbfM, true),
                v_f6::VMUNARY0_VMSOF_M => (VectorOp::VMSofM, true),
                v_f6::VMUNARY0_VMSIF_M => (VectorOp::VMSifM, true),
                v_f6::VMUNARY0_VIOTA_M => (VectorOp::VIotaM, true),
                v_f6::VMUNARY0_VID_V => (VectorOp::VIdV, true),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        // VMERGE_VMV in OPMVV decodes to vcompress.
        v_f6::VMERGE_VMV => (VectorOp::VCompress, true),
        v_f6::VMANDN => (VectorOp::VMAndnMM, true),
        v_f6::VMAND => (VectorOp::VMAndMM, true),
        v_f6::VMOR => (VectorOp::VMOrMM, true),
        v_f6::VMXOR => (VectorOp::VMXorMM, true),
        v_f6::VMORN => (VectorOp::VMOrnMM, true),
        v_f6::VMNAND => (VectorOp::VMNandMM, true),
        v_f6::VMNOR => (VectorOp::VMNorMM, true),
        v_f6::VMXNOR => (VectorOp::VMXnorMM, true),
        v_f6::VMUL => (VectorOp::VMul, true),
        v_f6::VMULH => (VectorOp::VMulh, true),
        v_f6::VMULHU => (VectorOp::VMulhu, true),
        v_f6::VMULHSU => (VectorOp::VMulhsu, true),
        v_f6::VMACC => (VectorOp::VMacc, true),
        v_f6::VNMSAC => (VectorOp::VNMSac, true),
        v_f6::VMADD => (VectorOp::VMadd, true),
        v_f6::VNMSUB => (VectorOp::VNMSub, true),
        v_f6::VDIVU => (VectorOp::VDivU, true),
        v_f6::VDIV => (VectorOp::VDiv, true),
        v_f6::VREMU => (VectorOp::VRemU, true),
        v_f6::VREM => (VectorOp::VRem, true),
        // VWREDSUMU/VWREDSUM share funct6 with these but live in OPIVV (decode_opivv).
        v_f6::VWADDU => (VectorOp::VWAddU, true),
        v_f6::VWADD => (VectorOp::VWAdd, true),
        v_f6::VWSUBU => (VectorOp::VWSubU, true),
        v_f6::VWSUB => (VectorOp::VWSub, true),
        v_f6::VWADDU_W => (VectorOp::VWAddUW, true),
        v_f6::VWADD_W => (VectorOp::VWAddW, true),
        v_f6::VWSUBU_W => (VectorOp::VWSubUW, true),
        v_f6::VWSUB_W => (VectorOp::VWSubW, true),
        v_f6::VWMULU => (VectorOp::VWMulU, true),
        v_f6::VWMULSU => (VectorOp::VWMulSU, true),
        v_f6::VWMUL => (VectorOp::VWMul, true),
        v_f6::VWMACCU => (VectorOp::VWMaccU, true),
        v_f6::VWMACC => (VectorOp::VWMacc, true),
        v_f6::VWMACCSU => (VectorOp::VWMaccSU, true),
        v_f6::VWMACCUS => (VectorOp::VWMaccUS, true),
        v_f6::VCLMUL => (VectorOp::VClMul, true),
        v_f6::VCLMULH => (VectorOp::VClMulH, true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPMVX funct3 (mask/move vector-scalar) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opmvx(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        0b010000 => (VectorOp::VMvSX, true),
        v_f6::VSLIDEUP => (VectorOp::VSlide1Up, true),
        v_f6::VSLIDEDOWN => (VectorOp::VSlide1Down, true),
        v_f6::VMUL => (VectorOp::VMul, true),
        v_f6::VMULH => (VectorOp::VMulh, true),
        v_f6::VMULHU => (VectorOp::VMulhu, true),
        v_f6::VMULHSU => (VectorOp::VMulhsu, true),
        v_f6::VMACC => (VectorOp::VMacc, true),
        v_f6::VNMSAC => (VectorOp::VNMSac, true),
        v_f6::VMADD => (VectorOp::VMadd, true),
        v_f6::VNMSUB => (VectorOp::VNMSub, true),
        v_f6::VDIVU => (VectorOp::VDivU, true),
        v_f6::VDIV => (VectorOp::VDiv, true),
        v_f6::VREMU => (VectorOp::VRemU, true),
        v_f6::VREM => (VectorOp::VRem, true),
        v_f6::VWADDU => (VectorOp::VWAddU, true),
        v_f6::VWADD => (VectorOp::VWAdd, true),
        v_f6::VWSUBU => (VectorOp::VWSubU, true),
        v_f6::VWSUB => (VectorOp::VWSub, true),
        v_f6::VWADDU_W => (VectorOp::VWAddUW, true),
        v_f6::VWADD_W => (VectorOp::VWAddW, true),
        v_f6::VWSUBU_W => (VectorOp::VWSubUW, true),
        v_f6::VWSUB_W => (VectorOp::VWSubW, true),
        v_f6::VWMULU => (VectorOp::VWMulU, true),
        v_f6::VWMULSU => (VectorOp::VWMulSU, true),
        v_f6::VWMUL => (VectorOp::VWMul, true),
        v_f6::VWMACCU => (VectorOp::VWMaccU, true),
        v_f6::VWMACC => (VectorOp::VWMacc, true),
        v_f6::VWMACCSU => (VectorOp::VWMaccSU, true),
        v_f6::VWMACCUS => (VectorOp::VWMaccUS, true),
        v_f6::VAADDU => (VectorOp::VAAddU, true),
        v_f6::VAADD => (VectorOp::VAAdd, true),
        v_f6::VASUBU => (VectorOp::VASubU, true),
        v_f6::VASUB => (VectorOp::VASub, true),
        v_f6::VCLMUL => (VectorOp::VClMul, true),
        v_f6::VCLMULH => (VectorOp::VClMulH, true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode `VFUNARY0` sub-operations (conversion ops) from the vs1 field.
const fn decode_vfunary0(inst: u32) -> Result<VectorOp, Trap> {
    let vs1_field = v_enc::vs1(inst);
    Ok(match vs1_field {
        0b00000 => VectorOp::VFCvtXuF,
        0b00001 => VectorOp::VFCvtXF,
        0b00010 => VectorOp::VFCvtFXu,
        0b00011 => VectorOp::VFCvtFX,
        0b00110 => VectorOp::VFCvtRtzXuF,
        0b00111 => VectorOp::VFCvtRtzXF,
        0b01000 => VectorOp::VFWCvtXuF,
        0b01001 => VectorOp::VFWCvtXF,
        0b01010 => VectorOp::VFWCvtFXu,
        0b01011 => VectorOp::VFWCvtFX,
        0b01100 => VectorOp::VFWCvtFF,
        0b01110 => VectorOp::VFWCvtRtzXuF,
        0b01111 => VectorOp::VFWCvtRtzXF,
        0b10000 => VectorOp::VFNCvtXuF,
        0b10001 => VectorOp::VFNCvtXF,
        0b10010 => VectorOp::VFNCvtFXu,
        0b10011 => VectorOp::VFNCvtFX,
        0b10100 => VectorOp::VFNCvtFF,
        0b10101 => VectorOp::VFNCvtRodFF,
        0b10110 => VectorOp::VFNCvtRtzXuF,
        0b10111 => VectorOp::VFNCvtRtzXF,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode `VFUNARY1` sub-operations (`vfsqrt`, `vfrsqrt7`, `vfrec7`, `vfclass`)
/// from the vs1 field.
const fn decode_vfunary1(inst: u32) -> Result<VectorOp, Trap> {
    let vs1_field = v_enc::vs1(inst);
    Ok(match vs1_field {
        0b00000 => VectorOp::VFSqrt,
        0b00100 => VectorOp::VFRsqrt7,
        0b00101 => VectorOp::VFRec7,
        0b10000 => VectorOp::VFClass,
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPFVV funct3 (FP vector-vector) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opfvv(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        v_f6::VFADD => (VectorOp::VFAdd, true),
        v_f6::VFSUB => (VectorOp::VFSub, true),
        v_f6::VFMIN => (VectorOp::VFMin, true),
        v_f6::VFMAX => (VectorOp::VFMax, true),
        v_f6::VFSGNJ => (VectorOp::VFSgnj, true),
        v_f6::VFSGNJN => (VectorOp::VFSgnjn, true),
        v_f6::VFSGNJX => (VectorOp::VFSgnjx, true),
        v_f6::VMFEQ => (VectorOp::VMFEq, true),
        v_f6::VMFLE => (VectorOp::VMFLe, true),
        v_f6::VMFLT => (VectorOp::VMFLt, true),
        v_f6::VMFNE => (VectorOp::VMFNe, true),
        v_f6::VFDIV => (VectorOp::VFDiv, true),
        v_f6::VFMUL => (VectorOp::VFMul, true),
        v_f6::VFMACC => (VectorOp::VFMacc, true),
        v_f6::VFNMACC => (VectorOp::VFNMacc, true),
        v_f6::VFMSAC => (VectorOp::VFMSac, true),
        v_f6::VFNMSAC => (VectorOp::VFNMSac, true),
        v_f6::VFMADD => (VectorOp::VFMAdd, true),
        v_f6::VFNMADD => (VectorOp::VFNMAdd, true),
        v_f6::VFMSUB => (VectorOp::VFMSub, true),
        v_f6::VFNMSUB => (VectorOp::VFNMSub, true),
        v_f6::VFWADD => (VectorOp::VFWAdd, true),
        v_f6::VFWSUB => (VectorOp::VFWSub, true),
        v_f6::VFWADD_W => (VectorOp::VFWAddW, true),
        v_f6::VFWSUB_W => (VectorOp::VFWSubW, true),
        v_f6::VFWMUL => (VectorOp::VFWMul, true),
        v_f6::VFWMACC => (VectorOp::VFWMacc, true),
        v_f6::VFWNMACC => (VectorOp::VFWNMacc, true),
        v_f6::VFWMSAC => (VectorOp::VFWMSac, true),
        v_f6::VFWNMSAC => (VectorOp::VFWNMSac, true),
        v_f6::VWFUNARY0 => {
            let vs1_field = v_enc::vs1(inst);
            match vs1_field {
                v_f6::VWFUNARY0_VFMV_F_S => (VectorOp::VFMvFS, false),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        v_f6::VFUNARY0 => match decode_vfunary0(inst) {
            Ok(op) => (op, true),
            Err(e) => return Err(e),
        },
        v_f6::VFUNARY1 => match decode_vfunary1(inst) {
            Ok(op) => (op, true),
            Err(e) => return Err(e),
        },
        v_f6::VFREDUSUM => (VectorOp::VFRedUSum, true),
        v_f6::VFREDOSUM => (VectorOp::VFRedOSum, true),
        v_f6::VFREDMIN => (VectorOp::VFRedMin, true),
        v_f6::VFREDMAX => (VectorOp::VFRedMax, true),
        v_f6::VFWREDUSUM => (VectorOp::VFWRedUSum, true),
        v_f6::VFWREDOSUM => (VectorOp::VFWRedOSum, true),
        _ => return Err(Trap::IllegalInstruction(inst)),
    })
}

/// Decode OPFVF funct3 (FP vector-scalar) operations.
/// Returns `(VectorOp, writes_vec_reg)`.
const fn decode_opfvf(f6: u32, inst: u32) -> Result<(VectorOp, bool), Trap> {
    Ok(match f6 {
        v_f6::VFADD => (VectorOp::VFAdd, true),
        v_f6::VFSUB => (VectorOp::VFSub, true),
        v_f6::VFRSUB => (VectorOp::VFRSub, true),
        v_f6::VFMIN => (VectorOp::VFMin, true),
        v_f6::VFMAX => (VectorOp::VFMax, true),
        v_f6::VFSGNJ => (VectorOp::VFSgnj, true),
        v_f6::VFSGNJN => (VectorOp::VFSgnjn, true),
        v_f6::VFSGNJX => (VectorOp::VFSgnjx, true),
        v_f6::VMFEQ => (VectorOp::VMFEq, true),
        v_f6::VMFLE => (VectorOp::VMFLe, true),
        v_f6::VMFLT => (VectorOp::VMFLt, true),
        v_f6::VMFNE => (VectorOp::VMFNe, true),
        v_f6::VMFGT => (VectorOp::VMFGt, true),
        v_f6::VMFGE => (VectorOp::VMFGe, true),
        v_f6::VFDIV => (VectorOp::VFDiv, true),
        v_f6::VFRDIV => (VectorOp::VFRDiv, true),
        v_f6::VFMUL => (VectorOp::VFMul, true),
        v_f6::VFMACC => (VectorOp::VFMacc, true),
        v_f6::VFNMACC => (VectorOp::VFNMacc, true),
        v_f6::VFMSAC => (VectorOp::VFMSac, true),
        v_f6::VFNMSAC => (VectorOp::VFNMSac, true),
        v_f6::VFMADD => (VectorOp::VFMAdd, true),
        v_f6::VFNMADD => (VectorOp::VFNMAdd, true),
        v_f6::VFMSUB => (VectorOp::VFMSub, true),
        v_f6::VFNMSUB => (VectorOp::VFNMSub, true),
        v_f6::VFWADD => (VectorOp::VFWAdd, true),
        v_f6::VFWSUB => (VectorOp::VFWSub, true),
        v_f6::VFWADD_W => (VectorOp::VFWAddW, true),
        v_f6::VFWSUB_W => (VectorOp::VFWSubW, true),
        v_f6::VFWMUL => (VectorOp::VFWMul, true),
        v_f6::VFWMACC => (VectorOp::VFWMacc, true),
        v_f6::VFWNMACC => (VectorOp::VFWNMacc, true),
        v_f6::VFWMSAC => (VectorOp::VFWMSac, true),
        v_f6::VFWNMSAC => (VectorOp::VFWNMSac, true),
        v_f6::VFSLIDE1UP => (VectorOp::VFSlide1Up, true),
        v_f6::VFSLIDE1DOWN => (VectorOp::VFSlide1Down, true),
        v_f6::VRFUNARY0 => {
            let vs2_field = v_enc::vs2(inst);
            match vs2_field {
                v_f6::VRFUNARY0_VFMV_S_F => (VectorOp::VFMvSF, true),
                _ => return Err(Trap::IllegalInstruction(inst)),
            }
        }
        // VMERGE_VMV: vm=0 vfmerge.vfm, vm=1 vfmv.v.f.
        v_f6::VMERGE_VMV => (VectorOp::VFMerge, true),
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
            c.vec_op = match lumop {
                LUMOP_UNIT => VectorOp::VLoadUnit,
                LUMOP_WHOLE_REG => VectorOp::VLoadWholeReg,
                LUMOP_MASK => VectorOp::VLoadMask,
                LUMOP_FAULT_FIRST => VectorOp::VLoadFF,
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
        }
        MOP_INDEXED_UNORD => {
            c.vec_op = VectorOp::VLoadIndexUnord;
        }
        MOP_STRIDED => {
            c.vec_op = VectorOp::VLoadStride;
        }
        MOP_INDEXED_ORD => {
            c.vec_op = VectorOp::VLoadIndexOrd;
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
            c.vec_op = match sumop {
                SUMOP_UNIT => VectorOp::VStoreUnit,
                SUMOP_WHOLE_REG => VectorOp::VStoreWholeReg,
                SUMOP_MASK => VectorOp::VStoreMask,
                _ => return Err(Trap::IllegalInstruction(inst)),
            };
        }
        MOP_INDEXED_UNORD => {
            c.vec_op = VectorOp::VStoreIndexUnord;
        }
        MOP_STRIDED => {
            c.vec_op = VectorOp::VStoreStride;
        }
        MOP_INDEXED_ORD => {
            c.vec_op = VectorOp::VStoreIndexOrd;
        }
        _ => return Err(Trap::IllegalInstruction(inst)),
    }

    Ok(())
}
