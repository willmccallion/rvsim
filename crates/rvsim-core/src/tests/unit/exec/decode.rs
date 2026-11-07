//! Scalar decode checked against GNU as: each row is the word the assembler
//! produced for an instruction and what that instruction means, so the
//! table does not rely on rvsim's own encoders.

use crate::exec::decode::{DecodedInst, decode_inst};
use crate::exec::signals::ControlFlow;
use crate::isa::fp::{FpFormat, Rounding, RoundingMode};
use crate::isa::op::{AluOp, AtomicOp, CsrOp, IntWidth, MemWidth, OperandFormat, SystemOp};
use crate::isa::privileged::Trap;
use crate::isa::reg::{ArchReg, RegIdx};

/// Operand format: 64-bit (`X`) or word (`W`) integer, or single, double
/// or half precision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    X,
    W,
    S,
    D,
    H,
}
use Format::{D, H, S, W, X};

/// The register file a result goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dest {
    Int,
    Fp,
}
use Dest::{Fp, Int};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Extension {
    Signed,
    Unsigned,
}
use Extension::{Signed, Unsigned};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ordering {
    None,
    Acquire,
    Release,
    AcquireRelease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Compute(AluOp, Format, Dest);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    Load(MemWidth, Extension, Dest),
    Store(MemWidth, Dest),
}
use Access::{Load, Store};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Atomic(AtomicOp, MemWidth, Ordering);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum System {
    Plain(SystemOp),
    Csr(CsrOp),
}
use System::{Csr, Plain};

const PC: u64 = 0x8000_0000;

fn decode(word: u32) -> DecodedInst {
    decode_inst(word, PC, 0)
}

fn format_of(decoded: &DecodedInst) -> Format {
    match decoded.ctrl.format {
        OperandFormat::Int(IntWidth::Word) => W,
        OperandFormat::Int(IntWidth::Double) => X,
        OperandFormat::Fp(FpFormat::Single) => S,
        OperandFormat::Fp(FpFormat::Double) => D,
        OperandFormat::Fp(FpFormat::Half) => H,
    }
}

fn dest_of(decoded: &DecodedInst) -> Option<Dest> {
    match (decoded.ctrl.reg_write, decoded.ctrl.fp_reg_write) {
        (true, false) => Some(Int),
        (false, true) => Some(Fp),
        _ => None,
    }
}

/// Checks every row with `meaning`, which describes what `decode` produced
/// in the row's terms, and fails listing every row that differs.
fn check_rows<T: PartialEq + std::fmt::Debug>(
    rows: &[(&str, u32, T)],
    meaning: impl Fn(&DecodedInst, &T) -> T,
) {
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|(asm, word, expected)| {
            let decoded = decode(*word);
            if let Some(trap) = &decoded.trap {
                return Some(format!("{asm} ({word:#010x}): trapped with {trap:?}"));
            }
            let actual = meaning(&decoded, expected);
            (actual != *expected)
                .then(|| format!("{asm} ({word:#010x}): expected {expected:?}, got {actual:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{} rows decode wrongly:\n{}", wrong.len(), wrong.join("\n"));
}

#[rustfmt::skip]
const COMPUTE: &[(&str, u32, Compute)] = &[
    ("addi a0, a1, 5", 0x00558513, Compute(AluOp::Add, X, Int)),
    ("slti a0, a1, 5", 0x0055a513, Compute(AluOp::Slt, X, Int)),
    ("sltiu a0, a1, 5", 0x0055b513, Compute(AluOp::Sltu, X, Int)),
    ("xori a0, a1, 5", 0x0055c513, Compute(AluOp::Xor, X, Int)),
    ("ori a0, a1, 5", 0x0055e513, Compute(AluOp::Or, X, Int)),
    ("andi a0, a1, 5", 0x0055f513, Compute(AluOp::And, X, Int)),
    ("slli a0, a1, 37", 0x02559513, Compute(AluOp::Sll, X, Int)),
    ("srli a0, a1, 37", 0x0255d513, Compute(AluOp::Srl, X, Int)),
    ("srai a0, a1, 37", 0x4255d513, Compute(AluOp::Sra, X, Int)),
    ("addiw a0, a1, 5", 0x0055851b, Compute(AluOp::Add, W, Int)),
    ("slliw a0, a1, 5", 0x0055951b, Compute(AluOp::Sll, W, Int)),
    ("srliw a0, a1, 5", 0x0055d51b, Compute(AluOp::Srl, W, Int)),
    ("sraiw a0, a1, 5", 0x4055d51b, Compute(AluOp::Sra, W, Int)),
    ("add a0, a1, a2", 0x00c58533, Compute(AluOp::Add, X, Int)),
    ("sub a0, a1, a2", 0x40c58533, Compute(AluOp::Sub, X, Int)),
    ("sll a0, a1, a2", 0x00c59533, Compute(AluOp::Sll, X, Int)),
    ("slt a0, a1, a2", 0x00c5a533, Compute(AluOp::Slt, X, Int)),
    ("sltu a0, a1, a2", 0x00c5b533, Compute(AluOp::Sltu, X, Int)),
    ("xor a0, a1, a2", 0x00c5c533, Compute(AluOp::Xor, X, Int)),
    ("srl a0, a1, a2", 0x00c5d533, Compute(AluOp::Srl, X, Int)),
    ("sra a0, a1, a2", 0x40c5d533, Compute(AluOp::Sra, X, Int)),
    ("or a0, a1, a2", 0x00c5e533, Compute(AluOp::Or, X, Int)),
    ("and a0, a1, a2", 0x00c5f533, Compute(AluOp::And, X, Int)),
    ("addw a0, a1, a2", 0x00c5853b, Compute(AluOp::Add, W, Int)),
    ("subw a0, a1, a2", 0x40c5853b, Compute(AluOp::Sub, W, Int)),
    ("sllw a0, a1, a2", 0x00c5953b, Compute(AluOp::Sll, W, Int)),
    ("srlw a0, a1, a2", 0x00c5d53b, Compute(AluOp::Srl, W, Int)),
    ("sraw a0, a1, a2", 0x40c5d53b, Compute(AluOp::Sra, W, Int)),
    ("mul a0, a1, a2", 0x02c58533, Compute(AluOp::Mul, X, Int)),
    ("mulh a0, a1, a2", 0x02c59533, Compute(AluOp::Mulh, X, Int)),
    ("mulhsu a0, a1, a2", 0x02c5a533, Compute(AluOp::Mulhsu, X, Int)),
    ("mulhu a0, a1, a2", 0x02c5b533, Compute(AluOp::Mulhu, X, Int)),
    ("div a0, a1, a2", 0x02c5c533, Compute(AluOp::Div, X, Int)),
    ("divu a0, a1, a2", 0x02c5d533, Compute(AluOp::Divu, X, Int)),
    ("rem a0, a1, a2", 0x02c5e533, Compute(AluOp::Rem, X, Int)),
    ("remu a0, a1, a2", 0x02c5f533, Compute(AluOp::Remu, X, Int)),
    ("mulw a0, a1, a2", 0x02c5853b, Compute(AluOp::Mul, W, Int)),
    ("divw a0, a1, a2", 0x02c5c53b, Compute(AluOp::Div, W, Int)),
    ("divuw a0, a1, a2", 0x02c5d53b, Compute(AluOp::Divu, W, Int)),
    ("remw a0, a1, a2", 0x02c5e53b, Compute(AluOp::Rem, W, Int)),
    ("remuw a0, a1, a2", 0x02c5f53b, Compute(AluOp::Remu, W, Int)),
    ("sh1add a0, a1, a2", 0x20c5a533, Compute(AluOp::Sh1Add, X, Int)),
    ("sh2add a0, a1, a2", 0x20c5c533, Compute(AluOp::Sh2Add, X, Int)),
    ("sh3add a0, a1, a2", 0x20c5e533, Compute(AluOp::Sh3Add, X, Int)),
    ("add.uw a0, a1, a2", 0x08c5853b, Compute(AluOp::AddUw, X, Int)),
    ("sh1add.uw a0, a1, a2", 0x20c5a53b, Compute(AluOp::Sh1AddUw, X, Int)),
    ("sh2add.uw a0, a1, a2", 0x20c5c53b, Compute(AluOp::Sh2AddUw, X, Int)),
    ("sh3add.uw a0, a1, a2", 0x20c5e53b, Compute(AluOp::Sh3AddUw, X, Int)),
    ("slli.uw a0, a1, 5", 0x0855951b, Compute(AluOp::SlliUw, X, Int)),
    ("andn a0, a1, a2", 0x40c5f533, Compute(AluOp::Andn, X, Int)),
    ("orn a0, a1, a2", 0x40c5e533, Compute(AluOp::Orn, X, Int)),
    ("xnor a0, a1, a2", 0x40c5c533, Compute(AluOp::Xnor, X, Int)),
    ("clz a0, a1", 0x60059513, Compute(AluOp::Clz, X, Int)),
    ("ctz a0, a1", 0x60159513, Compute(AluOp::Ctz, X, Int)),
    ("cpop a0, a1", 0x60259513, Compute(AluOp::Cpop, X, Int)),
    ("clzw a0, a1", 0x6005951b, Compute(AluOp::Clz, W, Int)),
    ("ctzw a0, a1", 0x6015951b, Compute(AluOp::Ctz, W, Int)),
    ("cpopw a0, a1", 0x6025951b, Compute(AluOp::Cpop, W, Int)),
    ("max a0, a1, a2", 0x0ac5e533, Compute(AluOp::Max, X, Int)),
    ("maxu a0, a1, a2", 0x0ac5f533, Compute(AluOp::Maxu, X, Int)),
    ("min a0, a1, a2", 0x0ac5c533, Compute(AluOp::Min, X, Int)),
    ("minu a0, a1, a2", 0x0ac5d533, Compute(AluOp::Minu, X, Int)),
    ("sext.b a0, a1", 0x60459513, Compute(AluOp::SextB, X, Int)),
    ("sext.h a0, a1", 0x60559513, Compute(AluOp::SextH, X, Int)),
    ("rol a0, a1, a2", 0x60c59533, Compute(AluOp::Rol, X, Int)),
    ("ror a0, a1, a2", 0x60c5d533, Compute(AluOp::Ror, X, Int)),
    ("rori a0, a1, 5", 0x6055d513, Compute(AluOp::Ror, X, Int)),
    ("rolw a0, a1, a2", 0x60c5953b, Compute(AluOp::Rol, W, Int)),
    ("rorw a0, a1, a2", 0x60c5d53b, Compute(AluOp::Ror, W, Int)),
    ("roriw a0, a1, 5", 0x6055d51b, Compute(AluOp::Ror, W, Int)),
    ("orc.b a0, a1", 0x2875d513, Compute(AluOp::OrcB, X, Int)),
    ("rev8 a0, a1", 0x6b85d513, Compute(AluOp::Rev8, X, Int)),
    ("clmul a0, a1, a2", 0x0ac59533, Compute(AluOp::Clmul, X, Int)),
    ("clmulh a0, a1, a2", 0x0ac5b533, Compute(AluOp::Clmulh, X, Int)),
    ("clmulr a0, a1, a2", 0x0ac5a533, Compute(AluOp::Clmulr, X, Int)),
    ("bclr a0, a1, a2", 0x48c59533, Compute(AluOp::Bclr, X, Int)),
    ("bclri a0, a1, 5", 0x48559513, Compute(AluOp::Bclr, X, Int)),
    ("bext a0, a1, a2", 0x48c5d533, Compute(AluOp::Bext, X, Int)),
    ("bexti a0, a1, 5", 0x4855d513, Compute(AluOp::Bext, X, Int)),
    ("binv a0, a1, a2", 0x68c59533, Compute(AluOp::Binv, X, Int)),
    ("binvi a0, a1, 5", 0x68559513, Compute(AluOp::Binv, X, Int)),
    ("bset a0, a1, a2", 0x28c59533, Compute(AluOp::Bset, X, Int)),
    ("bseti a0, a1, 5", 0x28559513, Compute(AluOp::Bset, X, Int)),
    ("pack a0, a1, a2", 0x08c5c533, Compute(AluOp::Pack, X, Int)),
    ("packh a0, a1, a2", 0x08c5f533, Compute(AluOp::Packh, X, Int)),
    ("packw a0, a1, a2", 0x08c5c53b, Compute(AluOp::Packw, W, Int)),
    ("brev8 a0, a1", 0x6875d513, Compute(AluOp::Brev8, X, Int)),
    ("xperm4 a0, a1, a2", 0x28c5a533, Compute(AluOp::Xperm4, X, Int)),
    ("xperm8 a0, a1, a2", 0x28c5c533, Compute(AluOp::Xperm8, X, Int)),
    ("fadd.s fa0, fa1, fa2, rne", 0x00c58553, Compute(AluOp::FAdd, S, Fp)),
    ("fsub.d fa0, fa1, fa2, rtz", 0x0ac59553, Compute(AluOp::FSub, D, Fp)),
    ("fmul.h fa0, fa1, fa2, dyn", 0x14c5f553, Compute(AluOp::FMul, H, Fp)),
    ("fdiv.s fa0, fa1, fa2", 0x18c5f553, Compute(AluOp::FDiv, S, Fp)),
    ("fsqrt.d fa0, fa1", 0x5a05f553, Compute(AluOp::FSqrt, D, Fp)),
    ("fsgnj.s fa0, fa1, fa2", 0x20c58553, Compute(AluOp::FSgnJ, S, Fp)),
    ("fsgnjn.d fa0, fa1, fa2", 0x22c59553, Compute(AluOp::FSgnJN, D, Fp)),
    ("fsgnjx.h fa0, fa1, fa2", 0x24c5a553, Compute(AluOp::FSgnJX, H, Fp)),
    ("fmin.s fa0, fa1, fa2", 0x28c58553, Compute(AluOp::FMin, S, Fp)),
    ("fmax.d fa0, fa1, fa2", 0x2ac59553, Compute(AluOp::FMax, D, Fp)),
    ("feq.s a0, fa1, fa2", 0xa0c5a553, Compute(AluOp::FEq, S, Int)),
    ("flt.d a0, fa1, fa2", 0xa2c59553, Compute(AluOp::FLt, D, Int)),
    ("fle.h a0, fa1, fa2", 0xa4c58553, Compute(AluOp::FLe, H, Int)),
    ("fmv.x.w a0, fa1", 0xe0058553, Compute(AluOp::FMvToX, S, Int)),
    ("fmv.x.d a0, fa1", 0xe2058553, Compute(AluOp::FMvToX, D, Int)),
    ("fmv.x.h a0, fa1", 0xe4058553, Compute(AluOp::FMvToX, H, Int)),
    ("fclass.s a0, fa1", 0xe0059553, Compute(AluOp::FClass, S, Int)),
    ("fclass.d a0, fa1", 0xe2059553, Compute(AluOp::FClass, D, Int)),
    ("fmv.w.x fa0, a1", 0xf0058553, Compute(AluOp::FMvToF, S, Fp)),
    ("fmv.d.x fa0, a1", 0xf2058553, Compute(AluOp::FMvToF, D, Fp)),
    ("fmv.h.x fa0, a1", 0xf4058553, Compute(AluOp::FMvToF, H, Fp)),
    ("fcvt.w.s a0, fa1", 0xc005f553, Compute(AluOp::FCvtWS, S, Int)),
    ("fcvt.wu.d a0, fa1", 0xc215f553, Compute(AluOp::FCvtWUS, D, Int)),
    ("fcvt.l.h a0, fa1", 0xc425f553, Compute(AluOp::FCvtLS, H, Int)),
    ("fcvt.lu.s a0, fa1", 0xc035f553, Compute(AluOp::FCvtLUS, S, Int)),
    ("fcvt.s.w fa0, a1", 0xd005f553, Compute(AluOp::FCvtSW, S, Fp)),
    ("fcvt.d.wu fa0, a1", 0xd2158553, Compute(AluOp::FCvtSWU, D, Fp)),
    ("fcvt.h.l fa0, a1", 0xd425f553, Compute(AluOp::FCvtSL, H, Fp)),
    ("fcvt.s.lu fa0, a1", 0xd035f553, Compute(AluOp::FCvtSLU, S, Fp)),
    ("fcvt.d.s fa0, fa1", 0x42058553, Compute(AluOp::FCvtDS, D, Fp)),
    ("fcvt.d.h fa0, fa1", 0x42258553, Compute(AluOp::FCvtDH, D, Fp)),
    ("fcvt.s.d fa0, fa1", 0x4015f553, Compute(AluOp::FCvtSD, S, Fp)),
    ("fcvt.s.h fa0, fa1", 0x40258553, Compute(AluOp::FCvtSH, S, Fp)),
    ("fcvt.h.s fa0, fa1", 0x4405f553, Compute(AluOp::FCvtHS, H, Fp)),
    ("fcvt.h.d fa0, fa1", 0x4415f553, Compute(AluOp::FCvtHD, H, Fp)),
    ("fmadd.s fa0, fa1, fa2, fa3", 0x68c5f543, Compute(AluOp::FMAdd, S, Fp)),
    ("fmsub.d fa0, fa1, fa2, fa3", 0x6ac5f547, Compute(AluOp::FMSub, D, Fp)),
    ("fnmadd.h fa0, fa1, fa2, fa3", 0x6cc5f54f, Compute(AluOp::FNMAdd, H, Fp)),
    ("fnmsub.s fa0, fa1, fa2, fa3", 0x68c5f54b, Compute(AluOp::FNMSub, S, Fp)),
];

#[test]
fn each_compute_instruction_decodes_to_its_operation_format_and_destination() {
    check_rows(COMPUTE, |decoded, _| {
        let dest = dest_of(decoded).unwrap_or_else(|| panic!("{decoded:?} writes no register"));
        Compute(decoded.ctrl.alu, format_of(decoded), dest)
    });
}

#[rustfmt::skip]
const MEMORY: &[(&str, u32, Access)] = &[
    ("lb a0, 8(a1)", 0x00858503, Load(MemWidth::Byte, Signed, Int)),
    ("lh a0, 8(a1)", 0x00859503, Load(MemWidth::Half, Signed, Int)),
    ("lw a0, 8(a1)", 0x0085a503, Load(MemWidth::Word, Signed, Int)),
    ("ld a0, 8(a1)", 0x0085b503, Load(MemWidth::Double, Signed, Int)),
    ("lbu a0, 8(a1)", 0x0085c503, Load(MemWidth::Byte, Unsigned, Int)),
    ("lhu a0, 8(a1)", 0x0085d503, Load(MemWidth::Half, Unsigned, Int)),
    ("lwu a0, 8(a1)", 0x0085e503, Load(MemWidth::Word, Unsigned, Int)),
    ("sb a2, 8(a1)", 0x00c58423, Store(MemWidth::Byte, Int)),
    ("sh a2, 8(a1)", 0x00c59423, Store(MemWidth::Half, Int)),
    ("sw a2, 8(a1)", 0x00c5a423, Store(MemWidth::Word, Int)),
    ("sd a2, 8(a1)", 0x00c5b423, Store(MemWidth::Double, Int)),
    ("flh fa0, 8(a1)", 0x00859507, Load(MemWidth::Half, Unsigned, Fp)),
    ("flw fa0, 8(a1)", 0x0085a507, Load(MemWidth::Word, Unsigned, Fp)),
    ("fld fa0, 8(a1)", 0x0085b507, Load(MemWidth::Double, Unsigned, Fp)),
    ("fsh fa2, 8(a1)", 0x00c59427, Store(MemWidth::Half, Fp)),
    ("fsw fa2, 8(a1)", 0x00c5a427, Store(MemWidth::Word, Fp)),
    ("fsd fa2, 8(a1)", 0x00c5b427, Store(MemWidth::Double, Fp)),
];

#[test]
fn each_load_and_store_decodes_to_its_width_extension_and_register_file() {
    check_rows(MEMORY, |decoded, _| {
        let ctrl = &decoded.ctrl;
        match (ctrl.mem_read, ctrl.mem_write) {
            (true, false) => {
                let extension = if ctrl.signed_load { Signed } else { Unsigned };
                Load(ctrl.width, extension, dest_of(decoded).unwrap_or(Int))
            }
            (false, true) => Store(ctrl.width, if ctrl.rs2_fp { Fp } else { Int }),
            access => panic!("{decoded:?} has memory access {access:?}"),
        }
    });
}

#[rustfmt::skip]
const ATOMIC: &[(&str, u32, Atomic)] = &[
    ("lr.w a0, (a1)", 0x1005a52f, Atomic(AtomicOp::Lr, MemWidth::Word, Ordering::None)),
    ("sc.w a0, a2, (a1)", 0x18c5a52f, Atomic(AtomicOp::Sc, MemWidth::Word, Ordering::None)),
    ("amoswap.w a0, a2, (a1)", 0x08c5a52f, Atomic(AtomicOp::Swap, MemWidth::Word, Ordering::None)),
    ("amoadd.w a0, a2, (a1)", 0x00c5a52f, Atomic(AtomicOp::Add, MemWidth::Word, Ordering::None)),
    ("amoxor.w a0, a2, (a1)", 0x20c5a52f, Atomic(AtomicOp::Xor, MemWidth::Word, Ordering::None)),
    ("amoand.w a0, a2, (a1)", 0x60c5a52f, Atomic(AtomicOp::And, MemWidth::Word, Ordering::None)),
    ("amoor.w a0, a2, (a1)", 0x40c5a52f, Atomic(AtomicOp::Or, MemWidth::Word, Ordering::None)),
    ("amomin.w a0, a2, (a1)", 0x80c5a52f, Atomic(AtomicOp::Min, MemWidth::Word, Ordering::None)),
    ("amomax.w a0, a2, (a1)", 0xa0c5a52f, Atomic(AtomicOp::Max, MemWidth::Word, Ordering::None)),
    ("amominu.w a0, a2, (a1)", 0xc0c5a52f, Atomic(AtomicOp::MinU, MemWidth::Word, Ordering::None)),
    ("amomaxu.w a0, a2, (a1)", 0xe0c5a52f, Atomic(AtomicOp::MaxU, MemWidth::Word, Ordering::None)),
    ("lr.d.aqrl a0, (a1)", 0x1605b52f, Atomic(AtomicOp::Lr, MemWidth::Double, Ordering::AcquireRelease)),
    ("sc.d a0, a2, (a1)", 0x18c5b52f, Atomic(AtomicOp::Sc, MemWidth::Double, Ordering::None)),
    ("amoadd.d.aq a0, a2, (a1)", 0x04c5b52f, Atomic(AtomicOp::Add, MemWidth::Double, Ordering::Acquire)),
    ("amoswap.d.rl a0, a2, (a1)", 0x0ac5b52f, Atomic(AtomicOp::Swap, MemWidth::Double, Ordering::Release)),
];

#[test]
fn each_atomic_decodes_to_its_operation_width_and_ordering() {
    check_rows(ATOMIC, |decoded, _| {
        let ctrl = &decoded.ctrl;
        let ordering = match (ctrl.acquire, ctrl.release) {
            (false, false) => Ordering::None,
            (true, false) => Ordering::Acquire,
            (false, true) => Ordering::Release,
            (true, true) => Ordering::AcquireRelease,
        };
        let op = ctrl.atomic_op.unwrap_or_else(|| panic!("{decoded:?} is not atomic"));
        Atomic(op, ctrl.width, ordering)
    });
}

#[rustfmt::skip]
const FLOW: &[(&str, u32, ControlFlow)] = &[
    ("lui a0, 0x12345", 0x12345537, ControlFlow::Sequential),
    ("auipc a0, 0x12345", 0x12345517, ControlFlow::Sequential),
    ("jal ra, .+8", 0x008000ef, ControlFlow::Jump),
    ("jalr ra, 8(a1)", 0x008580e7, ControlFlow::Jump),
    ("beq a1, a2, .+16", 0x00c58863, ControlFlow::Branch),
    ("bne a1, a2, .+16", 0x00c59863, ControlFlow::Branch),
    ("blt a1, a2, .+16", 0x00c5c863, ControlFlow::Branch),
    ("bge a1, a2, .+16", 0x00c5d863, ControlFlow::Branch),
    ("bltu a1, a2, .+16", 0x00c5e863, ControlFlow::Branch),
    ("bgeu a1, a2, .+16", 0x00c5f863, ControlFlow::Branch),
];

#[test]
fn each_jump_and_branch_decodes_to_its_control_flow() {
    check_rows(FLOW, |decoded, _| decoded.ctrl.control_flow);
}

#[rustfmt::skip]
const SYSTEM: &[(&str, u32, System)] = &[
    ("ecall", 0x00000073, Plain(SystemOp::Ecall)),
    ("mret", 0x30200073, Plain(SystemOp::Mret)),
    ("sret", 0x10200073, Plain(SystemOp::Sret)),
    ("wfi", 0x10500073, Plain(SystemOp::Wfi)),
    ("sfence.vma a1, a2", 0x12c58073, Plain(SystemOp::SfenceVma)),
    ("csrrw a0, mstatus, a1", 0x30059573, Csr(CsrOp::Rw)),
    ("csrrs a0, mstatus, a1", 0x3005a573, Csr(CsrOp::Rs)),
    ("csrrc a0, mstatus, a1", 0x3005b573, Csr(CsrOp::Rc)),
    ("csrrwi a0, mstatus, 5", 0x3002d573, Csr(CsrOp::Rwi)),
    ("csrrsi a0, mstatus, 5", 0x3002e573, Csr(CsrOp::Rsi)),
    ("csrrci zero, mstatus, 5", 0x3002f073, Csr(CsrOp::Rci)),
    ("fence rw, rw", 0x0330000f, Plain(SystemOp::Fence)),
    ("fence.i", 0x0000100f, Plain(SystemOp::FenceI)),
    ("cbo.zero (a1)", 0x0045a00f, Plain(SystemOp::CboZero)),
    ("cbo.inval (a1)", 0x0005a00f, Plain(SystemOp::CboInval)),
    ("cbo.clean (a1)", 0x0015a00f, Plain(SystemOp::CboClean)),
    ("cbo.flush (a1)", 0x0025a00f, Plain(SystemOp::CboFlush)),
];

#[test]
fn each_system_instruction_decodes_to_its_system_or_csr_operation() {
    check_rows(SYSTEM, |decoded, _| match decoded.ctrl.csr_op {
        CsrOp::None => Plain(decoded.ctrl.system_op),
        op => Csr(op),
    });
}

#[test]
fn ebreak_raises_a_breakpoint_at_its_pc() {
    assert_eq!(decode(0x0010_0073).trap, Some(Trap::Breakpoint(PC)));
}

/// An R-type word.
const fn r_type(opcode: u32, funct3: u32, funct7: u32) -> u32 {
    const RD: u32 = 10;
    const RS1: u32 = 11;
    const RS2: u32 = 12;
    (funct7 << 25) | (RS2 << 20) | (RS1 << 15) | (funct3 << 12) | (RD << 7) | opcode
}

/// An I-type word with the whole 12-bit immediate given.
const fn i_type(opcode: u32, funct3: u32, imm12: u32) -> u32 {
    const RD: u32 = 10;
    const RS1: u32 = 11;
    (imm12 << 20) | (RS1 << 15) | (funct3 << 12) | (RD << 7) | opcode
}

/// A B-type word with a zero offset.
const fn b_type(funct3: u32) -> u32 {
    const BRANCH: u32 = 0x63;
    (12 << 20) | (11 << 15) | (funct3 << 12) | BRANCH
}

/// `word` with its rs2 field cleared.
const fn without_rs2(word: u32) -> u32 {
    word & !(0x1F << 20)
}

const OP_IMM: u32 = 0x13;
const OP_IMM_32: u32 = 0x1b;
const OP: u32 = 0x33;
const OP_32: u32 = 0x3b;
const JALR: u32 = 0x67;
const OP_FP: u32 = 0x53;
const AMO: u32 = 0x2f;

/// Encodings the ratified extensions leave reserved. The names are what
/// the word would be if its extension had defined it.
#[rustfmt::skip]
const RESERVED: &[(&str, u32)] = &[
    ("sltw", r_type(OP_32, 2, 0x00)),
    ("sltuw", r_type(OP_32, 3, 0x00)),
    ("xorw", r_type(OP_32, 4, 0x00)),
    ("orw", r_type(OP_32, 6, 0x00)),
    ("andw", r_type(OP_32, 7, 0x00)),
    ("mulhw", r_type(OP_32, 1, 0x01)),
    ("mulhsuw", r_type(OP_32, 2, 0x01)),
    ("mulhuw", r_type(OP_32, 3, 0x01)),
    ("xnorw", r_type(OP_32, 4, 0x20)),
    ("ornw", r_type(OP_32, 6, 0x20)),
    ("andnw", r_type(OP_32, 7, 0x20)),
    ("minw", r_type(OP_32, 4, 0x05)),
    ("minuw", r_type(OP_32, 5, 0x05)),
    ("maxw", r_type(OP_32, 6, 0x05)),
    ("maxuw", r_type(OP_32, 7, 0x05)),
    ("clmulw", r_type(OP_32, 1, 0x05)),
    ("bclrw", r_type(OP_32, 1, 0x24)),
    ("bextw", r_type(OP_32, 5, 0x24)),
    ("binvw", r_type(OP_32, 1, 0x34)),
    ("bsetw", r_type(OP_32, 1, 0x14)),
    ("packhw", r_type(OP_32, 7, 0x04)),
    ("sub with funct3=1", r_type(OP, 1, 0x20)),
    ("add.uw on OP", r_type(OP, 0, 0x04)),
    ("sltiw", i_type(OP_IMM_32, 2, 5)),
    ("sltiuw", i_type(OP_IMM_32, 3, 5)),
    ("xoriw", i_type(OP_IMM_32, 4, 5)),
    ("oriw", i_type(OP_IMM_32, 6, 5)),
    ("andiw", i_type(OP_IMM_32, 7, 5)),
    ("slliw with shamt[5] set", i_type(OP_IMM_32, 1, 0x020)),
    ("srliw with shamt[5] set", i_type(OP_IMM_32, 5, 0x020)),
    ("sraiw with shamt[5] set", i_type(OP_IMM_32, 5, 0x420)),
    ("roriw with shamt[5] set", i_type(OP_IMM_32, 5, 0x620)),
    ("bclriw", i_type(OP_IMM_32, 1, 0x480)),
    ("bsetiw", i_type(OP_IMM_32, 1, 0x280)),
    ("binviw", i_type(OP_IMM_32, 1, 0x680)),
    ("bextiw", i_type(OP_IMM_32, 5, 0x480)),
    ("slli with funct6=1", i_type(OP_IMM, 1, 0x043)),
    ("srli with funct6=1", i_type(OP_IMM, 5, 0x043)),
    ("srai with funct6=0x11", i_type(OP_IMM, 5, 0x443)),
    ("an undefined Zbb unary op", i_type(OP_IMM, 1, 0x603)),
    ("slli.uw on OP-IMM", i_type(OP_IMM, 1, 0x085)),
    ("branch with funct3=2", b_type(2)),
    ("branch with funct3=3", b_type(3)),
    ("jalr with funct3=1", i_type(JALR, 1, 8)),
    ("jalr with funct3=7", i_type(JALR, 7, 8)),
    ("load with funct3=7", i_type(0x03, 7, 8)),
    ("fsqrt.s with rs2 set", r_type(OP_FP, 0, 0x2c)),
    ("fclass.d with rs2 set", r_type(OP_FP, 1, 0x71)),
    ("fmv.x.w with rs2 set", r_type(OP_FP, 0, 0x70)),
    ("fmv.w.x with rs2 set", r_type(OP_FP, 0, 0x78)),
    ("fmv.w.x with funct3=1", without_rs2(r_type(OP_FP, 1, 0x78))),
    ("fcvt.s.d with rs2=3", without_rs2(r_type(OP_FP, 0, 0x20)) | (3 << 20)),
    ("fadd.q", r_type(OP_FP, 0, 0x03)),
    ("lr.w with rs2 set", r_type(AMO, 2, 0b00010 << 2)),
    ("fadd.s with rm=5", r_type(OP_FP, 5, 0x00)),
    ("fcvt.w.s with rm=6", without_rs2(r_type(OP_FP, 6, 0x60))),
    ("fmadd.s with rm=5", r_type(0x43, 5, 0x00)),
];

#[test]
fn reserved_encodings_raise_illegal_instruction() {
    let wrong: Vec<String> = RESERVED
        .iter()
        .filter_map(|(name, word)| {
            let trap = decode(*word).trap;
            (trap != Some(Trap::IllegalInstruction(*word)))
                .then(|| format!("{name} ({word:#010x}): {trap:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{} reserved words do not trap:\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn a_rounding_mode_field_decodes_to_static_or_dynamic_rounding() {
    let fadd_s = |rm: u32| r_type(OP_FP, rm, 0x00);

    assert_eq!(decode(fadd_s(0b001)).ctrl.fp_rm, Some(Rounding::Static(RoundingMode::Rtz)));
    assert_eq!(decode(fadd_s(0b111)).ctrl.fp_rm, Some(Rounding::Dynamic));
}

#[test]
fn an_op_that_does_not_round_carries_no_rounding_mode() {
    let fsgnjx_s = r_type(OP_FP, 2, 0x10);

    assert_eq!(decode(fsgnjx_s).ctrl.fp_rm, None);
}

const A0: RegIdx = RegIdx::new(10);
const FA0: RegIdx = RegIdx::new(10);

fn dest_reg(word: u32) -> Option<ArchReg> {
    let decoded = decode(word);
    decoded.ctrl.dest(decoded.fields.rd)
}

#[test]
fn an_fp_compare_writes_its_integer_destination() {
    let feq_s = 0xa0c5_a553;

    assert_eq!(dest_reg(feq_s), Some(ArchReg::Int(A0)));
}

#[test]
fn an_fp_add_writes_its_fp_destination() {
    let fadd_s = 0x00c5_8553;

    assert_eq!(dest_reg(fadd_s), Some(ArchReg::Fp(FA0)));
}

#[test]
fn a_write_to_x0_and_a_store_have_no_destination() {
    let addi_x0 = 0x0055_8013;
    let sw = 0x00c5_a423;

    assert_eq!(dest_reg(addi_x0), None);
    assert_eq!(dest_reg(sw), None);
}

#[test]
fn only_a_fused_multiply_add_reads_rs3() {
    let fa3 = RegIdx::new(13);
    let fmadd_s = decode(0x68c5_f543);
    let fadd_s = decode(0x00c5_8553);

    assert_eq!(fmadd_s.ctrl.src3(fa3), Some(ArchReg::Fp(fa3)));
    assert_eq!(fadd_s.ctrl.src3(fa3), None);
}
