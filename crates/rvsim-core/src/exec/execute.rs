//! Executing an instruction.
//!
//! Operand selection, ALU/FPU evaluation, branch and jump targets, and the
//! privilege and CSR checks that decide what a system instruction does.

use crate::arch::Hart;
use crate::arch::translation::SfenceVmaInfo;
use crate::common::VirtAddr;
use crate::exec::cbo;
use crate::exec::compute::alu;
use crate::exec::compute::fpu;
use crate::exec::compute::fpu::exact::{self, Exact};
use crate::exec::compute::vector::fpu::is_vec_fp;
use crate::exec::execute::fpu::host::{
    clear_host_fp_flags, read_host_fp_flags, restore_host_round_mode, set_host_round_mode,
};
use crate::exec::execute::fpu::nan_handling::box_f32;
use crate::exec::inst::Inst;
use crate::exec::signals::{ControlFlow, JumpKind, OpASrc, OpBSrc};
use crate::exec::state::ArchState;
use crate::isa::csr;
use crate::isa::csr::CsrAddr;
use crate::isa::fp::{FpFlags, FpFormat, Rounding, RoundingMode};
use crate::isa::op::{AluOp, CsrOp, OperandFormat, SystemOp, VectorOp};
use crate::isa::privileged::{PrivilegeMode, Trap};
use crate::trace_csr;

const JALR_ALIGNMENT_MASK: u64 = !1;

/// The ALU's two main operands, selected by the decoded sources.
pub const fn operands(inst: &Inst) -> (u64, u64) {
    let op_a = match inst.ctrl.a_src {
        OpASrc::Reg1 => inst.rv1,
        OpASrc::Pc => inst.pc.val(),
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

/// True when `inst` decodes but cannot execute in the hart's current state:
/// it needs a unit `mstatus` has switched Off, or rounds by `frm` while
/// `frm` holds a reserved mode.
pub const fn illegal_in_current_state(hart: &Hart, inst: &Inst) -> bool {
    unit_disabled(hart, inst) || rounds_by_reserved_frm(hart, inst)
}

/// The rounding mode `fcsr.frm` selects, or `None` when it holds a
/// reserved one.
const fn frm_mode(hart: &Hart) -> Option<RoundingMode> {
    hart.csrs.frm.rounding_mode()
}

/// True when `inst` rounds by `frm`, as every vector FP instruction does,
/// and `frm` holds a reserved mode.
const fn rounds_by_reserved_frm(hart: &Hart, inst: &Inst) -> bool {
    let dynamic =
        matches!(inst.ctrl.fp_rm, Some(Rounding::Dynamic)) || is_vector_fp(inst.ctrl.vec_op);
    dynamic && frm_mode(hart).is_none()
}

/// True when `id` needs a unit `mstatus` has switched Off.
///
/// Any vector instruction while VS is Off, and anything touching the FP
/// registers or doing vector floating-point arithmetic while FS is Off.
/// Checked here rather than at decode because `mstatus` writes apply at
/// commit.
const fn unit_disabled(hart: &Hart, inst: &Inst) -> bool {
    let is_vector = !matches!(inst.ctrl.vec_op, VectorOp::None);
    let is_fp = inst.ctrl.touches_fp_registers() || is_vector_fp(inst.ctrl.vec_op);
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

/// The illegal-instruction trap an xRET, WFI or SFENCE.VMA raises at the
/// current privilege level, if any.
pub const fn privileged_op_fault(hart: &Hart, inst: &Inst) -> Option<Trap> {
    let privilege = hart.privilege;
    let illegal = match inst.ctrl.system_op {
        SystemOp::Mret => !matches!(privilege, PrivilegeMode::Machine),
        SystemOp::Sret => match privilege {
            PrivilegeMode::User => true,
            PrivilegeMode::Supervisor => hart.csrs.mstatus & csr::MSTATUS_TSR != 0,
            PrivilegeMode::Machine => false,
        },
        SystemOp::Wfi => match privilege {
            PrivilegeMode::User => true,
            PrivilegeMode::Supervisor => hart.csrs.mstatus & csr::MSTATUS_TW != 0,
            PrivilegeMode::Machine => false,
        },
        SystemOp::SfenceVma => match privilege {
            PrivilegeMode::User => true,
            PrivilegeMode::Supervisor => hart.csrs.mstatus & csr::MSTATUS_TVM != 0,
            PrivilegeMode::Machine => false,
        },
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
    /// A permitted cache-block operation on the block at `rs1`; memory1
    /// evaluates its effect when it translates the block.
    Cbo,
    /// A permitted CSR access, for the backend to perform once every older
    /// CSR write has been applied.
    Csr(CsrRequest),
}

/// What the system instruction `id` does at the current privilege level.
pub fn system_effect(state: &impl ArchState, inst: &Inst) -> SystemEffect {
    if let Some(trap) = privileged_op_fault(state.hart(), inst) {
        return SystemEffect::Trap(trap);
    }
    match inst.ctrl.system_op {
        SystemOp::None | SystemOp::Fence(_) => SystemEffect::NotSystem,
        SystemOp::FenceI | SystemOp::Mret | SystemOp::Sret | SystemOp::Wfi | SystemOp::Pseudo => {
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
                Ok(_) => SystemEffect::Cbo,
                Err(trap) => SystemEffect::Trap(trap),
            }
        }
        // In SE mode a user program's ecall is a syscall the simulator
        // serves when the instruction retires.
        SystemOp::Ecall
            if state.serves_syscalls() && state.hart().privilege == PrivilegeMode::User =>
        {
            SystemEffect::AtRetire
        }
        SystemOp::Ecall => SystemEffect::Trap(ecall_trap(state.hart())),
        SystemOp::Csr => match csr_access_permitted(state, inst) {
            Ok(()) => SystemEffect::Csr(CsrRequest::of(inst)),
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

/// A permitted CSR instruction's operation, performed where the backend
/// has every older CSR write applied.
#[derive(Clone, Copy, Debug)]
pub struct CsrRequest {
    /// The instruction's PC, for the trace.
    pub pc: VirtAddr,
    /// The CSR.
    pub addr: CsrAddr,
    /// The form of access.
    pub op: CsrOp,
    /// The source operand: `rs1`'s value or the immediate.
    pub src: u64,
    /// Whether the form writes the CSR, which its encoding decides.
    pub writes: bool,
}

impl CsrRequest {
    /// The operation `inst` requests.
    #[must_use]
    pub const fn of(inst: &Inst) -> Self {
        let op = inst.ctrl.csr_op;
        let src = match op {
            CsrOp::Rwi | CsrOp::Rsi | CsrOp::Rci => inst.rs1.as_u8() as u64,
            _ => inst.rv1,
        };
        let writes = csr_op_writes(op, inst.rs1.as_u8());
        Self { pc: inst.pc, addr: inst.ctrl.csr_addr, op, src, writes }
    }
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

/// Checks the CSR instruction `inst` against the current privilege and
/// counter enables.
///
/// # Errors
///
/// The illegal-instruction trap when the access is not permitted.
pub fn csr_access_permitted(state: &impl ArchState, inst: &Inst) -> Result<(), Trap> {
    let addr = inst.ctrl.csr_addr;
    let writes = CsrRequest::of(inst).writes;
    let privilege = state.hart().privilege;

    let satp_trapped = addr == csr::SATP
        && matches!(privilege, PrivilegeMode::Supervisor)
        && state.hart().csrs.mstatus & csr::MSTATUS_TVM != 0;
    if satp_trapped
        || csr_unit_disabled(state, addr)
        || counter_access_denied(state, inst)
        || !state.hart().is_valid_csr(addr)
        || u32::from(privilege.to_u8()) < addr.privilege_level() as u32
        || (addr.is_read_only() && writes)
    {
        return Err(Trap::IllegalInstruction(inst.bits));
    }
    Ok(())
}

/// Reads the CSR `request` names and computes the write it makes.
pub fn perform_csr_access(state: &impl ArchState, request: &CsrRequest) -> CsrAccess {
    let CsrRequest { pc, addr, op, src, writes } = *request;
    let old = state.csr_read(addr);
    let base = state.csr_read_for_update(addr);
    let new = match op {
        CsrOp::Rw | CsrOp::Rwi => src,
        CsrOp::Rs | CsrOp::Rsi => base | src,
        CsrOp::Rc | CsrOp::Rci => base & !src,
        CsrOp::None => old,
    };
    trace_csr!(state.tracing();
        op        = "write-deferred",
        pc        = %crate::common::trace::Hex(pc.val()),
        csr_addr  = %crate::common::trace::Hex32(addr.as_u32()),
        csr_op    = ?op,
        old_val   = %crate::common::trace::Hex(old),
        new_val   = %crate::common::trace::Hex(new),
        writes,
        "CSR access"
    );
    let update = writes.then_some(CsrWrite { addr, old, new });
    CsrAccess { old, update }
}

/// Whether the CSR form writes, given its `rs1` field: CSRRS/CSRRC with
/// rs1=x0 and CSRRSI/CSRRCI with uimm=0 only read. A non-x0 `rs1` writes
/// even when it holds zero (Zicsr).
const fn csr_op_writes(op: CsrOp, rs1_field: u8) -> bool {
    match op {
        CsrOp::Rw | CsrOp::Rwi => true,
        CsrOp::Rs | CsrOp::Rc | CsrOp::Rsi | CsrOp::Rci => rs1_field != 0,
        CsrOp::None => false,
    }
}

/// True when `mcounteren`/`scounteren` hide the user-level counter `inst`
/// reads (`cycle`, `time`, `instret` or an `hpmcounter`) from the current
/// privilege level; counter `n` is enabled by bit `n`.
fn counter_access_denied(state: &impl ArchState, inst: &Inst) -> bool {
    let raw = inst.ctrl.csr_addr.as_u32();
    if !(csr::CYCLE.as_u32()..=csr::HPMCOUNTER31.as_u32()).contains(&raw) {
        return false;
    }
    let mask = 1u64 << (raw - csr::CYCLE.as_u32());
    let csrs = &state.hart().csrs;
    match state.hart().privilege {
        PrivilegeMode::Supervisor => csrs.mcounteren & mask == 0,
        PrivilegeMode::User => csrs.mcounteren & mask == 0 || csrs.scounteren & mask == 0,
        PrivilegeMode::Machine => false,
    }
}

/// Evaluates `id`'s ALU or FPU operation and returns `(result, fp_flags)`.
pub fn evaluate(state: &impl ArchState, inst: &Inst, op_a: u64, op_b: u64) -> (u64, FpFlags) {
    let fp_rm = match inst.ctrl.fp_rm {
        Some(Rounding::Static(mode)) => Some(mode),
        Some(Rounding::Dynamic) => frm_mode(state.hart()),
        None => None,
    };
    compute_alu(inst.ctrl.alu, op_a, op_b, inst.rv3, inst.ctrl.format, fp_rm)
}

/// Checks a taken branch or jump target's alignment: four bytes without
/// the C extension (IALIGN=32), two with it.
///
/// # Errors
///
/// The instruction-address-misaligned trap for a misaligned `target`.
pub const fn check_target_alignment(hart: &Hart, target: VirtAddr) -> Result<(), Trap> {
    if target.val() & crate::arch::csr::ialign_low_bits(hart.csrs.misa) == 0 {
        return Ok(());
    }
    Err(Trap::InstructionAddressMisaligned(target.val()))
}

/// Where the JAL or JALR `id` jumps.
#[must_use]
pub fn jump_target(inst: &Inst) -> VirtAddr {
    if inst.ctrl.control_flow == ControlFlow::Jump(JumpKind::Jalr) {
        VirtAddr::new(inst.rv1.wrapping_add(inst.imm as u64) & JALR_ALIGNMENT_MASK)
    } else {
        inst.pc.wrapping_add(inst.imm as u64)
    }
}

/// Runs `convert` with the host FPU set to `rm` and returns its result with
/// the IEEE flags the host raised, so conversions report INEXACT/OVERFLOW.
fn on_host_fpu(rm: RoundingMode, convert: impl FnOnce() -> u64) -> (u64, FpFlags) {
    let saved = set_host_round_mode(rm);
    clear_host_fp_flags();
    let value = std::hint::black_box(convert());
    let flags = read_host_fp_flags();
    restore_host_round_mode(saved);
    (value, flags)
}

/// Runs a rounding conversion on the host FPU in `rm`, and under RMM, which
/// the host rounds as nearest-even, rounds `exact` (the value converted) in
/// software instead when the host's result was inexact. `format` is the
/// destination format.
fn convert_rounded(
    rm: RoundingMode,
    format: FpFormat,
    convert: impl FnOnce() -> u64,
    exact: impl FnOnce() -> Exact,
) -> (u64, FpFlags) {
    use crate::exec::compute::fpu::nan_handling::box_f32_canon;
    let (value, flags) = on_host_fpu(rm, convert);
    match exact::rmm_correction(rm, flags, format, || Some(exact())) {
        Some((bits, flags)) => {
            let boxed = if format == FpFormat::Single {
                box_f32_canon(f32::from_bits(bits as u32))
            } else {
                bits
            };
            (boxed, flags)
        }
        None => (value, flags),
    }
}

/// Computes the ALU/FPU result and returns `(result, fp_flags)`; the flags
/// are empty for everything but floating-point arithmetic.
pub fn compute_alu(
    alu_op: AluOp,
    op_a: u64,
    op_b: u64,
    op_c: u64,
    format: OperandFormat,
    fp_rm: Option<RoundingMode>,
) -> (u64, FpFlags) {
    use crate::exec::compute::fpu::half::{box_f16, f16_to_f32, unbox_f16};
    use crate::exec::compute::fpu::nan_handling::{
        box_f32_canon, canonicalize_f64_bits, unbox_f32,
    };
    use std::hint::black_box;

    let rm = fp_rm.unwrap_or(RoundingMode::Rne);
    let fp = format.fp_format();
    let half = fp == FpFormat::Half;
    let single = fp == FpFormat::Single;
    match alu_op {
        AluOp::FCvtSW if !half => convert_rounded(
            rm,
            fp,
            || {
                let v = black_box(op_a as i32);
                if single { box_f32(v as f32) } else { f64::from(v).to_bits() }
            },
            || Exact::of_integer(i128::from(op_a as i32)),
        ),
        AluOp::FCvtSWU if !half => convert_rounded(
            rm,
            fp,
            || {
                let v = black_box(op_a as u32);
                if single { box_f32(v as f32) } else { f64::from(v).to_bits() }
            },
            || Exact::of_unsigned(u128::from(op_a as u32)),
        ),
        AluOp::FCvtSL if !half => convert_rounded(
            rm,
            fp,
            || {
                let v = black_box(op_a as i64);
                if single { box_f32(v as f32) } else { (v as f64).to_bits() }
            },
            || Exact::of_integer(i128::from(op_a as i64)),
        ),
        AluOp::FCvtSLU if !half => convert_rounded(
            rm,
            fp,
            || {
                let v = black_box(op_a);
                if single { box_f32(v as f32) } else { (v as f64).to_bits() }
            },
            || Exact::of_unsigned(u128::from(op_a)),
        ),
        AluOp::FCvtSD if !half => convert_rounded(
            rm,
            FpFormat::Single,
            || box_f32_canon(black_box(f64::from_bits(op_a)) as f32),
            || Exact::of_f64(f64::from_bits(op_a)),
        ),
        AluOp::FCvtDS if !half => {
            on_host_fpu(rm, || canonicalize_f64_bits(f64::from(black_box(unbox_f32(op_a)))))
        }
        AluOp::FCvtSH if !half => on_host_fpu(rm, || box_f32_canon(f16_to_f32(unbox_f16(op_a)))),
        AluOp::FCvtDH if !half => on_host_fpu(rm, || {
            canonicalize_f64_bits(f64::from(black_box(f16_to_f32(unbox_f16(op_a)))))
        }),
        AluOp::FMvToF => {
            let value = match fp {
                FpFormat::Half => box_f16(op_a as u16),
                FpFormat::Single => box_f32(f32::from_bits(op_a as u32)),
                FpFormat::Double => op_a,
            };
            (value, FpFlags::NONE)
        }
        _ if is_fp_op(alu_op) => {
            let (result, fp_flags) = fpu::execute_full_rm(alu_op, op_a, op_b, op_c, fp, rm);
            (result, fp_flags)
        }
        _ => (alu::execute(alu_op, op_a, op_b, format.int_width()), FpFlags::NONE),
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

        let (res, flags) =
            compute_alu(AluOp::FCvtSW, 1, 0, 0, OperandFormat::Fp(FpFormat::Double), rne);
        assert_eq!(res, (1.0f64).to_bits());
        assert_eq!(flags, FpFlags::NONE);

        let (res, flags) =
            compute_alu(AluOp::FCvtSW, 1, 0, 0, OperandFormat::Fp(FpFormat::Single), rne);
        assert_eq!(res, 0xFFFF_FFFF_0000_0000 | u64::from((1.0f32).to_bits()));
        assert_eq!(flags, FpFlags::NONE);
    }

    #[test]
    fn move_to_float_passes_the_bits_through() {
        let (res, flags) = compute_alu(
            AluOp::FMvToF,
            42,
            0,
            0,
            OperandFormat::Fp(FpFormat::Double),
            Some(RoundingMode::Rne),
        );

        assert_eq!(res, 42);
        assert_eq!(flags, FpFlags::NONE);
    }

    #[test]
    fn inexact_conversion_raises_the_inexact_flag() {
        let (_, flags) = compute_alu(
            AluOp::FCvtSL,
            (1u64 << 60) + 1,
            0,
            0,
            OperandFormat::Fp(FpFormat::Single),
            Some(RoundingMode::Rne),
        );

        assert!(!flags.is_empty());
    }
}
