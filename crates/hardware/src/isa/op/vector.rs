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

    /// `vandn` — vector bitwise AND-NOT (vd[i] = vs2[i] & ~op1[i]).
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
    /// `vslideup` — slide elements up.
    VSlideUp,
    /// `vslidedown` — slide elements down.
    VSlideDown,
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
            | VASub | VSmul | VSSrl | VSSra | VMerge | VSlideUp | VSlideDown | VSlide1Up
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
