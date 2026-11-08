//! Disassembly of the vector instructions.

use super::{freg, xreg};
use crate::exec::signals::ControlSignals;
use crate::isa::instruction::Decoded;
use crate::isa::op::{
    CarryOp, CompareOp, CryptoOp, ExtendFactor, FmaAddend, FmaSigns, FpBinaryOp, FpCompareOp,
    FpNarrowConvertOp, FpReduceOp, FpUnaryOp, FpWidenConvertOp, FpWidenOp, FpWidenReduceOp,
    IntConvert, IntOp, IntReduceOp, IntRounding, MaccOp, MaskLogicalOp, MaskOp, MaskSetOp,
    NarrowOp, PermuteOp, ReduceOp, Signedness, VecAluOp, VecConfigOp, VecFpOp, VecLoadOp,
    VecSrcEncoding, VecStoreOp, VectorOp, WidenIntReduceOp, WidenMaccOp, WidenOp,
};
use crate::isa::rvv::VRegIdx;

/// Names of v0–v31.
const VREG_NAMES: [&str; 32] = [
    "v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7", "v8", "v9", "v10", "v11", "v12", "v13", "v14",
    "v15", "v16", "v17", "v18", "v19", "v20", "v21", "v22", "v23", "v24", "v25", "v26", "v27",
    "v28", "v29", "v30", "v31",
];

/// Disassembles the vector instruction decode turned into `op` and `ctrl`.
pub(super) fn disassemble(op: VectorOp, ctrl: &ControlSignals, fields: &Decoded) -> String {
    let operands = Operands::of(ctrl, fields);
    match op {
        VectorOp::Config(op) => config(op, fields),
        VectorOp::Load(op) => load(op, ctrl, fields, operands.mask),
        VectorOp::Store(op) => store(op, ctrl, fields, operands.mask),
        VectorOp::Alu(op) => alu(op, &operands),
        VectorOp::Fp(op) => floating_point(op, ctrl, fields, &operands),
        VectorOp::Reduce(op) => {
            let Operands { vd, vs2, source, mask, .. } = operands;
            format!("{}.vs {vd}, {vs2}, {source}{mask}", reduce_name(op))
        }
        VectorOp::Mask(op) => mask(op, ctrl, fields, &operands),
        VectorOp::Permute(op) => permute(op, ctrl, fields, &operands),
        VectorOp::Crypto(op) => crypto(op, ctrl, &operands),
    }
}

/// A vector instruction's operands as text: `source` is vs1, rs1, fs1 or
/// the immediate, as `letter` (`v`, `x`, `f` or `i`) names it.
struct Operands {
    vd: &'static str,
    vs2: &'static str,
    source: String,
    letter: char,
    mask: &'static str,
}

impl Operands {
    fn of(ctrl: &ControlSignals, fields: &Decoded) -> Self {
        let (source, letter) = match ctrl.vec_src_encoding {
            VecSrcEncoding::VV | VecSrcEncoding::None => (vreg(ctrl.vs1).to_owned(), 'v'),
            VecSrcEncoding::VX => (xreg(fields.rs1).to_owned(), 'x'),
            VecSrcEncoding::VF => (freg(fields.rs1).to_owned(), 'f'),
            VecSrcEncoding::VI(imm) => (imm.to_string(), 'i'),
        };
        let mask = if ctrl.vm { "" } else { ", v0.t" };
        Self { vd: vreg(ctrl.vd), vs2: vreg(ctrl.vs2), source, letter, mask }
    }

    /// `name.vv vd, vs2, vs1`, or `.wv` and the like when `first` is `w`.
    fn binary(&self, name: &str, first: char) -> String {
        let Self { vd, vs2, source, letter, mask } = self;
        format!("{name}.{first}{letter} {vd}, {vs2}, {source}{mask}")
    }

    /// The multiply-adds, whose multiplier comes first: `name.vv vd, vs1, vs2`.
    fn multiply_add(&self, name: &str) -> String {
        let Self { vd, vs2, source, letter, mask } = self;
        format!("{name}.v{letter} {vd}, {source}, {vs2}{mask}")
    }

    /// `name vd, vs2`, where `name` carries its own suffix.
    fn unary(&self, name: &str) -> String {
        let Self { vd, vs2, mask, .. } = self;
        format!("{name} {vd}, {vs2}{mask}")
    }

    /// `name.vvm vd, vs2, vs1, v0` when masked, which takes v0 as an
    /// operand; `unmasked` otherwise.
    fn with_carry_or(&self, name: &str, unmasked: impl FnOnce(&Self) -> String) -> String {
        if self.mask.is_empty() {
            return unmasked(self);
        }
        let Self { vd, vs2, source, letter, .. } = self;
        format!("{name}.v{letter}m {vd}, {vs2}, {source}, v0")
    }
}

const fn vreg(idx: VRegIdx) -> &'static str {
    VREG_NAMES[idx.as_usize()]
}

/// `vsetvli`, `vsetivli` or `vsetvl`.
fn config(op: VecConfigOp, fields: &Decoded) -> String {
    let rd = xreg(fields.rd);
    match op {
        VecConfigOp::Vsetvli { vtype } => {
            format!("vsetvli {rd}, {}, {}", xreg(fields.rs1), vtype_text(vtype))
        }
        VecConfigOp::Vsetivli { avl, vtype } => {
            format!("vsetivli {rd}, {avl}, {}", vtype_text(vtype))
        }
        VecConfigOp::Vsetvl => format!("vsetvl {rd}, {}, {}", xreg(fields.rs1), xreg(fields.rs2)),
    }
}

/// A vtype immediate as `e32, m4, ta, ma`, or in hex when it sets a
/// reserved SEW or LMUL or a bit above `vma`.
fn vtype_text(vtype: u16) -> String {
    let sew = match (vtype >> 3) & 0x7 {
        0 => "e8",
        1 => "e16",
        2 => "e32",
        3 => "e64",
        _ => return format!("{vtype:#x}"),
    };
    let lmul = match vtype & 0x7 {
        0 => "m1",
        1 => "m2",
        2 => "m4",
        3 => "m8",
        5 => "mf8",
        6 => "mf4",
        7 => "mf2",
        _ => return format!("{vtype:#x}"),
    };
    if vtype >> 8 != 0 {
        return format!("{vtype:#x}");
    }
    let ta = if (vtype >> 6) & 1 != 0 { "ta" } else { "tu" };
    let ma = if (vtype >> 7) & 1 != 0 { "ma" } else { "mu" };
    format!("{sew}, {lmul}, {ta}, {ma}")
}

/// A vector load: `vle32.v vd, (rs1)` and its strided, indexed, segment,
/// fault-only-first, mask and whole-register forms.
fn load(op: VecLoadOp, ctrl: &ControlSignals, fields: &Decoded, mask: &str) -> String {
    let (vd, rs1) = (vreg(ctrl.vd), xreg(fields.rs1));
    match op {
        VecLoadOp::Unit => format!("{}.v {vd}, ({rs1}){mask}", memory_name("vl", "", "e", ctrl)),
        VecLoadOp::FaultOnlyFirst => {
            format!("{}ff.v {vd}, ({rs1}){mask}", memory_name("vl", "", "e", ctrl))
        }
        VecLoadOp::Mask => format!("vlm.v {vd}, ({rs1})"),
        VecLoadOp::WholeRegister => {
            format!("vl{}re{}.v {vd}, ({rs1})", ctrl.vec_nf + 1, ctrl.vec_eew.bits())
        }
        VecLoadOp::Strided => format!(
            "{}.v {vd}, ({rs1}), {}{mask}",
            memory_name("vl", "s", "e", ctrl),
            xreg(fields.rs2)
        ),
        VecLoadOp::IndexedOrdered => format!(
            "{}.v {vd}, ({rs1}), {}{mask}",
            memory_name("vl", "ox", "ei", ctrl),
            vreg(ctrl.vs2)
        ),
        VecLoadOp::IndexedUnordered => format!(
            "{}.v {vd}, ({rs1}), {}{mask}",
            memory_name("vl", "ux", "ei", ctrl),
            vreg(ctrl.vs2)
        ),
    }
}

/// A vector store, named as the load it mirrors; vd holds the data, vs3.
fn store(op: VecStoreOp, ctrl: &ControlSignals, fields: &Decoded, mask: &str) -> String {
    let (vs3, rs1) = (vreg(ctrl.vd), xreg(fields.rs1));
    match op {
        VecStoreOp::Unit => format!("{}.v {vs3}, ({rs1}){mask}", memory_name("vs", "", "e", ctrl)),
        VecStoreOp::Mask => format!("vsm.v {vs3}, ({rs1})"),
        VecStoreOp::WholeRegister => format!("vs{}r.v {vs3}, ({rs1})", ctrl.vec_nf + 1),
        VecStoreOp::Strided => format!(
            "{}.v {vs3}, ({rs1}), {}{mask}",
            memory_name("vs", "s", "e", ctrl),
            xreg(fields.rs2)
        ),
        VecStoreOp::IndexedOrdered => format!(
            "{}.v {vs3}, ({rs1}), {}{mask}",
            memory_name("vs", "ox", "ei", ctrl),
            vreg(ctrl.vs2)
        ),
        VecStoreOp::IndexedUnordered => format!(
            "{}.v {vs3}, ({rs1}), {}{mask}",
            memory_name("vs", "ux", "ei", ctrl),
            vreg(ctrl.vs2)
        ),
    }
}

/// `vlsseg3e32`: the direction, the addressing mode, `seg<nf>` for a
/// segment access, then `e` or `ei` and the element width.
fn memory_name(direction: &str, mode: &str, element: &str, ctrl: &ControlSignals) -> String {
    let fields = ctrl.vec_nf + 1;
    let segment = if fields > 1 { format!("seg{fields}") } else { String::new() };
    format!("{direction}{mode}{segment}{element}{}", ctrl.vec_eew.bits())
}

fn alu(op: VecAluOp, operands: &Operands) -> String {
    match op {
        VecAluOp::Int(op) => int_unary_name(op)
            .map_or_else(|| operands.binary(int_name(op), 'v'), |name| operands.unary(name)),
        VecAluOp::Compare(op) => operands.binary(compare_name(op), 'v'),
        VecAluOp::Carry(op) => {
            let name = carry_name(op);
            operands.with_carry_or(name, |operands| operands.binary(name, 'v'))
        }
        VecAluOp::Macc(op) => operands.multiply_add(macc_name(op)),
        VecAluOp::Widen(op) => {
            let (name, first) = widen_name(op);
            operands.binary(name, first)
        }
        VecAluOp::WidenMacc(op) => operands.multiply_add(widen_macc_name(op)),
        VecAluOp::Narrow(op) => operands.binary(narrow_name(op), 'w'),
        VecAluOp::Extend(op) => {
            let kind = if op.signedness == Signedness::Signed { "vsext" } else { "vzext" };
            let factor = match op.factor {
                ExtendFactor::Vf2 => "vf2",
                ExtendFactor::Vf4 => "vf4",
                ExtendFactor::Vf8 => "vf8",
            };
            operands.unary(&format!("{kind}.{factor}"))
        }
        VecAluOp::Merge => operands.with_carry_or("vmerge", |operands| {
            format!("vmv.v.{} {}, {}", operands.letter, operands.vd, operands.source)
        }),
    }
}

const fn int_name(op: IntOp) -> &'static str {
    match op {
        IntOp::Add => "vadd",
        IntOp::Sub => "vsub",
        IntOp::Rsub => "vrsub",
        IntOp::And => "vand",
        IntOp::Or => "vor",
        IntOp::Xor => "vxor",
        IntOp::Sll => "vsll",
        IntOp::Srl => "vsrl",
        IntOp::Sra => "vsra",
        IntOp::MinU => "vminu",
        IntOp::Min => "vmin",
        IntOp::MaxU => "vmaxu",
        IntOp::Max => "vmax",
        IntOp::Mul => "vmul",
        IntOp::Mulh => "vmulh",
        IntOp::Mulhu => "vmulhu",
        IntOp::Mulhsu => "vmulhsu",
        IntOp::DivU => "vdivu",
        IntOp::Div => "vdiv",
        IntOp::RemU => "vremu",
        IntOp::Rem => "vrem",
        IntOp::SAddU => "vsaddu",
        IntOp::SAdd => "vsadd",
        IntOp::SSubU => "vssubu",
        IntOp::SSub => "vssub",
        IntOp::AAddU => "vaaddu",
        IntOp::AAdd => "vaadd",
        IntOp::ASubU => "vasubu",
        IntOp::ASub => "vasub",
        IntOp::Smul => "vsmul",
        IntOp::SSrl => "vssrl",
        IntOp::SSra => "vssra",
        IntOp::AndN => "vandn",
        IntOp::Rol => "vrol",
        IntOp::Ror => "vror",
        IntOp::ClMul => "vclmul",
        IntOp::ClMulH => "vclmulh",
        IntOp::Brev => "vbrev",
        IntOp::Brev8 => "vbrev8",
        IntOp::Rev8 => "vrev8",
        IntOp::Clz => "vclz",
        IntOp::Ctz => "vctz",
        IntOp::CpopV => "vcpop",
    }
}

/// The Zvbb ops with one source, named with their `.v` suffix.
const fn int_unary_name(op: IntOp) -> Option<&'static str> {
    Some(match op {
        IntOp::Brev => "vbrev.v",
        IntOp::Brev8 => "vbrev8.v",
        IntOp::Rev8 => "vrev8.v",
        IntOp::Clz => "vclz.v",
        IntOp::Ctz => "vctz.v",
        IntOp::CpopV => "vcpop.v",
        _ => return None,
    })
}

const fn compare_name(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Eq => "vmseq",
        CompareOp::Ne => "vmsne",
        CompareOp::LtU => "vmsltu",
        CompareOp::Lt => "vmslt",
        CompareOp::LeU => "vmsleu",
        CompareOp::Le => "vmsle",
        CompareOp::GtU => "vmsgtu",
        CompareOp::Gt => "vmsgt",
    }
}

const fn carry_name(op: CarryOp) -> &'static str {
    match op {
        CarryOp::Adc => "vadc",
        CarryOp::Madc => "vmadc",
        CarryOp::Sbc => "vsbc",
        CarryOp::Msbc => "vmsbc",
    }
}

const fn macc_name(op: MaccOp) -> &'static str {
    match op {
        MaccOp::Macc => "vmacc",
        MaccOp::NMSac => "vnmsac",
        MaccOp::Madd => "vmadd",
        MaccOp::NMSub => "vnmsub",
    }
}

/// The name and the first operand's kind: `w` for the forms whose vs2 is
/// already wide.
const fn widen_name(op: WidenOp) -> (&'static str, char) {
    match op {
        WidenOp::AddU => ("vwaddu", 'v'),
        WidenOp::Add => ("vwadd", 'v'),
        WidenOp::SubU => ("vwsubu", 'v'),
        WidenOp::Sub => ("vwsub", 'v'),
        WidenOp::AddUW => ("vwaddu", 'w'),
        WidenOp::AddW => ("vwadd", 'w'),
        WidenOp::SubUW => ("vwsubu", 'w'),
        WidenOp::SubW => ("vwsub", 'w'),
        WidenOp::MulU => ("vwmulu", 'v'),
        WidenOp::Mul => ("vwmul", 'v'),
        WidenOp::MulSU => ("vwmulsu", 'v'),
        WidenOp::Sll => ("vwsll", 'v'),
    }
}

const fn widen_macc_name(op: WidenMaccOp) -> &'static str {
    match op {
        WidenMaccOp::MaccU => "vwmaccu",
        WidenMaccOp::Macc => "vwmacc",
        WidenMaccOp::MaccSU => "vwmaccsu",
        WidenMaccOp::MaccUS => "vwmaccus",
    }
}

const fn narrow_name(op: NarrowOp) -> &'static str {
    match op {
        NarrowOp::Srl => "vnsrl",
        NarrowOp::Sra => "vnsra",
        NarrowOp::ClipU => "vnclipu",
        NarrowOp::Clip => "vnclip",
    }
}

fn floating_point(
    op: VecFpOp,
    ctrl: &ControlSignals,
    fields: &Decoded,
    operands: &Operands,
) -> String {
    match op {
        VecFpOp::Binary(op) => operands.binary(fp_binary_name(op), 'v'),
        VecFpOp::Unary(op) => operands.unary(&fp_unary_name(op)),
        VecFpOp::Fma(signs, addend) => operands.multiply_add(fma_name(signs, addend)),
        VecFpOp::Compare(op) => operands.binary(fp_compare_name(op), 'v'),
        VecFpOp::Widen(op) => {
            let (name, first) = fp_widen_name(op);
            operands.binary(name, first)
        }
        VecFpOp::WidenFma(signs) => operands.multiply_add(widen_fma_name(signs)),
        VecFpOp::WidenConvert(FpWidenConvertOp::Int(convert)) => {
            operands.unary(&format!("vfwcvt.{}.v", convert_name(convert)))
        }
        VecFpOp::WidenConvert(FpWidenConvertOp::Float) => operands.unary("vfwcvt.f.f.v"),
        VecFpOp::NarrowConvert(FpNarrowConvertOp::Int(convert)) => {
            operands.unary(&format!("vfncvt.{}.w", convert_name(convert)))
        }
        VecFpOp::NarrowConvert(FpNarrowConvertOp::Float) => operands.unary("vfncvt.f.f.w"),
        VecFpOp::NarrowConvert(FpNarrowConvertOp::FloatRoundToOdd) => {
            operands.unary("vfncvt.rod.f.f.w")
        }
        VecFpOp::Merge => operands.with_carry_or("vfmerge", |operands| {
            format!("vfmv.v.f {}, {}", operands.vd, operands.source)
        }),
        VecFpOp::MoveToScalar => format!("vfmv.f.s {}, {}", freg(fields.rd), vreg(ctrl.vs2)),
        VecFpOp::MoveFromScalar => format!("vfmv.s.f {}, {}", operands.vd, operands.source),
        VecFpOp::Slide1Up => operands.binary("vfslide1up", 'v'),
        VecFpOp::Slide1Down => operands.binary("vfslide1down", 'v'),
    }
}

const fn fp_binary_name(op: FpBinaryOp) -> &'static str {
    match op {
        FpBinaryOp::Add => "vfadd",
        FpBinaryOp::Sub => "vfsub",
        FpBinaryOp::RSub => "vfrsub",
        FpBinaryOp::Mul => "vfmul",
        FpBinaryOp::Div => "vfdiv",
        FpBinaryOp::RDiv => "vfrdiv",
        FpBinaryOp::Min => "vfmin",
        FpBinaryOp::Max => "vfmax",
        FpBinaryOp::Sgnj => "vfsgnj",
        FpBinaryOp::Sgnjn => "vfsgnjn",
        FpBinaryOp::Sgnjx => "vfsgnjx",
    }
}

fn fp_unary_name(op: FpUnaryOp) -> String {
    match op {
        FpUnaryOp::Sqrt => "vfsqrt.v".to_owned(),
        FpUnaryOp::Rsqrt7 => "vfrsqrt7.v".to_owned(),
        FpUnaryOp::Rec7 => "vfrec7.v".to_owned(),
        FpUnaryOp::Class => "vfclass.v".to_owned(),
        FpUnaryOp::Convert(convert) => format!("vfcvt.{}.v", convert_name(convert)),
    }
}

/// The middle of a conversion's name: `xu.f`, `rtz.x.f`, `f.xu` and so on.
const fn convert_name(convert: IntConvert) -> &'static str {
    match convert {
        IntConvert::ToInt(Signedness::Unsigned, IntRounding::Dynamic) => "xu.f",
        IntConvert::ToInt(Signedness::Signed, IntRounding::Dynamic) => "x.f",
        IntConvert::ToInt(Signedness::Unsigned, IntRounding::TowardZero) => "rtz.xu.f",
        IntConvert::ToInt(Signedness::Signed, IntRounding::TowardZero) => "rtz.x.f",
        IntConvert::FromInt(Signedness::Unsigned) => "f.xu",
        IntConvert::FromInt(Signedness::Signed) => "f.x",
    }
}

/// `vfmacc` and the like when vd is the addend, `vfmadd` and the like when
/// vs2 is.
const fn fma_name(signs: FmaSigns, addend: FmaAddend) -> &'static str {
    match (addend, signs) {
        (FmaAddend::Vd, FmaSigns::Macc) => "vfmacc",
        (FmaAddend::Vd, FmaSigns::NMacc) => "vfnmacc",
        (FmaAddend::Vd, FmaSigns::MSac) => "vfmsac",
        (FmaAddend::Vd, FmaSigns::NMSac) => "vfnmsac",
        (FmaAddend::Vs2, FmaSigns::Macc) => "vfmadd",
        (FmaAddend::Vs2, FmaSigns::NMacc) => "vfnmadd",
        (FmaAddend::Vs2, FmaSigns::MSac) => "vfmsub",
        (FmaAddend::Vs2, FmaSigns::NMSac) => "vfnmsub",
    }
}

const fn widen_fma_name(signs: FmaSigns) -> &'static str {
    match signs {
        FmaSigns::Macc => "vfwmacc",
        FmaSigns::NMacc => "vfwnmacc",
        FmaSigns::MSac => "vfwmsac",
        FmaSigns::NMSac => "vfwnmsac",
    }
}

const fn fp_compare_name(op: FpCompareOp) -> &'static str {
    match op {
        FpCompareOp::Eq => "vmfeq",
        FpCompareOp::Ne => "vmfne",
        FpCompareOp::Lt => "vmflt",
        FpCompareOp::Le => "vmfle",
        FpCompareOp::Gt => "vmfgt",
        FpCompareOp::Ge => "vmfge",
    }
}

/// The name and the first operand's kind, as [`widen_name`] gives them.
const fn fp_widen_name(op: FpWidenOp) -> (&'static str, char) {
    match op {
        FpWidenOp::Add => ("vfwadd", 'v'),
        FpWidenOp::Sub => ("vfwsub", 'v'),
        FpWidenOp::Mul => ("vfwmul", 'v'),
        FpWidenOp::AddW => ("vfwadd", 'w'),
        FpWidenOp::SubW => ("vfwsub", 'w'),
    }
}

const fn reduce_name(op: ReduceOp) -> &'static str {
    match op {
        ReduceOp::Int(op) => match op {
            IntReduceOp::Sum => "vredsum",
            IntReduceOp::And => "vredand",
            IntReduceOp::Or => "vredor",
            IntReduceOp::Xor => "vredxor",
            IntReduceOp::MinU => "vredminu",
            IntReduceOp::Min => "vredmin",
            IntReduceOp::MaxU => "vredmaxu",
            IntReduceOp::Max => "vredmax",
        },
        ReduceOp::WidenInt(WidenIntReduceOp::SumU) => "vwredsumu",
        ReduceOp::WidenInt(WidenIntReduceOp::Sum) => "vwredsum",
        ReduceOp::Fp(op) => match op {
            FpReduceOp::OSum => "vfredosum",
            FpReduceOp::USum => "vfredusum",
            FpReduceOp::Max => "vfredmax",
            FpReduceOp::Min => "vfredmin",
        },
        ReduceOp::FpWiden(FpWidenReduceOp::OSum) => "vfwredosum",
        ReduceOp::FpWiden(FpWidenReduceOp::USum) => "vfwredusum",
    }
}

fn mask(op: MaskOp, ctrl: &ControlSignals, fields: &Decoded, operands: &Operands) -> String {
    let Operands { vd, vs2, mask, .. } = operands;
    match op {
        MaskOp::Logical(op) => {
            format!("{}.mm {vd}, {vs2}, {}", mask_logical_name(op), vreg(ctrl.vs1))
        }
        MaskOp::CPop => format!("vcpop.m {}, {vs2}{mask}", xreg(fields.rd)),
        MaskOp::First => format!("vfirst.m {}, {vs2}{mask}", xreg(fields.rd)),
        MaskOp::Set(MaskSetOp::BeforeFirst) => operands.unary("vmsbf.m"),
        MaskOp::Set(MaskSetOp::IncludingFirst) => operands.unary("vmsif.m"),
        MaskOp::Set(MaskSetOp::OnlyFirst) => operands.unary("vmsof.m"),
        MaskOp::Iota => operands.unary("viota.m"),
        MaskOp::Id => format!("vid.v {vd}{mask}"),
    }
}

const fn mask_logical_name(op: MaskLogicalOp) -> &'static str {
    match op {
        MaskLogicalOp::And => "vmand",
        MaskLogicalOp::Nand => "vmnand",
        MaskLogicalOp::AndNot => "vmandn",
        MaskLogicalOp::Or => "vmor",
        MaskLogicalOp::Nor => "vmnor",
        MaskLogicalOp::OrNot => "vmorn",
        MaskLogicalOp::Xor => "vmxor",
        MaskLogicalOp::Xnor => "vmxnor",
    }
}

fn permute(op: PermuteOp, ctrl: &ControlSignals, fields: &Decoded, operands: &Operands) -> String {
    let Operands { vd, vs2, source, .. } = operands;
    match op {
        PermuteOp::MvXS => format!("vmv.x.s {}, {vs2}", xreg(fields.rd)),
        PermuteOp::MvSX => format!("vmv.s.x {vd}, {source}"),
        PermuteOp::SlideUp(_) => operands.binary("vslideup", 'v'),
        PermuteOp::SlideDown(_) => operands.binary("vslidedown", 'v'),
        PermuteOp::Slide1Up => operands.binary("vslide1up", 'v'),
        PermuteOp::Slide1Down => operands.binary("vslide1down", 'v'),
        PermuteOp::Rgather => operands.binary("vrgather", 'v'),
        PermuteOp::RgatherEi16 => operands.binary("vrgatherei16", 'v'),
        PermuteOp::Compress => format!("vcompress.vm {vd}, {vs2}, {}", vreg(ctrl.vs1)),
        PermuteOp::WholeMove(registers) => format!("vmv{}r.v {vd}, {vs2}", registers.count()),
    }
}

/// The vector crypto ops. The AES and SM4 rounds name their key group's
/// form, `.vv` or `.vs`; the key schedules take vs1's field as `uimm`.
fn crypto(op: CryptoOp, ctrl: &ControlSignals, operands: &Operands) -> String {
    let Operands { vd, vs2, source, .. } = operands;
    let key_form = if ctrl.vec_broadcast_vs2 { "vs" } else { "vv" };
    let uimm = ctrl.vs1.as_u8();
    match op {
        CryptoOp::AesEm => format!("vaesem.{key_form} {vd}, {vs2}"),
        CryptoOp::AesEf => format!("vaesef.{key_form} {vd}, {vs2}"),
        CryptoOp::AesDm => format!("vaesdm.{key_form} {vd}, {vs2}"),
        CryptoOp::AesDf => format!("vaesdf.{key_form} {vd}, {vs2}"),
        CryptoOp::Sm4R => format!("vsm4r.{key_form} {vd}, {vs2}"),
        CryptoOp::AesZ => format!("vaesz.vs {vd}, {vs2}"),
        CryptoOp::Gmul => format!("vgmul.vv {vd}, {vs2}"),
        CryptoOp::Sha2Ms => format!("vsha2ms.vv {vd}, {vs2}, {source}"),
        CryptoOp::Sha2Ch => format!("vsha2ch.vv {vd}, {vs2}, {source}"),
        CryptoOp::Sha2Cl => format!("vsha2cl.vv {vd}, {vs2}, {source}"),
        CryptoOp::Sm3Me => format!("vsm3me.vv {vd}, {vs2}, {source}"),
        CryptoOp::Ghsh => format!("vghsh.vv {vd}, {vs2}, {source}"),
        CryptoOp::AesKf1 => format!("vaeskf1.vi {vd}, {vs2}, {uimm}"),
        CryptoOp::AesKf2 => format!("vaeskf2.vi {vd}, {vs2}, {uimm}"),
        CryptoOp::Sm3C => format!("vsm3c.vi {vd}, {vs2}, {uimm}"),
        CryptoOp::Sm4K => format!("vsm4k.vi {vd}, {vs2}, {uimm}"),
    }
}
