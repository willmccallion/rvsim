//! The vector operations an instruction decodes to.

/// Vector operation type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VectorOp {
    /// No vector operation.
    #[default]
    None,

    /// `vsetvli` — set vl/vtype from rs1 and immediate.
    Vsetvli,
    /// `vsetivli` — set vl/vtype from uimm and immediate.
    Vsetivli,
    /// `vsetvl` — set vl/vtype from rs1 and rs2.
    Vsetvl,

    /// `vadd` — vector add.
    VAdd,
    /// `vsub` — vector subtract.
    VSub,
    /// `vrsub` — vector reverse subtract (imm/scalar - vs2).
    VRsub,
    /// `vand` — vector bitwise AND.
    VAnd,
    /// `vor` — vector bitwise OR.
    VOr,
    /// `vxor` — vector bitwise XOR.
    VXor,
    /// `vsll` — vector shift left logical.
    VSll,
    /// `vsrl` — vector shift right logical.
    VSrl,
    /// `vsra` — vector shift right arithmetic.
    VSra,
    /// `vminu` — vector unsigned minimum.
    VMinU,
    /// `vmin` — vector signed minimum.
    VMin,
    /// `vmaxu` — vector unsigned maximum.
    VMaxU,
    /// `vmax` — vector signed maximum.
    VMax,

    /// `vmerge` / `vmv` — vector merge or move.
    VMerge,

    /// `vmseq` — set mask if equal.
    VMSeq,
    /// `vmsne` — set mask if not equal.
    VMSne,
    /// `vmsltu` — set mask if less than unsigned.
    VMSltu,
    /// `vmslt` — set mask if less than signed.
    VMSlt,
    /// `vmsleu` — set mask if less than or equal unsigned.
    VMSleu,
    /// `vmsle` — set mask if less than or equal signed.
    VMSle,
    /// `vmsgtu` — set mask if greater than unsigned.
    VMSgtu,
    /// `vmsgt` — set mask if greater than signed.
    VMSgt,

    /// `vadc` — add with carry from v0 mask.
    VAdc,
    /// `vmadc` — mask-producing add with carry.
    VMadc,
    /// `vsbc` — subtract with borrow from v0 mask.
    VSbc,
    /// `vmsbc` — mask-producing subtract with borrow.
    VMsbc,

    /// `vmul` — multiply low bits.
    VMul,
    /// `vmulh` — multiply high bits (signed × signed).
    VMulh,
    /// `vmulhu` — multiply high bits (unsigned × unsigned).
    VMulhu,
    /// `vmulhsu` — multiply high bits (signed × unsigned).
    VMulhsu,
    /// `vmacc` — multiply-accumulate (vd = vs1*vs2 + vd).
    VMacc,
    /// `vnmsac` — negated multiply-subtract accumulate (vd = -(vs1*vs2) + vd).
    VNMSac,
    /// `vmadd` — multiply-add (vd = vs1*vd + vs2).
    VMadd,
    /// `vnmsub` — negated multiply-subtract (vd = -(vs1*vd) + vs2).
    VNMSub,

    /// `vdivu` — unsigned divide.
    VDivU,
    /// `vdiv` — signed divide.
    VDiv,
    /// `vremu` — unsigned remainder.
    VRemU,
    /// `vrem` — signed remainder.
    VRem,

    /// `vwaddu` — widening unsigned add (SEW → 2×SEW).
    VWAddU,
    /// `vwadd` — widening signed add (SEW → 2×SEW).
    VWAdd,
    /// `vwsubu` — widening unsigned subtract (SEW → 2×SEW).
    VWSubU,
    /// `vwsub` — widening signed subtract (SEW → 2×SEW).
    VWSub,
    /// `vwaddu.w` — widening unsigned add wide (2×SEW op SEW → 2×SEW).
    VWAddUW,
    /// `vwadd.w` — widening signed add wide (2×SEW op SEW → 2×SEW).
    VWAddW,
    /// `vwsubu.w` — widening unsigned subtract wide (2×SEW op SEW → 2×SEW).
    VWSubUW,
    /// `vwsub.w` — widening signed subtract wide (2×SEW op SEW → 2×SEW).
    VWSubW,

    /// `vwmulu` — widening unsigned multiply.
    VWMulU,
    /// `vwmul` — widening signed multiply.
    VWMul,
    /// `vwmulsu` — widening signed-unsigned multiply.
    VWMulSU,
    /// `vwmaccu` — widening unsigned multiply-accumulate.
    VWMaccU,
    /// `vwmacc` — widening signed multiply-accumulate.
    VWMacc,
    /// `vwmaccsu` — widening signed-unsigned multiply-accumulate.
    VWMaccSU,
    /// `vwmaccus` — widening unsigned-signed multiply-accumulate.
    VWMaccUS,

    /// `vnsrl` — narrowing shift right logical (2×SEW → SEW).
    VNSrl,
    /// `vnsra` — narrowing shift right arithmetic (2×SEW → SEW).
    VNSra,
    /// `vnclipu` — narrowing clip unsigned with saturation.
    VNClipU,
    /// `vnclip` — narrowing clip signed with saturation.
    VNClip,

    /// `vsaddu` — saturating unsigned add.
    VSAddU,
    /// `vsadd` — saturating signed add.
    VSAdd,
    /// `vssubu` — saturating unsigned subtract.
    VSSubU,
    /// `vssub` — saturating signed subtract.
    VSSub,

    /// `vaaddu` — averaging unsigned add.
    VAAddU,
    /// `vaadd` — averaging signed add.
    VAAdd,
    /// `vasubu` — averaging unsigned subtract.
    VASubU,
    /// `vasub` — averaging signed subtract.
    VASub,

    /// `vsmul` — signed fractional multiply with rounding.
    VSmul,
    /// `vssrl` — scaling shift right logical with rounding.
    VSSrl,
    /// `vssra` — scaling shift right arithmetic with rounding.
    VSSra,

    /// `vzext.vf2` — zero-extend SEW/2 to SEW.
    VZextVf2,
    /// `vzext.vf4` — zero-extend SEW/4 to SEW.
    VZextVf4,
    /// `vzext.vf8` — zero-extend SEW/8 to SEW.
    VZextVf8,
    /// `vsext.vf2` — sign-extend SEW/2 to SEW.
    VSextVf2,
    /// `vsext.vf4` — sign-extend SEW/4 to SEW.
    VSextVf4,
    /// `vsext.vf8` — sign-extend SEW/8 to SEW.
    VSextVf8,

    /// `vandn` — vector bitwise AND-NOT (vd\[i\] = vs2\[i\] & ~op1\[i\]).
    VAndN,
    /// `vbrev.v` — reverse all bits within each element.
    VBrev,
    /// `vbrev8.v` — reverse bits within each byte of each element.
    VBrev8,
    /// `vrev8.v` — reverse bytes within each element.
    VRev8,
    /// `vclz.v` — count leading zeros within each element (at SEW).
    VClz,
    /// `vctz.v` — count trailing zeros within each element (at SEW).
    VCtz,
    /// `vcpop.v` — per-element population count (Zvbb; distinct from `vcpop.m`).
    VCpopV,
    /// `vrol` — vector rotate left (vv/vx).
    VRol,
    /// `vror` — vector rotate right (vv/vx/vi with 6-bit imm).
    VRor,
    /// `vwsll` — widening shift left logical (vd is 2*SEW).
    VWsll,

    /// `vclmul` — carry-less multiply low (GF(2) multiply, low half).
    VClMul,
    /// `vclmulh` — carry-less multiply high (GF(2) multiply, high half).
    VClMulH,

    /// `vaesem.vv`/`vaesem.vs` — AES single-round encryption (middle round).
    VAesEm,
    /// `vaesef.vv`/`vaesef.vs` — AES single-round encryption (final round).
    VAesEf,
    /// `vaesdm.vv`/`vaesdm.vs` — AES single-round decryption (middle round).
    VAesDm,
    /// `vaesdf.vv`/`vaesdf.vs` — AES single-round decryption (final round).
    VAesDf,
    /// `vaesz.vs` — AES round-zero (XOR with key).
    VAesZ,
    /// `vaeskf1.vi` — AES-128 forward key schedule.
    VAesKf1,
    /// `vaeskf2.vi` — AES-256 forward key schedule.
    VAesKf2,

    /// `vsha2ms.vv` — SHA-2 message scheduling.
    VSha2Ms,
    /// `vsha2ch.vv` — SHA-2 compression (high half).
    VSha2Ch,
    /// `vsha2cl.vv` — SHA-2 compression (low half).
    VSha2Cl,

    /// `vsm3me.vv` — SM3 message expansion.
    VSm3Me,
    /// `vsm3c.vi` — SM3 compression.
    VSm3C,

    /// `vsm4r.vv`/`vsm4r.vs` — SM4 round.
    VSm4R,
    /// `vsm4k.vi` — SM4 key expansion.
    VSm4K,

    /// `vghsh.vv` — vector GHASH add-multiply.
    VGhsh,
    /// `vgmul.vv` — vector GHASH multiply.
    VGmul,

    /// Unit-stride vector load (`vle8/16/32/64`).
    VLoadUnit,
    /// Unit-stride vector store (`vse8/16/32/64`).
    VStoreUnit,
    /// Fault-only-first vector load (`vle8ff/16ff/32ff/64ff`).
    VLoadFF,
    /// Mask load (`vlm.v`).
    VLoadMask,
    /// Mask store (`vsm.v`).
    VStoreMask,
    /// Whole-register load (`vl1re8`, `vl2re8`, etc.).
    VLoadWholeReg,
    /// Whole-register store (`vs1r`, `vs2r`, etc.).
    VStoreWholeReg,

    /// Strided vector load (`vlse8/16/32/64`).
    VLoadStride,
    /// Strided vector store (`vsse8/16/32/64`).
    VStoreStride,

    /// Indexed ordered vector load (`vloxei8/16/32/64`).
    VLoadIndexOrd,
    /// Indexed ordered vector store (`vsoxei8/16/32/64`).
    VStoreIndexOrd,
    /// Indexed unordered vector load (`vluxei8/16/32/64`).
    VLoadIndexUnord,
    /// Indexed unordered vector store (`vsuxei8/16/32/64`).
    VStoreIndexUnord,

    /// `vfadd` — vector FP add.
    VFAdd,
    /// `vfsub` — vector FP subtract.
    VFSub,
    /// `vfrsub` — vector FP reverse subtract (scalar - vs2).
    VFRSub,
    /// `vfmul` — vector FP multiply.
    VFMul,
    /// `vfdiv` — vector FP divide.
    VFDiv,
    /// `vfrdiv` — vector FP reverse divide (scalar / vs2).
    VFRDiv,

    /// `vfmin` — vector FP minimum.
    VFMin,
    /// `vfmax` — vector FP maximum.
    VFMax,

    /// `vfsgnj` — vector FP sign injection (copy sign).
    VFSgnj,
    /// `vfsgnjn` — vector FP negated sign injection.
    VFSgnjn,
    /// `vfsgnjx` — vector FP XOR sign injection.
    VFSgnjx,

    /// `vmfeq` — set mask if FP equal.
    VMFEq,
    /// `vmfne` — set mask if FP not equal.
    VMFNe,
    /// `vmflt` — set mask if FP less than.
    VMFLt,
    /// `vmfle` — set mask if FP less than or equal.
    VMFLe,
    /// `vmfgt` — set mask if FP greater than.
    VMFGt,
    /// `vmfge` — set mask if FP greater than or equal.
    VMFGe,

    /// `vfmacc` — FP multiply-accumulate (vd = vs1*vs2 + vd).
    VFMacc,
    /// `vfnmacc` — FP negated multiply-accumulate (vd = -(vs1*vs2) - vd).
    VFNMacc,
    /// `vfmsac` — FP multiply-subtract accumulate (vd = vs1*vs2 - vd).
    VFMSac,
    /// `vfnmsac` — FP negated multiply-subtract accumulate (vd = -(vs1*vs2) + vd).
    VFNMSac,
    /// `vfmadd` — FP multiply-add (vd = vs1*vd + vs2).
    VFMAdd,
    /// `vfnmadd` — FP negated multiply-add (vd = -(vs1*vd) - vs2).
    VFNMAdd,
    /// `vfmsub` — FP multiply-subtract (vd = vs1*vd - vs2).
    VFMSub,
    /// `vfnmsub` — FP negated multiply-subtract (vd = -(vs1*vd) + vs2).
    VFNMSub,

    /// `vfsqrt` — vector FP square root.
    VFSqrt,
    /// `vfrsqrt7` — vector FP reciprocal square root (7-bit accuracy).
    VFRsqrt7,
    /// `vfrec7` — vector FP reciprocal (7-bit accuracy).
    VFRec7,
    /// `vfclass` — vector FP classify.
    VFClass,

    /// `vfcvt.xu.f` — convert FP to unsigned integer.
    VFCvtXuF,
    /// `vfcvt.x.f` — convert FP to signed integer.
    VFCvtXF,
    /// `vfcvt.f.xu` — convert unsigned integer to FP.
    VFCvtFXu,
    /// `vfcvt.f.x` — convert signed integer to FP.
    VFCvtFX,
    /// `vfcvt.rtz.xu.f` — convert FP to unsigned integer (round toward zero).
    VFCvtRtzXuF,
    /// `vfcvt.rtz.x.f` — convert FP to signed integer (round toward zero).
    VFCvtRtzXF,

    /// `vfwadd` — widening FP add (SEW -> 2*SEW).
    VFWAdd,
    /// `vfwsub` — widening FP subtract (SEW -> 2*SEW).
    VFWSub,
    /// `vfwmul` — widening FP multiply (SEW -> 2*SEW).
    VFWMul,
    /// `vfwadd.w` — widening FP add wide (2*SEW op SEW -> 2*SEW).
    VFWAddW,
    /// `vfwsub.w` — widening FP subtract wide (2*SEW op SEW -> 2*SEW).
    VFWSubW,

    /// `vfwmacc` — widening FP multiply-accumulate.
    VFWMacc,
    /// `vfwnmacc` — widening FP negated multiply-accumulate.
    VFWNMacc,
    /// `vfwmsac` — widening FP multiply-subtract accumulate.
    VFWMSac,
    /// `vfwnmsac` — widening FP negated multiply-subtract accumulate.
    VFWNMSac,

    /// `vfwcvt.xu.f` — widening convert FP to unsigned integer.
    VFWCvtXuF,
    /// `vfwcvt.x.f` — widening convert FP to signed integer.
    VFWCvtXF,
    /// `vfwcvt.f.xu` — widening convert unsigned integer to FP.
    VFWCvtFXu,
    /// `vfwcvt.f.x` — widening convert signed integer to FP.
    VFWCvtFX,
    /// `vfwcvt.f.f` — widening convert FP to wider FP.
    VFWCvtFF,
    /// `vfwcvt.rtz.xu.f` — widening convert FP to unsigned integer (round toward zero).
    VFWCvtRtzXuF,
    /// `vfwcvt.rtz.x.f` — widening convert FP to signed integer (round toward zero).
    VFWCvtRtzXF,

    /// `vfncvt.xu.f` — narrowing convert FP to unsigned integer.
    VFNCvtXuF,
    /// `vfncvt.x.f` — narrowing convert FP to signed integer.
    VFNCvtXF,
    /// `vfncvt.f.xu` — narrowing convert unsigned integer to FP.
    VFNCvtFXu,
    /// `vfncvt.f.x` — narrowing convert signed integer to FP.
    VFNCvtFX,
    /// `vfncvt.f.f` — narrowing convert FP to narrower FP.
    VFNCvtFF,
    /// `vfncvt.rod.f.f` — narrowing convert FP to narrower FP (round-odd).
    VFNCvtRodFF,
    /// `vfncvt.rtz.xu.f` — narrowing convert FP to unsigned integer (round toward zero).
    VFNCvtRtzXuF,
    /// `vfncvt.rtz.x.f` — narrowing convert FP to signed integer (round toward zero).
    VFNCvtRtzXF,

    /// `vfmerge` — vector FP merge with mask.
    VFMerge,
    /// `vfmv.s.f` — move FP scalar to vector element 0.
    VFMvSF,
    /// `vfmv.f.s` — move vector element 0 to FP scalar.
    VFMvFS,

    /// `vfslide1up` — slide up by one with FP scalar.
    VFSlide1Up,
    /// `vfslide1down` — slide down by one with FP scalar.
    VFSlide1Down,

    /// `vredsum` — reduction sum.
    VRedSum,
    /// `vredand` — reduction AND.
    VRedAnd,
    /// `vredor` — reduction OR.
    VRedOr,
    /// `vredxor` — reduction XOR.
    VRedXor,
    /// `vredminu` — reduction unsigned minimum.
    VRedMinU,
    /// `vredmin` — reduction signed minimum.
    VRedMin,
    /// `vredmaxu` — reduction unsigned maximum.
    VRedMaxU,
    /// `vredmax` — reduction signed maximum.
    VRedMax,

    /// `vwredsumu` — widening unsigned reduction sum.
    VWRedSumU,
    /// `vwredsum` — widening signed reduction sum.
    VWRedSum,

    /// `vfredosum` — FP ordered reduction sum.
    VFRedOSum,
    /// `vfredusum` — FP unordered reduction sum.
    VFRedUSum,
    /// `vfredmax` — FP reduction maximum.
    VFRedMax,
    /// `vfredmin` — FP reduction minimum.
    VFRedMin,

    /// `vfwredosum` — widening FP ordered reduction sum.
    VFWRedOSum,
    /// `vfwredusum` — widening FP unordered reduction sum.
    VFWRedUSum,

    /// `vmand.mm` — mask AND.
    VMAndMM,
    /// `vmnand.mm` — mask NAND.
    VMNandMM,
    /// `vmandn.mm` — mask AND-NOT.
    VMAndnMM,
    /// `vmor.mm` — mask OR.
    VMOrMM,
    /// `vmnor.mm` — mask NOR.
    VMNorMM,
    /// `vmorn.mm` — mask OR-NOT.
    VMOrnMM,
    /// `vmxor.mm` — mask XOR.
    VMXorMM,
    /// `vmxnor.mm` — mask XNOR.
    VMXnorMM,

    /// `vcpop.m` — count population of mask register.
    VCPopM,
    /// `vfirst.m` — find first set bit in mask register.
    VFirstM,

    /// `vmsbf.m` — set-before-first mask bit.
    VMSbfM,
    /// `vmsif.m` — set-including-first mask bit.
    VMSifM,
    /// `vmsof.m` — set-only-first mask bit.
    VMSofM,

    /// `viota.m` — iota (prefix sum of mask bits).
    VIotaM,
    /// `vid.v` — vector element index.
    VIdV,

    /// `vmv.x.s` — move vector element 0 to scalar GPR.
    VMvXS,
    /// `vmv.s.x` — move scalar GPR to vector element 0.
    VMvSX,
    /// `vslideup` — slide elements up by the offset.
    VSlideUp(SlideOffset),
    /// `vslidedown` — slide elements down by the offset.
    VSlideDown(SlideOffset),
    /// `vslide1up` — slide up by one with scalar.
    VSlide1Up,
    /// `vslide1down` — slide down by one with scalar.
    VSlide1Down,
    /// `vrgather` — register gather (permute by index).
    VRgather,
    /// `vrgatherei16` — register gather with 16-bit indices.
    VRgatherEi16,
    /// `vcompress` — compress active elements.
    VCompress,
    /// `vmv1r` — whole-register move (1 register).
    VMv1r,
    /// `vmv2r` — whole-register move (2 registers).
    VMv2r,
    /// `vmv4r` — whole-register move (4 registers).
    VMv4r,
    /// `vmv8r` — whole-register move (8 registers).
    VMv8r,
}

/// Per-operand vector register group sizes for a given instruction.
///
/// Models how real hardware derives operand grouping from the opcode and LMUL.
/// A value of 0 means the operand field is not a vector register (it may be a
/// scalar GPR/FPR or a sub-opcode selector encoded in the vs1/vs2 field).
#[derive(Clone, Copy, Debug)]
pub struct VecOperandGroups {
    /// Number of registers in vd group (0 = scalar/sub-opcode, not a vreg).
    pub vd: u8,
    /// Number of registers in vs1 group (0 = scalar/immediate/sub-opcode).
    pub vs1: u8,
    /// Number of registers in vs2 group (0 = not used as vreg source).
    pub vs2: u8,
}

impl VectorOp {
    /// `vsetvli`, `vsetivli` or `vsetvl`: writes `vtype` and `vl` instead of
    /// a vector register.
    #[must_use]
    pub const fn is_config(self) -> bool {
        matches!(self, Self::Vsetvli | Self::Vsetivli | Self::Vsetvl)
    }

    /// Compute the vector register group size for each operand given the base
    /// LMUL (1, 2, 4, or 8) and the source encoding.
    ///
    /// This is the single source of truth for operand grouping — every pipeline
    /// stage (decode alignment check, rename, execute sync, commit) should call
    /// this rather than maintaining separate per-stage logic.
    ///
    /// RVV 1.0 operand semantics:
    ///  - Most arithmetic: vd, vs2, vs1 are all LMUL-sized groups.
    ///  - Widening: vd = 2×LMUL, vs2 = LMUL (or 2×LMUL for .wv/.wf), vs1 = LMUL.
    ///  - Narrowing: vd = LMUL, vs2 = 2×LMUL.
    ///  - Scalar-result (vmv.x.s, vcpop, vfirst, vfmv.f.s): vd = 0 (scalar GPR/FPR).
    ///  - Mask-destination (comparisons, vmadc, vmsbf …): vd = 1 (single mask reg).
    ///  - Mask-source (vcpop, vmsbf …): vs2 = 1 (single mask reg).
    ///  - Sub-opcode in vs1 (UNARY0 families): vs1 = 0.
    ///  - Mask logical: all operands are single mask registers = 1.
    ///  - Whole-register moves/loads/stores: fixed group sizes independent of LMUL.
    ///
    /// `lmul` is the source register-group count (1/2/4/8 for both fractional
    /// LMUL and M1 — they all share group=1). `lmul_is_fractional` distinguishes
    /// fractional LMUL (Mf8/Mf4/Mf2) from M1: it matters for widening, where
    /// `2*LMUL` for fractional still fits in 1 register but `2*M1 = M2` needs 2.
    #[allow(clippy::enum_glob_use)]
    pub fn operand_groups(
        self,
        lmul: u8,
        lmul_is_fractional: bool,
        src_enc: VecSrcEncoding,
        nf: u8,
        broadcast_vs2: bool,
    ) -> VecOperandGroups {
        use VectorOp::*;

        // Doubled register group for widening / narrowing. For fractional LMUL,
        // `2*LMUL` is still ≤ 1 register (e.g. 2*Mf8 = Mf4), so emul_widened = 1
        // even though `lmul == 1` would otherwise produce 2.
        let emul_widened: u8 = if lmul_is_fractional { 1 } else { (lmul * 2).min(8) };

        // vs1 is only a vector register for VV encoding; for VX/VI/VF it's a
        // scalar or immediate, so group size = 0.
        let vs1_base = if src_enc == VecSrcEncoding::VV { lmul } else { 0 };

        // .vs-form crypto ops broadcast vs2 element group 0; vs2 EMUL is 1.
        let vs2_crypto = if broadcast_vs2 { 1 } else { lmul };

        match self {
            None | Vsetvli | Vsetivli | Vsetvl => VecOperandGroups { vd: 0, vs1: 0, vs2: 0 },

            VAdd | VSub | VRsub | VAnd | VOr | VXor | VSll | VSrl | VSra | VMinU | VMin | VMaxU
            | VMax | VMul | VMulh | VMulhu | VMulhsu | VMacc | VNMSac | VMadd | VNMSub | VDivU
            | VDiv | VRemU | VRem | VSAddU | VSAdd | VSSubU | VSSub | VAAddU | VAAdd | VASubU
            | VASub | VSmul | VSSrl | VSSra | VMerge | VSlideUp(_) | VSlideDown(_) | VSlide1Up
            | VSlide1Down | VRgather | VRgatherEi16 | VCompress | VFAdd | VFSub | VFRSub
            | VFMul | VFDiv | VFRDiv | VFMin | VFMax | VFSgnj | VFSgnjn | VFSgnjx | VFMacc
            | VFNMacc | VFMSac | VFNMSac | VFMAdd | VFNMAdd | VFMSub | VFNMSub | VFMerge
            | VFSlide1Up | VFSlide1Down | VAdc | VSbc | VAndN | VRol | VRor | VClMul | VClMulH => {
                VecOperandGroups { vd: lmul, vs2: lmul, vs1: vs1_base }
            }

            // Zvknh / Zvksh vsm3me / Zvkg vghsh: vs1 is a per-group vector input.
            VSha2Ms | VSha2Ch | VSha2Cl | VSm3Me | VGhsh => {
                VecOperandGroups { vd: lmul, vs2: lmul, vs1: lmul }
            }

            // AES/SM4 rounds: vs1 field is a sub-opcode, not a register; .vs broadcasts vs2 EMUL=1.
            VAesEm | VAesEf | VAesDm | VAesDf | VAesZ | VSm4R => {
                VecOperandGroups { vd: lmul, vs2: vs2_crypto, vs1: 0 }
            }

            // vs1 field is a sub-opcode/imm here, not a register reference.
            VAesKf1 | VAesKf2 | VSm4K | VSm3C | VGmul | VFCvtXuF | VFCvtXF | VFCvtFXu | VFCvtFX
            | VFCvtRtzXuF | VFCvtRtzXF | VFSqrt | VFRsqrt7 | VFRec7 | VFClass | VLoadIndexOrd
            | VLoadIndexUnord | VStoreIndexOrd | VStoreIndexUnord | VBrev | VBrev8 | VRev8
            | VClz | VCtz | VCpopV => VecOperandGroups { vd: lmul, vs2: lmul, vs1: 0 },

            // RVV 1.0 §14.1: reduction vd/vs1 are single registers; vs2 is the full LMUL group.
            VRedSum | VRedAnd | VRedOr | VRedXor | VRedMinU | VRedMin | VRedMaxU | VRedMax
            | VFRedOSum | VFRedUSum | VFRedMax | VFRedMin | VWRedSumU | VWRedSum | VFWRedOSum
            | VFWRedUSum => VecOperandGroups { vd: 1, vs2: lmul, vs1: 1 },

            VWAddU | VWAdd | VWSubU | VWSub | VWMulU | VWMul | VWMulSU | VWMaccU | VWMacc
            | VWMaccSU | VWMaccUS | VWsll | VFWAdd | VFWSub | VFWMul | VFWMacc | VFWNMacc
            | VFWMSac | VFWNMSac => VecOperandGroups { vd: emul_widened, vs2: lmul, vs1: vs1_base },

            VWAddUW | VWAddW | VWSubUW | VWSubW | VFWAddW | VFWSubW => {
                VecOperandGroups { vd: emul_widened, vs2: emul_widened, vs1: vs1_base }
            }

            VNSrl | VNSra | VNClipU | VNClip => {
                VecOperandGroups { vd: lmul, vs2: emul_widened, vs1: vs1_base }
            }

            // Use 1 as minimum; vs2 EMUL = LMUL/factor may be fractional (<1 register).
            VZextVf2 | VSextVf2 => VecOperandGroups { vd: lmul, vs2: (lmul / 2).max(1), vs1: 0 },
            VZextVf4 | VSextVf4 => VecOperandGroups { vd: lmul, vs2: (lmul / 4).max(1), vs1: 0 },
            VZextVf8 | VSextVf8 => VecOperandGroups { vd: lmul, vs2: (lmul / 8).max(1), vs1: 0 },

            VFWCvtXuF | VFWCvtXF | VFWCvtFXu | VFWCvtFX | VFWCvtFF | VFWCvtRtzXuF | VFWCvtRtzXF => {
                VecOperandGroups { vd: emul_widened, vs2: lmul, vs1: 0 }
            }

            VFNCvtXuF | VFNCvtXF | VFNCvtFXu | VFNCvtFX | VFNCvtFF | VFNCvtRodFF | VFNCvtRtzXuF
            | VFNCvtRtzXF => VecOperandGroups { vd: lmul, vs2: emul_widened, vs1: 0 },

            // RVV 1.0 §16.1: vmv.x.s / vfmv.f.s read only element 0 of vs2.
            VMvXS | VFMvFS | VCPopM | VFirstM => VecOperandGroups { vd: 0, vs2: 1, vs1: 0 },

            // vmv.s.x / vfmv.s.f write only element 0 (§16.1); vlm/vsm: single mask register.
            VMvSX | VFMvSF | VLoadMask | VStoreMask => VecOperandGroups { vd: 1, vs2: 0, vs1: 0 },

            VMSeq | VMSne | VMSltu | VMSlt | VMSleu | VMSle | VMSgtu | VMSgt | VMFEq | VMFNe
            | VMFLt | VMFLe | VMFGt | VMFGe | VMadc | VMsbc => {
                VecOperandGroups { vd: 1, vs2: lmul, vs1: vs1_base }
            }

            VMSbfM | VMSofM | VMSifM | VMv1r => VecOperandGroups { vd: 1, vs2: 1, vs1: 0 },

            VIotaM => VecOperandGroups { vd: lmul, vs2: 1, vs1: 0 },

            // vid.v's vs2 field is part of the opcode (must be zero), not a register.
            VIdV | VLoadUnit | VLoadFF | VStoreUnit | VLoadStride | VStoreStride => {
                VecOperandGroups { vd: lmul, vs2: 0, vs1: 0 }
            }

            VMAndMM | VMNandMM | VMAndnMM | VMOrMM | VMNorMM | VMOrnMM | VMXorMM | VMXnorMM => {
                VecOperandGroups { vd: 1, vs2: 1, vs1: 1 }
            }

            VMv2r => VecOperandGroups { vd: 2, vs2: 2, vs1: 0 },
            VMv4r => VecOperandGroups { vd: 4, vs2: 4, vs1: 0 },
            VMv8r => VecOperandGroups { vd: 8, vs2: 8, vs1: 0 },

            // nf encoding: 0=1reg, 1=2reg, 3=4reg, 7=8reg; group = nf+1.
            VLoadWholeReg | VStoreWholeReg => {
                let regs = nf + 1;
                VecOperandGroups { vd: regs, vs2: 0, vs1: 0 }
            }
        }
    }
}

/// Where a slide takes its element offset: the ISA allows `.vx` and `.vi`
/// forms only, so an executor never sees a vector operand here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlideOffset {
    /// The value of `rs1`.
    Rs1,
    /// A zero-extended 5-bit immediate.
    Imm(u8),
}

/// Element-wise integer operations: one result element per source element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntOp {
    /// `vadd`.
    Add,
    /// `vsub`.
    Sub,
    /// `vrsub`.
    Rsub,
    /// `vand`.
    And,
    /// `vor`.
    Or,
    /// `vxor`.
    Xor,
    /// `vsll`.
    Sll,
    /// `vsrl`.
    Srl,
    /// `vsra`.
    Sra,
    /// `vminu`.
    MinU,
    /// `vmin`.
    Min,
    /// `vmaxu`.
    MaxU,
    /// `vmax`.
    Max,
    /// `vmul`.
    Mul,
    /// `vmulh`.
    Mulh,
    /// `vmulhu`.
    Mulhu,
    /// `vmulhsu`.
    Mulhsu,
    /// `vdivu`.
    DivU,
    /// `vdiv`.
    Div,
    /// `vremu`.
    RemU,
    /// `vrem`.
    Rem,
    /// `vsaddu`.
    SAddU,
    /// `vsadd`.
    SAdd,
    /// `vssubu`.
    SSubU,
    /// `vssub`.
    SSub,
    /// `vaaddu`.
    AAddU,
    /// `vaadd`.
    AAdd,
    /// `vasubu`.
    ASubU,
    /// `vasub`.
    ASub,
    /// `vsmul`.
    Smul,
    /// `vssrl`.
    SSrl,
    /// `vssra`.
    SSra,
    /// `vandn`.
    AndN,
    /// `vbrev.v`.
    Brev,
    /// `vbrev8.v`.
    Brev8,
    /// `vrev8.v`.
    Rev8,
    /// `vclz.v`.
    Clz,
    /// `vctz.v`.
    Ctz,
    /// `vcpop.v`.
    CpopV,
    /// `vrol`.
    Rol,
    /// `vror`.
    Ror,
    /// `vclmul`.
    ClMul,
    /// `vclmulh`.
    ClMulH,
}

/// Integer comparisons that write a mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareOp {
    /// `vmseq`.
    Eq,
    /// `vmsne`.
    Ne,
    /// `vmsltu`.
    LtU,
    /// `vmslt`.
    Lt,
    /// `vmsleu`.
    LeU,
    /// `vmsle`.
    Le,
    /// `vmsgtu`.
    GtU,
    /// `vmsgt`.
    Gt,
}

/// Add and subtract with the carry in `v0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CarryOp {
    /// `vadc`: the sum.
    Adc,
    /// `vmadc`: the carry out, as a mask.
    Madc,
    /// `vsbc`: the difference.
    Sbc,
    /// `vmsbc`: the borrow out, as a mask.
    Msbc,
}

impl CarryOp {
    /// True for the forms that write a mask rather than elements.
    #[must_use]
    pub const fn writes_mask(self) -> bool {
        matches!(self, Self::Madc | Self::Msbc)
    }
}

/// Multiply-accumulate at SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaccOp {
    /// `vmacc`: `vd = vs1 * vs2 + vd`.
    Macc,
    /// `vnmsac`: `vd = -(vs1 * vs2) + vd`.
    NMSac,
    /// `vmadd`: `vd = vs1 * vd + vs2`.
    Madd,
    /// `vnmsub`: `vd = -(vs1 * vd) + vs2`.
    NMSub,
}

/// Widening arithmetic: sources at SEW (or `vs2` at 2×SEW for the `.w`
/// forms), result at 2×SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidenOp {
    /// `vwaddu`.
    AddU,
    /// `vwadd`.
    Add,
    /// `vwsubu`.
    SubU,
    /// `vwsub`.
    Sub,
    /// `vwaddu.w`.
    AddUW,
    /// `vwadd.w`.
    AddW,
    /// `vwsubu.w`.
    SubUW,
    /// `vwsub.w`.
    SubW,
    /// `vwmulu`.
    MulU,
    /// `vwmul`.
    Mul,
    /// `vwmulsu`.
    MulSU,
    /// `vwsll`.
    Sll,
}

impl WidenOp {
    /// True for the `.w` forms, which read `vs2` at the wide width.
    #[must_use]
    pub const fn reads_wide_vs2(self) -> bool {
        matches!(self, Self::AddUW | Self::AddW | Self::SubUW | Self::SubW)
    }
}

/// Widening multiply-accumulate: product at 2×SEW added to `vd`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidenMaccOp {
    /// `vwmaccu`.
    MaccU,
    /// `vwmacc`.
    Macc,
    /// `vwmaccsu`.
    MaccSU,
    /// `vwmaccus`.
    MaccUS,
}

/// Narrowing shifts and clips: `vs2` at 2×SEW, result at SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NarrowOp {
    /// `vnsrl`.
    Srl,
    /// `vnsra`.
    Sra,
    /// `vnclipu`.
    ClipU,
    /// `vnclip`.
    Clip,
}

/// Integer extension from a fraction of SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtendOp {
    /// Sign-extend rather than zero-extend.
    pub signed: bool,
    /// The source width is `SEW / factor`: 2, 4 or 8.
    pub factor: u8,
}

/// What the vector integer ALU computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecAluOp {
    /// An element-wise operation at SEW.
    Int(IntOp),
    /// A comparison into a mask.
    Compare(CompareOp),
    /// Add or subtract with carry.
    Carry(CarryOp),
    /// Multiply-accumulate.
    Macc(MaccOp),
    /// Widening arithmetic.
    Widen(WidenOp),
    /// Widening multiply-accumulate.
    WidenMacc(WidenMaccOp),
    /// Narrowing shift or clip.
    Narrow(NarrowOp),
    /// Zero- or sign-extension.
    Extend(ExtendOp),
    /// `vmerge` / `vmv.v`.
    Merge,
}

/// Integer reductions at SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntReduceOp {
    /// `vredsum`.
    Sum,
    /// `vredand`.
    And,
    /// `vredor`.
    Or,
    /// `vredxor`.
    Xor,
    /// `vredminu`.
    MinU,
    /// `vredmin`.
    Min,
    /// `vredmaxu`.
    MaxU,
    /// `vredmax`.
    Max,
}

/// Widening integer reductions: sources at SEW, accumulator at 2×SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidenIntReduceOp {
    /// `vwredsumu`.
    SumU,
    /// `vwredsum`.
    Sum,
}

/// Floating-point reductions at SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpReduceOp {
    /// `vfredosum`.
    OSum,
    /// `vfredusum`.
    USum,
    /// `vfredmax`.
    Max,
    /// `vfredmin`.
    Min,
}

/// Widening floating-point reductions: sources at SEW, accumulator at 2×SEW.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpWidenReduceOp {
    /// `vfwredosum`.
    OSum,
    /// `vfwredusum`.
    USum,
}

/// A reduction of `vs2` into element 0 of `vd`, seeded from `vs1[0]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReduceOp {
    /// Integer, at SEW.
    Int(IntReduceOp),
    /// Integer, widening.
    WidenInt(WidenIntReduceOp),
    /// Floating-point, at SEW.
    Fp(FpReduceOp),
    /// Floating-point, widening.
    FpWiden(FpWidenReduceOp),
}

/// Bitwise operations between mask registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskLogicalOp {
    /// `vmand.mm`.
    And,
    /// `vmnand.mm`.
    Nand,
    /// `vmandn.mm`.
    AndNot,
    /// `vmor.mm`.
    Or,
    /// `vmnor.mm`.
    Nor,
    /// `vmorn.mm`.
    OrNot,
    /// `vmxor.mm`.
    Xor,
    /// `vmxnor.mm`.
    Xnor,
}

/// The set-before/including/only-first mask operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskSetOp {
    /// `vmsbf.m`: set every bit before the first set source bit.
    BeforeFirst,
    /// `vmsif.m`: set every bit up to and including the first set source bit.
    IncludingFirst,
    /// `vmsof.m`: set only the first set source bit.
    OnlyFirst,
}

/// Operations on mask registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskOp {
    /// A bitwise operation between two masks.
    Logical(MaskLogicalOp),
    /// `vcpop.m`: count the set bits.
    CPop,
    /// `vfirst.m`: index of the first set bit.
    First,
    /// `vmsbf.m`, `vmsif.m` or `vmsof.m`.
    Set(MaskSetOp),
    /// `viota.m`.
    Iota,
    /// `vid.v`.
    Id,
}

/// Permutations: moves, slides, gathers and compress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermuteOp {
    /// `vmv.x.s`.
    MvXS,
    /// `vmv.s.x`.
    MvSX,
    /// `vslideup` by the offset.
    SlideUp(SlideOffset),
    /// `vslidedown` by the offset.
    SlideDown(SlideOffset),
    /// `vslide1up`.
    Slide1Up,
    /// `vslide1down`.
    Slide1Down,
    /// `vrgather`.
    Rgather,
    /// `vrgatherei16`.
    RgatherEi16,
    /// `vcompress`.
    Compress,
    /// `vmv<n>r.v`: a whole-register move of `n` registers.
    WholeMove(u8),
}

/// Vector cryptography operations (Zvkned, Zvknha/b, Zvksed, Zvksh, Zvkg).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoOp {
    /// `vaesem`.
    AesEm,
    /// `vaesef`.
    AesEf,
    /// `vaesdm`.
    AesDm,
    /// `vaesdf`.
    AesDf,
    /// `vaesz`.
    AesZ,
    /// `vaeskf1`.
    AesKf1,
    /// `vaeskf2`.
    AesKf2,
    /// `vsha2ms`.
    Sha2Ms,
    /// `vsha2ch`.
    Sha2Ch,
    /// `vsha2cl`.
    Sha2Cl,
    /// `vsm3me`.
    Sm3Me,
    /// `vsm3c`.
    Sm3C,
    /// `vsm4r`.
    Sm4R,
    /// `vsm4k`.
    Sm4K,
    /// `vgmul`.
    Gmul,
    /// `vghsh`.
    Ghsh,
}

/// Which unit executes a [`VectorOp`], and what it computes there.
///
/// Every op belongs to exactly one class, so an executor that takes a
/// class's op type cannot be handed an op it does not implement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecClass {
    /// Not a vector operation.
    None,
    /// `vsetvl`, `vsetvli` or `vsetivli`.
    Config,
    /// A vector load.
    Load,
    /// A vector store.
    Store,
    /// The floating-point unit's element-wise operations.
    Fp,
    /// The integer ALU.
    Alu(VecAluOp),
    /// A reduction.
    Reduce(ReduceOp),
    /// A mask operation.
    Mask(MaskOp),
    /// A permutation.
    Permute(PermuteOp),
    /// A cryptography operation.
    Crypto(CryptoOp),
}

impl VectorOp {
    /// The class this op executes as.
    #[must_use]
    #[allow(clippy::too_many_lines, clippy::enum_glob_use)]
    pub const fn class(self) -> VecClass {
        use VectorOp::*;
        match self {
            None => VecClass::None,
            Vsetvli | Vsetivli | Vsetvl => VecClass::Config,

            VLoadUnit | VLoadFF | VLoadMask | VLoadWholeReg | VLoadStride | VLoadIndexOrd
            | VLoadIndexUnord => VecClass::Load,
            VStoreUnit | VStoreMask | VStoreWholeReg | VStoreStride | VStoreIndexOrd
            | VStoreIndexUnord => VecClass::Store,

            VFAdd | VFSub | VFRSub | VFMul | VFDiv | VFRDiv | VFMin | VFMax | VFSgnj | VFSgnjn
            | VFSgnjx | VMFEq | VMFNe | VMFLt | VMFLe | VMFGt | VMFGe | VFMacc | VFNMacc
            | VFMSac | VFNMSac | VFMAdd | VFNMAdd | VFMSub | VFNMSub | VFSqrt | VFRsqrt7
            | VFRec7 | VFClass | VFCvtXuF | VFCvtXF | VFCvtFXu | VFCvtFX | VFCvtRtzXuF
            | VFCvtRtzXF | VFWAdd | VFWSub | VFWMul | VFWAddW | VFWSubW | VFWMacc | VFWNMacc
            | VFWMSac | VFWNMSac | VFWCvtXuF | VFWCvtXF | VFWCvtFXu | VFWCvtFX | VFWCvtFF
            | VFWCvtRtzXuF | VFWCvtRtzXF | VFNCvtXuF | VFNCvtXF | VFNCvtFXu | VFNCvtFX
            | VFNCvtFF | VFNCvtRodFF | VFNCvtRtzXuF | VFNCvtRtzXF | VFMerge | VFMvSF | VFMvFS
            | VFSlide1Up | VFSlide1Down => VecClass::Fp,

            VAdd => VecClass::Alu(VecAluOp::Int(IntOp::Add)),
            VSub => VecClass::Alu(VecAluOp::Int(IntOp::Sub)),
            VRsub => VecClass::Alu(VecAluOp::Int(IntOp::Rsub)),
            VAnd => VecClass::Alu(VecAluOp::Int(IntOp::And)),
            VOr => VecClass::Alu(VecAluOp::Int(IntOp::Or)),
            VXor => VecClass::Alu(VecAluOp::Int(IntOp::Xor)),
            VSll => VecClass::Alu(VecAluOp::Int(IntOp::Sll)),
            VSrl => VecClass::Alu(VecAluOp::Int(IntOp::Srl)),
            VSra => VecClass::Alu(VecAluOp::Int(IntOp::Sra)),
            VMinU => VecClass::Alu(VecAluOp::Int(IntOp::MinU)),
            VMin => VecClass::Alu(VecAluOp::Int(IntOp::Min)),
            VMaxU => VecClass::Alu(VecAluOp::Int(IntOp::MaxU)),
            VMax => VecClass::Alu(VecAluOp::Int(IntOp::Max)),
            VMul => VecClass::Alu(VecAluOp::Int(IntOp::Mul)),
            VMulh => VecClass::Alu(VecAluOp::Int(IntOp::Mulh)),
            VMulhu => VecClass::Alu(VecAluOp::Int(IntOp::Mulhu)),
            VMulhsu => VecClass::Alu(VecAluOp::Int(IntOp::Mulhsu)),
            VDivU => VecClass::Alu(VecAluOp::Int(IntOp::DivU)),
            VDiv => VecClass::Alu(VecAluOp::Int(IntOp::Div)),
            VRemU => VecClass::Alu(VecAluOp::Int(IntOp::RemU)),
            VRem => VecClass::Alu(VecAluOp::Int(IntOp::Rem)),
            VSAddU => VecClass::Alu(VecAluOp::Int(IntOp::SAddU)),
            VSAdd => VecClass::Alu(VecAluOp::Int(IntOp::SAdd)),
            VSSubU => VecClass::Alu(VecAluOp::Int(IntOp::SSubU)),
            VSSub => VecClass::Alu(VecAluOp::Int(IntOp::SSub)),
            VAAddU => VecClass::Alu(VecAluOp::Int(IntOp::AAddU)),
            VAAdd => VecClass::Alu(VecAluOp::Int(IntOp::AAdd)),
            VASubU => VecClass::Alu(VecAluOp::Int(IntOp::ASubU)),
            VASub => VecClass::Alu(VecAluOp::Int(IntOp::ASub)),
            VSmul => VecClass::Alu(VecAluOp::Int(IntOp::Smul)),
            VSSrl => VecClass::Alu(VecAluOp::Int(IntOp::SSrl)),
            VSSra => VecClass::Alu(VecAluOp::Int(IntOp::SSra)),
            VAndN => VecClass::Alu(VecAluOp::Int(IntOp::AndN)),
            VBrev => VecClass::Alu(VecAluOp::Int(IntOp::Brev)),
            VBrev8 => VecClass::Alu(VecAluOp::Int(IntOp::Brev8)),
            VRev8 => VecClass::Alu(VecAluOp::Int(IntOp::Rev8)),
            VClz => VecClass::Alu(VecAluOp::Int(IntOp::Clz)),
            VCtz => VecClass::Alu(VecAluOp::Int(IntOp::Ctz)),
            VCpopV => VecClass::Alu(VecAluOp::Int(IntOp::CpopV)),
            VRol => VecClass::Alu(VecAluOp::Int(IntOp::Rol)),
            VRor => VecClass::Alu(VecAluOp::Int(IntOp::Ror)),
            VClMul => VecClass::Alu(VecAluOp::Int(IntOp::ClMul)),
            VClMulH => VecClass::Alu(VecAluOp::Int(IntOp::ClMulH)),

            VMerge => VecClass::Alu(VecAluOp::Merge),

            VMSeq => VecClass::Alu(VecAluOp::Compare(CompareOp::Eq)),
            VMSne => VecClass::Alu(VecAluOp::Compare(CompareOp::Ne)),
            VMSltu => VecClass::Alu(VecAluOp::Compare(CompareOp::LtU)),
            VMSlt => VecClass::Alu(VecAluOp::Compare(CompareOp::Lt)),
            VMSleu => VecClass::Alu(VecAluOp::Compare(CompareOp::LeU)),
            VMSle => VecClass::Alu(VecAluOp::Compare(CompareOp::Le)),
            VMSgtu => VecClass::Alu(VecAluOp::Compare(CompareOp::GtU)),
            VMSgt => VecClass::Alu(VecAluOp::Compare(CompareOp::Gt)),

            VAdc => VecClass::Alu(VecAluOp::Carry(CarryOp::Adc)),
            VMadc => VecClass::Alu(VecAluOp::Carry(CarryOp::Madc)),
            VSbc => VecClass::Alu(VecAluOp::Carry(CarryOp::Sbc)),
            VMsbc => VecClass::Alu(VecAluOp::Carry(CarryOp::Msbc)),

            VMacc => VecClass::Alu(VecAluOp::Macc(MaccOp::Macc)),
            VNMSac => VecClass::Alu(VecAluOp::Macc(MaccOp::NMSac)),
            VMadd => VecClass::Alu(VecAluOp::Macc(MaccOp::Madd)),
            VNMSub => VecClass::Alu(VecAluOp::Macc(MaccOp::NMSub)),

            VWAddU => VecClass::Alu(VecAluOp::Widen(WidenOp::AddU)),
            VWAdd => VecClass::Alu(VecAluOp::Widen(WidenOp::Add)),
            VWSubU => VecClass::Alu(VecAluOp::Widen(WidenOp::SubU)),
            VWSub => VecClass::Alu(VecAluOp::Widen(WidenOp::Sub)),
            VWAddUW => VecClass::Alu(VecAluOp::Widen(WidenOp::AddUW)),
            VWAddW => VecClass::Alu(VecAluOp::Widen(WidenOp::AddW)),
            VWSubUW => VecClass::Alu(VecAluOp::Widen(WidenOp::SubUW)),
            VWSubW => VecClass::Alu(VecAluOp::Widen(WidenOp::SubW)),
            VWMulU => VecClass::Alu(VecAluOp::Widen(WidenOp::MulU)),
            VWMul => VecClass::Alu(VecAluOp::Widen(WidenOp::Mul)),
            VWMulSU => VecClass::Alu(VecAluOp::Widen(WidenOp::MulSU)),
            VWsll => VecClass::Alu(VecAluOp::Widen(WidenOp::Sll)),

            VWMaccU => VecClass::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccU)),
            VWMacc => VecClass::Alu(VecAluOp::WidenMacc(WidenMaccOp::Macc)),
            VWMaccSU => VecClass::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccSU)),
            VWMaccUS => VecClass::Alu(VecAluOp::WidenMacc(WidenMaccOp::MaccUS)),

            VNSrl => VecClass::Alu(VecAluOp::Narrow(NarrowOp::Srl)),
            VNSra => VecClass::Alu(VecAluOp::Narrow(NarrowOp::Sra)),
            VNClipU => VecClass::Alu(VecAluOp::Narrow(NarrowOp::ClipU)),
            VNClip => VecClass::Alu(VecAluOp::Narrow(NarrowOp::Clip)),

            VZextVf2 => VecClass::Alu(VecAluOp::Extend(ExtendOp { signed: false, factor: 2 })),
            VZextVf4 => VecClass::Alu(VecAluOp::Extend(ExtendOp { signed: false, factor: 4 })),
            VZextVf8 => VecClass::Alu(VecAluOp::Extend(ExtendOp { signed: false, factor: 8 })),
            VSextVf2 => VecClass::Alu(VecAluOp::Extend(ExtendOp { signed: true, factor: 2 })),
            VSextVf4 => VecClass::Alu(VecAluOp::Extend(ExtendOp { signed: true, factor: 4 })),
            VSextVf8 => VecClass::Alu(VecAluOp::Extend(ExtendOp { signed: true, factor: 8 })),

            VRedSum => VecClass::Reduce(ReduceOp::Int(IntReduceOp::Sum)),
            VRedAnd => VecClass::Reduce(ReduceOp::Int(IntReduceOp::And)),
            VRedOr => VecClass::Reduce(ReduceOp::Int(IntReduceOp::Or)),
            VRedXor => VecClass::Reduce(ReduceOp::Int(IntReduceOp::Xor)),
            VRedMinU => VecClass::Reduce(ReduceOp::Int(IntReduceOp::MinU)),
            VRedMin => VecClass::Reduce(ReduceOp::Int(IntReduceOp::Min)),
            VRedMaxU => VecClass::Reduce(ReduceOp::Int(IntReduceOp::MaxU)),
            VRedMax => VecClass::Reduce(ReduceOp::Int(IntReduceOp::Max)),
            VWRedSumU => VecClass::Reduce(ReduceOp::WidenInt(WidenIntReduceOp::SumU)),
            VWRedSum => VecClass::Reduce(ReduceOp::WidenInt(WidenIntReduceOp::Sum)),
            VFRedOSum => VecClass::Reduce(ReduceOp::Fp(FpReduceOp::OSum)),
            VFRedUSum => VecClass::Reduce(ReduceOp::Fp(FpReduceOp::USum)),
            VFRedMax => VecClass::Reduce(ReduceOp::Fp(FpReduceOp::Max)),
            VFRedMin => VecClass::Reduce(ReduceOp::Fp(FpReduceOp::Min)),
            VFWRedOSum => VecClass::Reduce(ReduceOp::FpWiden(FpWidenReduceOp::OSum)),
            VFWRedUSum => VecClass::Reduce(ReduceOp::FpWiden(FpWidenReduceOp::USum)),

            VMAndMM => VecClass::Mask(MaskOp::Logical(MaskLogicalOp::And)),
            VMNandMM => VecClass::Mask(MaskOp::Logical(MaskLogicalOp::Nand)),
            VMAndnMM => VecClass::Mask(MaskOp::Logical(MaskLogicalOp::AndNot)),
            VMOrMM => VecClass::Mask(MaskOp::Logical(MaskLogicalOp::Or)),
            VMNorMM => VecClass::Mask(MaskOp::Logical(MaskLogicalOp::Nor)),
            VMOrnMM => VecClass::Mask(MaskOp::Logical(MaskLogicalOp::OrNot)),
            VMXorMM => VecClass::Mask(MaskOp::Logical(MaskLogicalOp::Xor)),
            VMXnorMM => VecClass::Mask(MaskOp::Logical(MaskLogicalOp::Xnor)),
            VCPopM => VecClass::Mask(MaskOp::CPop),
            VFirstM => VecClass::Mask(MaskOp::First),
            VMSbfM => VecClass::Mask(MaskOp::Set(MaskSetOp::BeforeFirst)),
            VMSifM => VecClass::Mask(MaskOp::Set(MaskSetOp::IncludingFirst)),
            VMSofM => VecClass::Mask(MaskOp::Set(MaskSetOp::OnlyFirst)),
            VIotaM => VecClass::Mask(MaskOp::Iota),
            VIdV => VecClass::Mask(MaskOp::Id),

            VMvXS => VecClass::Permute(PermuteOp::MvXS),
            VMvSX => VecClass::Permute(PermuteOp::MvSX),
            VSlideUp(offset) => VecClass::Permute(PermuteOp::SlideUp(offset)),
            VSlideDown(offset) => VecClass::Permute(PermuteOp::SlideDown(offset)),
            VSlide1Up => VecClass::Permute(PermuteOp::Slide1Up),
            VSlide1Down => VecClass::Permute(PermuteOp::Slide1Down),
            VRgather => VecClass::Permute(PermuteOp::Rgather),
            VRgatherEi16 => VecClass::Permute(PermuteOp::RgatherEi16),
            VCompress => VecClass::Permute(PermuteOp::Compress),
            VMv1r => VecClass::Permute(PermuteOp::WholeMove(1)),
            VMv2r => VecClass::Permute(PermuteOp::WholeMove(2)),
            VMv4r => VecClass::Permute(PermuteOp::WholeMove(4)),
            VMv8r => VecClass::Permute(PermuteOp::WholeMove(8)),

            VAesEm => VecClass::Crypto(CryptoOp::AesEm),
            VAesEf => VecClass::Crypto(CryptoOp::AesEf),
            VAesDm => VecClass::Crypto(CryptoOp::AesDm),
            VAesDf => VecClass::Crypto(CryptoOp::AesDf),
            VAesZ => VecClass::Crypto(CryptoOp::AesZ),
            VAesKf1 => VecClass::Crypto(CryptoOp::AesKf1),
            VAesKf2 => VecClass::Crypto(CryptoOp::AesKf2),
            VSha2Ms => VecClass::Crypto(CryptoOp::Sha2Ms),
            VSha2Ch => VecClass::Crypto(CryptoOp::Sha2Ch),
            VSha2Cl => VecClass::Crypto(CryptoOp::Sha2Cl),
            VSm3Me => VecClass::Crypto(CryptoOp::Sm3Me),
            VSm3C => VecClass::Crypto(CryptoOp::Sm3C),
            VSm4R => VecClass::Crypto(CryptoOp::Sm4R),
            VSm4K => VecClass::Crypto(CryptoOp::Sm4K),
            VGmul => VecClass::Crypto(CryptoOp::Gmul),
            VGhsh => VecClass::Crypto(CryptoOp::Ghsh),
        }
    }
}

/// Vector operand source encoding category.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VecSrcEncoding {
    /// Not a vector source encoding.
    #[default]
    None,
    /// Vector-vector (OPIVV, OPFVV, OPMVV).
    VV,
    /// Vector-scalar integer (OPIVX, OPMVX).
    VX,
    /// Vector-immediate (OPIVI).
    VI,
    /// Vector-scalar FP (OPFVF).
    VF,
}
