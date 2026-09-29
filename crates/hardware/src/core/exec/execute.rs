//! Executing an instruction.
//!
//! Operand selection, ALU/FPU evaluation, branch and jump targets, and the
//! privilege and CSR checks that decide what a system instruction does.

use crate::common::SfenceVmaInfo;
use crate::common::error::Trap;
use crate::core::Hart;
use crate::core::arch::csr;
use crate::core::exec::arch::ArchState;
use crate::core::exec::cbo::{self, CboEffect};
use crate::core::exec::inst::Inst;
use crate::core::exec::signals::{OpASrc, OpBSrc};
use crate::core::units::alu::Alu;
use crate::core::units::fpu::Fpu;
use crate::core::units::vpu::fpu::is_vec_fp;
use crate::isa::csr::CsrAddr;
use crate::isa::encoding::rv64i::{funct3, opcodes};
use crate::isa::fp::RoundingMode;
use crate::isa::op::{AluOp, CsrOp, SystemOp, VectorOp};
use crate::isa::privileged::mode::PrivilegeMode;
use crate::trace_csr;

const FUNCT3_SHIFT: u32 = 12;
const FUNCT3_MASK: u32 = 0x7;
const JALR_ALIGNMENT_MASK: u64 = !1;
const MSTATUS_TVM_BIT: u32 = 20;
const MSTATUS_TW_BIT: u32 = 21;
const MSTATUS_TSR_BIT: u32 = 22;

/// The ALU's two main operands, selected by the decoded sources.
pub const fn operands(inst: &Inst) -> (u64, u64) {
    let op_a = match inst.ctrl.a_src {
        OpASrc::Reg1 => inst.rv1,
        OpASrc::Pc => inst.pc,
        OpASrc::Zero => 0,
    };
    let op_b = match inst.ctrl.b_src {
        OpBSrc::Reg2 => inst.rv2,
        OpBSrc::Imm => inst.imm as u64,
        OpBSrc::Zero => 0,
    };
    (op_a, op_b)
}

const fn fs_off(hart: &Hart) -> bool {
    hart.csrs.mstatus & csr::MSTATUS_FS == 0
}

const fn vs_off(hart: &Hart) -> bool {
    hart.csrs.mstatus & csr::MSTATUS_VS == 0
}

/// True when `id` needs a unit `mstatus` has switched Off.
///
/// Any vector instruction while VS is Off, and anything touching the FP
/// registers or doing vector floating-point arithmetic while FS is Off.
/// Checked here rather than at decode because `mstatus` writes apply at
/// commit.
pub const fn unit_disabled(hart: &Hart, inst: &Inst) -> bool {
    let is_vector = !matches!(inst.ctrl.vec_op, VectorOp::None);
    let is_fp = inst.ctrl.fp_reg_write
        || inst.ctrl.rs1_fp
        || inst.ctrl.rs2_fp
        || inst.ctrl.rs3_fp
        || is_vector_fp(inst.ctrl.vec_op);
    (is_vector && vs_off(hart)) || (is_fp && fs_off(hart))
}

/// True for vector instructions that do floating-point arithmetic.
const fn is_vector_fp(op: VectorOp) -> bool {
    is_vec_fp(op)
        || matches!(
            op,
            VectorOp::VFRedMax
                | VectorOp::VFRedMin
                | VectorOp::VFRedOSum
                | VectorOp::VFRedUSum
                | VectorOp::VFWRedOSum
                | VectorOp::VFWRedUSum
        )
}

/// True when `addr` belongs to a unit `mstatus` has switched Off.
fn csr_unit_disabled(state: &impl ArchState, addr: CsrAddr) -> bool {
    let fp_csr = addr == csr::FFLAGS || addr == csr::FRM || addr == csr::FCSR;
    let vector_csr = addr == csr::VSTART
        || addr == csr::VXSAT
        || addr == csr::VXRM
        || addr == csr::VCSR
        || addr == csr::VL
        || addr == csr::VTYPE
        || addr == csr::VLENB;
    (fp_csr && fs_off(state.hart())) || (vector_csr && vs_off(state.hart()))
}

const fn mstatus_bit(hart: &Hart, bit: u32) -> bool {
    (hart.csrs.mstatus >> bit) & 1 != 0
}

/// The illegal-instruction trap an xRET, WFI or SFENCE.VMA raises at the
/// current privilege level, if any.
pub const fn privileged_op_fault(hart: &Hart, inst: &Inst) -> Option<Trap> {
    let privilege = hart.privilege;
    let illegal = match inst.ctrl.system_op {
        SystemOp::Mret => !matches!(privilege, PrivilegeMode::Machine),
        SystemOp::Sret => match privilege {
            PrivilegeMode::User => true,
            PrivilegeMode::Supervisor => mstatus_bit(hart, MSTATUS_TSR_BIT),
            PrivilegeMode::Machine => false,
        },
        SystemOp::Wfi => match privilege {
            PrivilegeMode::User => true,
            PrivilegeMode::Supervisor => mstatus_bit(hart, MSTATUS_TW_BIT),
            PrivilegeMode::Machine => false,
        },
        SystemOp::SfenceVma => {
            matches!(privilege, PrivilegeMode::Supervisor) && mstatus_bit(hart, MSTATUS_TVM_BIT)
        }
        _ => false,
    };
    if illegal { Some(Trap::IllegalInstruction(inst.bits)) } else { None }
}

/// The environment-call trap for the current privilege level.
pub const fn ecall_trap(hart: &Hart) -> Trap {
    match hart.privilege {
        PrivilegeMode::User => Trap::EnvironmentCallFromUMode,
        PrivilegeMode::Supervisor => Trap::EnvironmentCallFromSMode,
        PrivilegeMode::Machine => Trap::EnvironmentCallFromMMode,
    }
}

/// What executing a system instruction does, whatever engine executes it.
#[derive(Clone, Debug)]
pub enum SystemEffect {
    /// Not a system instruction, or FENCE, which orders memory at issue and
    /// retirement rather than here.
    NotSystem,
    /// The instruction traps.
    Trap(Trap),
    /// FENCE.I, MRET, SRET or WFI: the effect waits for retirement.
    AtRetire,
    /// SFENCE.VMA: the translations it names are flushed at retirement.
    SfenceVma(SfenceVmaInfo),
    /// A permitted cache-block operation on the block at `rs1`.
    Cbo(CboEffect),
    /// A permitted CSR access.
    Csr(CsrAccess),
}

/// What the system instruction `id` does at the current privilege level.
pub fn system_effect(state: &impl ArchState, inst: &Inst) -> SystemEffect {
    if let Some(trap) = privileged_op_fault(state.hart(), inst) {
        return SystemEffect::Trap(trap);
    }
    match inst.ctrl.system_op {
        SystemOp::None | SystemOp::Fence => SystemEffect::NotSystem,
        SystemOp::FenceI | SystemOp::Mret | SystemOp::Sret | SystemOp::Wfi => {
            SystemEffect::AtRetire
        }
        SystemOp::SfenceVma => SystemEffect::SfenceVma(SfenceVmaInfo {
            rs1_idx: inst.rs1,
            rs2_idx: inst.rs2,
            rs1_val: inst.rv1,
            rs2_val: inst.rv2,
        }),
        SystemOp::CboZero | SystemOp::CboInval | SystemOp::CboClean | SystemOp::CboFlush => {
            let hart = state.hart();
            match cbo::gate(&hart.csrs, hart.privilege, inst.ctrl.system_op, inst.bits) {
                Ok(effect) => SystemEffect::Cbo(effect),
                Err(trap) => SystemEffect::Trap(trap),
            }
        }
        SystemOp::Ecall => SystemEffect::Trap(ecall_trap(state.hart())),
        SystemOp::Csr => match csr_access(state, inst) {
            Ok(access) => SystemEffect::Csr(access),
            Err(trap) => SystemEffect::Trap(trap),
        },
    }
}

/// A CSR write an instruction makes when it retires.
#[derive(Clone, Copy, Debug)]
pub struct CsrWrite {
    /// The CSR.
    pub addr: CsrAddr,
    /// Its value before the write.
    pub old: u64,
    /// The value written.
    pub new: u64,
}

/// What a permitted CSR instruction reads, and the write it makes when it
/// retires.
#[derive(Clone, Debug)]
pub struct CsrAccess {
    /// The value written to `rd`.
    pub old: u64,
    /// The write commit applies; `None` for forms that must not write.
    pub update: Option<CsrWrite>,
}

/// Checks the CSR instruction `id` against the current privilege and
/// counter enables, and computes the value it reads and the write it makes.
///
/// # Errors
///
/// The illegal-instruction trap when the access is not permitted.
pub fn csr_access(state: &impl ArchState, inst: &Inst) -> Result<CsrAccess, Trap> {
    let addr = inst.ctrl.csr_addr;
    let illegal = Trap::IllegalInstruction(inst.bits);
    let writes = csr_op_writes(inst);
    let privilege = state.hart().privilege;

    let satp_trapped = addr == csr::SATP
        && matches!(privilege, PrivilegeMode::Supervisor)
        && mstatus_bit(state.hart(), MSTATUS_TVM_BIT);
    if satp_trapped
        || csr_unit_disabled(state, addr)
        || counter_access_denied(state, inst)
        || !state.hart().is_valid_csr(addr)
        || u32::from(privilege.to_u8()) < addr.privilege_level() as u32
        || (addr.is_read_only() && writes)
    {
        return Err(illegal);
    }

    let old = state.csr_read(addr);
    let base = state.csr_read_for_update(addr);
    let src = match inst.ctrl.csr_op {
        CsrOp::Rwi | CsrOp::Rsi | CsrOp::Rci => u64::from(inst.rs1.as_u8() & 0x1f),
        _ => inst.rv1,
    };
    let new = match inst.ctrl.csr_op {
        CsrOp::Rw | CsrOp::Rwi => src,
        CsrOp::Rs | CsrOp::Rsi => base | src,
        CsrOp::Rc | CsrOp::Rci => base & !src,
        CsrOp::None => old,
    };
    trace_csr!(state.tracing();
        op        = "write-deferred",
        pc        = %crate::trace::Hex(inst.pc),
        csr_addr  = %crate::trace::Hex32(addr.as_u32()),
        csr_op    = ?inst.ctrl.csr_op,
        old_val   = %crate::trace::Hex(old),
        new_val   = %crate::trace::Hex(new),
        writes,
        "EX: CSR access"
    );
    let update = writes.then_some(CsrWrite { addr, old, new });
    Ok(CsrAccess { old, update })
}

/// Whether the CSR form writes: CSRRS/CSRRC with rs1=x0 and CSRRSI/CSRRCI
/// with uimm=0 only read.
const fn csr_op_writes(inst: &Inst) -> bool {
    match inst.ctrl.csr_op {
        CsrOp::Rw | CsrOp::Rwi => true,
        CsrOp::Rs | CsrOp::Rc => !inst.rs1.is_zero(),
        CsrOp::Rsi | CsrOp::Rci => (inst.rs1.as_u8() & 0x1f) != 0,
        CsrOp::None => false,
    }
}

/// True when `mcounteren`/`scounteren` hide the CYCLE, TIME or INSTRET
/// counter `id` reads from the current privilege level.
fn counter_access_denied(state: &impl ArchState, inst: &Inst) -> bool {
    let addr = inst.ctrl.csr_addr;
    let bit = if addr == csr::CYCLE {
        0
    } else if addr == csr::TIME {
        1
    } else if addr == csr::INSTRET {
        2
    } else {
        return false;
    };
    let mask = 1u64 << bit;
    let csrs = &state.hart().csrs;
    match state.hart().privilege {
        PrivilegeMode::Supervisor => csrs.mcounteren & mask == 0,
        PrivilegeMode::User => csrs.mcounteren & mask == 0 || csrs.scounteren & mask == 0,
        PrivilegeMode::Machine => false,
    }
}

/// Evaluates `id`'s ALU or FPU operation and returns `(result, fp_flags)`.
pub fn evaluate(state: &impl ArchState, inst: &Inst, op_a: u64, op_b: u64) -> (u64, u8) {
    let fp_rm = inst.ctrl.fp_rm.or_else(|| RoundingMode::from_bits(state.hart().csrs.frm as u8));
    compute_alu(inst.ctrl.alu, op_a, op_b, inst.rv3, inst.ctrl.is_f16, inst.ctrl.is_rv32, fp_rm)
}

/// Checks a taken branch or jump target's alignment: four bytes without
/// the C extension (IALIGN=32), two with it.
///
/// # Errors
///
/// The instruction-address-misaligned trap for a misaligned `target`.
pub const fn check_target_alignment(hart: &Hart, target: u64) -> Result<(), Trap> {
    if target & csr::ialign_low_bits(hart.csrs.misa) == 0 {
        return Ok(());
    }
    Err(Trap::InstructionAddressMisaligned(target))
}

/// Whether the conditional branch `inst` is taken with operands `op_a` and
/// `op_b`.
#[must_use]
pub const fn branch_taken(inst: u32, op_a: u64, op_b: u64) -> bool {
    match (inst >> FUNCT3_SHIFT) & FUNCT3_MASK {
        funct3::BEQ => op_a == op_b,
        funct3::BNE => op_a != op_b,
        funct3::BLT => (op_a as i64) < (op_b as i64),
        funct3::BGE => (op_a as i64) >= (op_b as i64),
        funct3::BLTU => op_a < op_b,
        funct3::BGEU => op_a >= op_b,
        _ => false,
    }
}

/// Whether `inst` is a JALR (rather than a JAL).
#[must_use]
pub const fn is_jalr(inst: &Inst) -> bool {
    (inst.bits & crate::common::constants::OPCODE_MASK) == opcodes::OP_JALR
}

/// Where the JAL or JALR `id` jumps.
#[must_use]
pub const fn jump_target(inst: &Inst) -> u64 {
    if is_jalr(inst) {
        inst.rv1.wrapping_add(inst.imm as u64) & JALR_ALIGNMENT_MASK
    } else {
        inst.pc.wrapping_add(inst.imm as u64)
    }
}

/// Runs `convert` with the host FPU set to `rm` and returns its result with
/// the IEEE flags the host raised, so conversions report INEXACT/OVERFLOW.
fn on_host_fpu(rm: RoundingMode, convert: impl FnOnce() -> u64) -> (u64, u8) {
    use crate::core::units::fpu::{
        clear_host_fp_flags, read_host_fp_flags, restore_host_round_mode, set_host_round_mode,
    };
    let saved = set_host_round_mode(rm);
    clear_host_fp_flags();
    let value = std::hint::black_box(convert());
    let flags = read_host_fp_flags();
    restore_host_round_mode(saved);
    (value, flags.bits())
}

/// Computes the ALU/FPU result and returns `(result, fp_flags)`.
/// `fp_flags` is non-zero only for floating-point arithmetic operations.
pub fn compute_alu(
    alu_op: AluOp,
    op_a: u64,
    op_b: u64,
    op_c: u64,
    is_f16: bool,
    is_rv32: bool,
    fp_rm: Option<RoundingMode>,
) -> (u64, u8) {
    use crate::core::units::fpu::half::{box_f16, f16_to_f32, unbox_f16};
    use crate::core::units::fpu::nan_handling::{box_f32_canon, canonicalize_f64_bits, unbox_f32};
    use std::hint::black_box;

    let rm = fp_rm.unwrap_or(RoundingMode::Rne);
    match alu_op {
        AluOp::FCvtSW if !is_f16 => on_host_fpu(rm, || {
            let v = black_box(op_a as i32);
            if is_rv32 { Fpu::box_f32(v as f32) } else { f64::from(v).to_bits() }
        }),
        AluOp::FCvtSWU if !is_f16 => on_host_fpu(rm, || {
            let v = black_box(op_a as u32);
            if is_rv32 { Fpu::box_f32(v as f32) } else { f64::from(v).to_bits() }
        }),
        AluOp::FCvtSL if !is_f16 => on_host_fpu(rm, || {
            let v = black_box(op_a as i64);
            if is_rv32 { Fpu::box_f32(v as f32) } else { (v as f64).to_bits() }
        }),
        AluOp::FCvtSLU if !is_f16 => on_host_fpu(rm, || {
            let v = black_box(op_a);
            if is_rv32 { Fpu::box_f32(v as f32) } else { (v as f64).to_bits() }
        }),
        AluOp::FCvtSD if !is_f16 => {
            on_host_fpu(rm, || box_f32_canon(black_box(f64::from_bits(op_a)) as f32))
        }
        AluOp::FCvtDS if !is_f16 => {
            on_host_fpu(rm, || canonicalize_f64_bits(f64::from(black_box(unbox_f32(op_a)))))
        }
        AluOp::FCvtSH if !is_f16 => on_host_fpu(rm, || box_f32_canon(f16_to_f32(unbox_f16(op_a)))),
        AluOp::FCvtDH if !is_f16 => on_host_fpu(rm, || {
            canonicalize_f64_bits(f64::from(black_box(f16_to_f32(unbox_f16(op_a)))))
        }),
        AluOp::FMvToF => {
            let value = if is_f16 {
                box_f16(op_a as u16)
            } else if is_rv32 {
                Fpu::box_f32(f32::from_bits(op_a as u32))
            } else {
                op_a
            };
            (value, 0)
        }
        _ if is_fp_op(alu_op) => {
            let (result, fp_flags) =
                Fpu::execute_full_rm(alu_op, op_a, op_b, op_c, is_f16, is_rv32, rm);
            (result, fp_flags.bits())
        }
        _ => (Alu::execute(alu_op, op_a, op_b, op_c, is_rv32), 0),
    }
}

const fn is_fp_op(alu_op: AluOp) -> bool {
    matches!(
        alu_op,
        AluOp::FAdd
            | AluOp::FSub
            | AluOp::FMul
            | AluOp::FDiv
            | AluOp::FSqrt
            | AluOp::FMin
            | AluOp::FMax
            | AluOp::FMAdd
            | AluOp::FMSub
            | AluOp::FNMAdd
            | AluOp::FNMSub
            | AluOp::FSgnJ
            | AluOp::FSgnJN
            | AluOp::FSgnJX
            | AluOp::FEq
            | AluOp::FLt
            | AluOp::FLe
            | AluOp::FClass
            | AluOp::FCvtWS
            | AluOp::FCvtWUS
            | AluOp::FCvtLS
            | AluOp::FCvtLUS
            | AluOp::FCvtSW
            | AluOp::FCvtSWU
            | AluOp::FCvtSL
            | AluOp::FCvtSLU
            | AluOp::FCvtSD
            | AluOp::FCvtDS
            | AluOp::FCvtSH
            | AluOp::FCvtHS
            | AluOp::FCvtDH
            | AluOp::FCvtHD
            | AluOp::FMvToX
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_to_float_conversions_are_exact_for_small_values() {
        let rne = Some(RoundingMode::Rne);

        let (res, flags) = compute_alu(AluOp::FCvtSW, 1, 0, 0, false, false, rne);
        assert_eq!(res, (1.0f64).to_bits());
        assert_eq!(flags, 0);

        let (res, flags) = compute_alu(AluOp::FCvtSW, 1, 0, 0, false, true, rne);
        assert_eq!(res, 0xFFFF_FFFF_0000_0000 | u64::from((1.0f32).to_bits()));
        assert_eq!(flags, 0);
    }

    #[test]
    fn move_to_float_passes_the_bits_through() {
        let (res, flags) =
            compute_alu(AluOp::FMvToF, 42, 0, 0, false, false, Some(RoundingMode::Rne));

        assert_eq!(res, 42);
        assert_eq!(flags, 0);
    }

    #[test]
    fn inexact_conversion_raises_the_inexact_flag() {
        let (_, flags) = compute_alu(
            AluOp::FCvtSL,
            (1u64 << 60) + 1,
            0,
            0,
            false,
            true,
            Some(RoundingMode::Rne),
        );

        assert_ne!(flags, 0);
    }
}
