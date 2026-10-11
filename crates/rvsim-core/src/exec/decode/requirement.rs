//! Which extension an instruction belongs to, so a hart configured
//! without it refuses the instruction as hardware without it would.

use crate::exec::signals::ControlSignals;
use crate::isa::extension::{Extension, Requirement};
use crate::isa::instruction::InstructionBits;
use crate::isa::op::{AluOp, SystemOp, VectorOp};
use crate::isa::op::{CryptoOp, IntOp, VecAluOp, VecClass};

/// What the instruction decoded to `ctrl` from `inst` needs the hart to
/// implement. The base ISA, Zicsr, Zifencei, Zicntr and the hints are
/// always implemented, so they need nothing.
#[must_use]
pub fn requirement(ctrl: &ControlSignals, inst: u32) -> Requirement {
    if ctrl.vec_op != VectorOp::None {
        return vector_requirement(ctrl.vec_op);
    }
    if ctrl.atomic_op.is_some() {
        return Requirement::Letter('a');
    }
    match ctrl.system_op {
        SystemOp::CboZero => return Requirement::any_of(&[Extension::Zicboz]),
        SystemOp::CboInval | SystemOp::CboClean | SystemOp::CboFlush => {
            return Requirement::any_of(&[Extension::Zicbom]);
        }
        _ => {}
    }
    if let Some(bits) = bit_manipulation_requirement(ctrl.alu, inst) {
        return bits;
    }
    if uses_fp_registers(ctrl) {
        return fp_requirement(ctrl);
    }
    if matches!(
        ctrl.alu,
        AluOp::Mul
            | AluOp::Mulh
            | AluOp::Mulhsu
            | AluOp::Mulhu
            | AluOp::Div
            | AluOp::Divu
            | AluOp::Rem
            | AluOp::Remu
    ) {
        return Requirement::Letter('m');
    }
    Requirement::Base
}

/// The requirement of a bit-manipulation operation, or `None` when `alu`
/// is not one.
fn bit_manipulation_requirement(alu: AluOp, inst: u32) -> Option<Requirement> {
    use Extension::{Zba, Zbb, Zbc, Zbkb, Zbkc, Zbkx, Zbs};
    let extensions: &[Extension] = match alu {
        AluOp::Sh1Add
        | AluOp::Sh2Add
        | AluOp::Sh3Add
        | AluOp::AddUw
        | AluOp::Sh1AddUw
        | AluOp::Sh2AddUw
        | AluOp::Sh3AddUw
        | AluOp::SlliUw => &[Zba],
        AluOp::Clz
        | AluOp::Ctz
        | AluOp::Cpop
        | AluOp::Max
        | AluOp::Maxu
        | AluOp::Min
        | AluOp::Minu
        | AluOp::SextB
        | AluOp::SextH
        | AluOp::OrcB => &[Zbb],
        AluOp::Andn | AluOp::Orn | AluOp::Xnor | AluOp::Rol | AluOp::Ror | AluOp::Rev8 => {
            &[Zbb, Zbkb]
        }
        AluOp::Clmul | AluOp::Clmulh => &[Zbc, Zbkc],
        AluOp::Clmulr => &[Zbc],
        AluOp::Bclr | AluOp::Bext | AluOp::Binv | AluOp::Bset => &[Zbs],
        // `zext.h` is `packw rd, rs1, x0`, and Zbb's.
        AluOp::Packw if inst.rs2().is_zero() => &[Zbb, Zbkb],
        AluOp::Brev8 | AluOp::Pack | AluOp::Packh | AluOp::Packw => &[Zbkb],
        AluOp::Xperm4 | AluOp::Xperm8 => &[Zbkx],
        _ => return None,
    };
    Some(Requirement::any_of(extensions))
}

/// Whether the instruction reads or writes a floating-point register.
const fn uses_fp_registers(ctrl: &ControlSignals) -> bool {
    ctrl.fp_reg_write || ctrl.rs1_fp || ctrl.rs2_fp || ctrl.rs3_fp
}

/// The requirement of a floating-point operation: Zfh for half
/// precision, else F or D by the operand width.
fn fp_requirement(ctrl: &ControlSignals) -> Requirement {
    let half = ctrl.is_f16
        || matches!(ctrl.alu, AluOp::FCvtSH | AluOp::FCvtHS | AluOp::FCvtDH | AluOp::FCvtHD);
    if half {
        return Requirement::any_of(&[Extension::Zfh]);
    }
    let double = if ctrl.mem_read || ctrl.mem_write {
        ctrl.width.bytes() == 8
    } else {
        !ctrl.is_rv32 || matches!(ctrl.alu, AluOp::FCvtSD | AluOp::FCvtDS)
    };
    Requirement::Letter(if double { 'd' } else { 'f' })
}

/// The requirement of a vector operation: the vector crypto and
/// bit-manipulation extensions by instruction, V for the rest.
fn vector_requirement(op: VectorOp) -> Requirement {
    use Extension::{Zvbb, Zvbc, Zvkb, Zvkg, Zvkned, Zvknha, Zvknhb, Zvksed, Zvksh};
    if op == VectorOp::VWsll {
        return Requirement::any_of(&[Zvbb]);
    }
    let extensions: &[Extension] = match op.class() {
        VecClass::Crypto(crypto) => match crypto {
            CryptoOp::AesEm
            | CryptoOp::AesEf
            | CryptoOp::AesDm
            | CryptoOp::AesDf
            | CryptoOp::AesZ
            | CryptoOp::AesKf1
            | CryptoOp::AesKf2 => &[Zvkned],
            CryptoOp::Sha2Ms | CryptoOp::Sha2Ch | CryptoOp::Sha2Cl => &[Zvknha, Zvknhb],
            CryptoOp::Sm3Me | CryptoOp::Sm3C => &[Zvksh],
            CryptoOp::Sm4R | CryptoOp::Sm4K => &[Zvksed],
            CryptoOp::Ghsh | CryptoOp::Gmul => &[Zvkg],
        },
        VecClass::Alu(VecAluOp::Int(int)) => match int {
            IntOp::AndN | IntOp::Brev8 | IntOp::Rev8 | IntOp::Rol | IntOp::Ror => &[Zvbb, Zvkb],
            IntOp::Brev | IntOp::Clz | IntOp::Ctz | IntOp::CpopV => &[Zvbb],
            IntOp::ClMul | IntOp::ClMulH => &[Zvbc],
            _ => return Requirement::Letter('v'),
        },
        _ => return Requirement::Letter('v'),
    };
    Requirement::any_of(extensions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::decode::decode_inst;

    fn requirement_of(inst: u32) -> Requirement {
        let decoded = decode_inst(inst, 0, 0);
        assert!(decoded.trap.is_none(), "{inst:#010x} decodes");
        requirement(&decoded.ctrl, inst)
    }

    #[test]
    fn base_instructions_need_nothing() {
        assert_eq!(requirement_of(0x0000_0013), Requirement::Base); // addi x0, x0, 0
        assert_eq!(requirement_of(0x0000_1073), Requirement::Base); // csrw ustatus? csrrw x0
        assert_eq!(requirement_of(0x0000_100f), Requirement::Base); // fence.i
    }

    #[test]
    fn multiply_atomics_and_floating_point_need_their_letters() {
        assert_eq!(requirement_of(0x0220_0033), Requirement::Letter('m')); // mul x0, x0, x2
        assert_eq!(requirement_of(0x1000_202f), Requirement::Letter('a')); // lr.w x0, (x0)
        assert_eq!(requirement_of(0x0000_0053), Requirement::Letter('f')); // fadd.s f0, f0, f0
        assert_eq!(requirement_of(0x0200_0053), Requirement::Letter('d')); // fadd.d f0, f0, f0
        assert_eq!(requirement_of(0x0000_2007), Requirement::Letter('f')); // flw f0, 0(x0)
        assert_eq!(requirement_of(0x0000_3007), Requirement::Letter('d')); // fld f0, 0(x0)
    }

    #[test]
    fn half_precision_needs_zfh() {
        let zfh = Requirement::any_of(&[Extension::Zfh]);
        assert_eq!(requirement_of(0x0400_0053), zfh); // fadd.h f0, f0, f0
        assert_eq!(requirement_of(0x4020_0053), zfh); // fcvt.s.h f0, f0
    }

    #[test]
    fn bit_manipulation_names_every_extension_that_has_the_instruction() {
        let any = |list: &[Extension]| Requirement::any_of(list);
        assert_eq!(requirement_of(0x2000_2033), any(&[Extension::Zba])); // sh1add x0, x0, x0
        assert_eq!(requirement_of(0x6000_1013), any(&[Extension::Zbb])); // clz x0, x0
        assert_eq!(requirement_of(0x6000_1033), any(&[Extension::Zbb, Extension::Zbkb])); // rol
        assert_eq!(requirement_of(0x0a00_1033), any(&[Extension::Zbc, Extension::Zbkc])); // clmul
        assert_eq!(requirement_of(0x0a00_2033), any(&[Extension::Zbc])); // clmulr
        assert_eq!(requirement_of(0x4800_1033), any(&[Extension::Zbs])); // bclr
        assert_eq!(requirement_of(0x0800_4033), any(&[Extension::Zbkb])); // pack x0, x0, x0
        assert_eq!(requirement_of(0x0800_403b), any(&[Extension::Zbb, Extension::Zbkb])); // zext.h
        assert_eq!(requirement_of(0x2800_2033), any(&[Extension::Zbkx])); // xperm4
    }

    #[test]
    fn cache_block_operations_need_their_extension() {
        assert_eq!(requirement_of(0x0040_200f), Requirement::any_of(&[Extension::Zicboz]));
        assert_eq!(requirement_of(0x0010_200f), Requirement::any_of(&[Extension::Zicbom]));
    }

    #[test]
    fn vector_instructions_need_v_or_their_vector_extension() {
        assert_eq!(requirement_of(0x0200_0057), Requirement::Letter('v')); // vadd.vv
        assert_eq!(
            requirement_of(0x0600_0057),
            Requirement::any_of(&[Extension::Zvbb, Extension::Zvkb])
        ); // vandn.vv
        assert_eq!(requirement_of(0xa200_a077), Requirement::any_of(&[Extension::Zvkned])); // vaesem.vv
    }
}
