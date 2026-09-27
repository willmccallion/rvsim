//! Execute-stage logic both backends share.
//!
//! Operand selection, ALU/FPU evaluation, branch and jump resolution, and the
//! privilege and CSR checks that decide whether a system instruction faults.
//! Each backend decides for itself which instructions redirect fetch.

use crate::common::CsrAddr;
use crate::common::error::{ExceptionStage, Trap};
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::pipeline::latches::{ExMem1Entry, RenameIssueEntry};
use crate::core::pipeline::rob::{BpOutcome, CsrUpdate, Rob};
use crate::core::pipeline::signals::{AluOp, CsrOp, OpASrc, OpBSrc, SystemOp, VectorOp};
use crate::core::pipeline::squash::{BranchRepair, Redirect};
use crate::core::units::alu::Alu;
use crate::core::units::fpu::Fpu;
use crate::core::units::fpu::rounding_modes::RoundingMode;
use crate::core::units::vpu::fpu::is_vec_fp;
use crate::isa::abi;
use crate::isa::rv64i::{funct3, opcodes};
use crate::sim::StageCtx;
use crate::{trace_branch, trace_csr, trace_trap};

const FUNCT3_SHIFT: u32 = 12;
const FUNCT3_MASK: u32 = 0x7;
const JALR_ALIGNMENT_MASK: u64 = !1;
const MSTATUS_TVM_BIT: u32 = 20;
const MSTATUS_TW_BIT: u32 = 21;
const MSTATUS_TSR_BIT: u32 = 22;

/// The instruction after `id` in program order.
pub const fn next_pc(id: &RenameIssueEntry) -> u64 {
    id.pc.wrapping_add(id.inst_size.as_u64())
}

/// The ALU's two main operands, selected by the decoded sources.
pub const fn operands(id: &RenameIssueEntry) -> (u64, u64) {
    let op_a = match id.ctrl.a_src {
        OpASrc::Reg1 => id.rv1,
        OpASrc::Pc => id.pc,
        OpASrc::Zero => 0,
    };
    let op_b = match id.ctrl.b_src {
        OpBSrc::Reg2 => id.rv2,
        OpBSrc::Imm => id.imm as u64,
        OpBSrc::Zero => 0,
    };
    (op_a, op_b)
}

/// The result of an instruction that raised `trap` in `stage`. The trap
/// travels with it: writeback records it on the ROB entry and commit takes
/// it, flushing everything younger, as a real core does.
pub fn fault(
    state: &StageCtx<'_>,
    id: &RenameIssueEntry,
    trap: Trap,
    stage: ExceptionStage,
) -> ExMem1Entry {
    trace_trap!(state.trace_trap_enabled(&trap);
        event   = "fault",
        stage   = ?stage,
        pc      = %crate::trace::Hex(id.pc),
        rob_tag = id.rob_tag.0,
        trap    = ?trap,
        "EX: instruction faulted"
    );
    ExMem1Entry {
        trap: Some(trap),
        exception_stage: Some(stage),
        ..ExMem1Entry::from_issue(id, 0, 0)
    }
}

/// The result of an instruction that arrived carrying a trap from an
/// earlier stage.
pub fn propagate_trap(state: &StageCtx<'_>, id: &RenameIssueEntry, trap: Trap) -> ExMem1Entry {
    fault(state, id, trap, id.exception_stage.unwrap_or(ExceptionStage::Execute))
}

const fn fs_off(state: &StageCtx<'_>) -> bool {
    state.hart().csrs.mstatus & csr::MSTATUS_FS == 0
}

const fn vs_off(state: &StageCtx<'_>) -> bool {
    state.hart().csrs.mstatus & csr::MSTATUS_VS == 0
}

/// True when `id` needs a unit `mstatus` has switched Off.
///
/// Any vector instruction while VS is Off, and anything touching the FP
/// registers or doing vector floating-point arithmetic while FS is Off.
/// Checked here rather than at decode because `mstatus` writes apply at
/// commit.
pub const fn unit_disabled(state: &StageCtx<'_>, id: &RenameIssueEntry) -> bool {
    let is_vector = !matches!(id.ctrl.vec_op, VectorOp::None);
    let is_fp = id.ctrl.fp_reg_write
        || id.ctrl.rs1_fp
        || id.ctrl.rs2_fp
        || id.ctrl.rs3_fp
        || is_vector_fp(id.ctrl.vec_op);
    (is_vector && vs_off(state)) || (is_fp && fs_off(state))
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
fn csr_unit_disabled(state: &StageCtx<'_>, addr: CsrAddr) -> bool {
    let fp_csr = addr == csr::FFLAGS || addr == csr::FRM || addr == csr::FCSR;
    let vector_csr = addr == csr::VSTART
        || addr == csr::VXSAT
        || addr == csr::VXRM
        || addr == csr::VCSR
        || addr == csr::VL
        || addr == csr::VTYPE
        || addr == csr::VLENB;
    (fp_csr && fs_off(state)) || (vector_csr && vs_off(state))
}

const fn mstatus_bit(state: &StageCtx<'_>, bit: u32) -> bool {
    (state.hart().csrs.mstatus >> bit) & 1 != 0
}

/// The illegal-instruction trap an xRET, WFI or SFENCE.VMA raises at the
/// current privilege level, if any.
pub const fn privileged_op_fault(state: &StageCtx<'_>, id: &RenameIssueEntry) -> Option<Trap> {
    let privilege = state.hart().privilege;
    let illegal = match id.ctrl.system_op {
        SystemOp::Mret => !matches!(privilege, PrivilegeMode::Machine),
        SystemOp::Sret => match privilege {
            PrivilegeMode::User => true,
            PrivilegeMode::Supervisor => mstatus_bit(state, MSTATUS_TSR_BIT),
            PrivilegeMode::Machine => false,
        },
        SystemOp::Wfi => match privilege {
            PrivilegeMode::User => true,
            PrivilegeMode::Supervisor => mstatus_bit(state, MSTATUS_TW_BIT),
            PrivilegeMode::Machine => false,
        },
        SystemOp::SfenceVma => {
            matches!(privilege, PrivilegeMode::Supervisor) && mstatus_bit(state, MSTATUS_TVM_BIT)
        }
        _ => false,
    };
    if illegal { Some(Trap::IllegalInstruction(id.inst)) } else { None }
}

/// The environment-call trap for the current privilege level.
pub const fn ecall_trap(state: &StageCtx<'_>) -> Trap {
    match state.hart().privilege {
        PrivilegeMode::User => Trap::EnvironmentCallFromUMode,
        PrivilegeMode::Supervisor => Trap::EnvironmentCallFromSMode,
        PrivilegeMode::Machine => Trap::EnvironmentCallFromMMode,
    }
}

/// What a permitted CSR instruction reads, and the write it defers to commit.
#[derive(Clone, Debug)]
pub struct CsrAccess {
    /// The value written to `rd`.
    pub old: u64,
    /// The write commit applies; `None` for forms that must not write.
    pub update: Option<CsrUpdate>,
}

/// Checks the CSR instruction `id` against the current privilege and
/// counter enables, and computes the value it reads and the write it makes.
///
/// # Errors
///
/// The illegal-instruction trap when the access is not permitted.
pub fn csr_access(state: &StageCtx<'_>, id: &RenameIssueEntry) -> Result<CsrAccess, Trap> {
    let addr = id.ctrl.csr_addr;
    let illegal = Trap::IllegalInstruction(id.inst);
    let writes = csr_op_writes(id);
    let privilege = state.hart().privilege;

    let satp_trapped = addr == csr::SATP
        && matches!(privilege, PrivilegeMode::Supervisor)
        && mstatus_bit(state, MSTATUS_TVM_BIT);
    if satp_trapped
        || csr_unit_disabled(state, addr)
        || counter_access_denied(state, id)
        || !state.is_valid_csr(addr)
        || u32::from(privilege.to_u8()) < addr.privilege_level() as u32
        || (addr.is_read_only() && writes)
    {
        return Err(illegal);
    }

    let old = state.csr_read(addr);
    let base = state.csr_read_for_update(addr);
    let src = match id.ctrl.csr_op {
        CsrOp::Rwi | CsrOp::Rsi | CsrOp::Rci => u64::from(id.rs1.as_u8() & 0x1f),
        _ => id.rv1,
    };
    let new = match id.ctrl.csr_op {
        CsrOp::Rw | CsrOp::Rwi => src,
        CsrOp::Rs | CsrOp::Rsi => base | src,
        CsrOp::Rc | CsrOp::Rci => base & !src,
        CsrOp::None => old,
    };
    trace_csr!(state.config.general.trace_instructions;
        op        = "write-deferred",
        pc        = %crate::trace::Hex(id.pc),
        rob_tag   = id.rob_tag.0,
        csr_addr  = %crate::trace::Hex32(addr.as_u32()),
        csr_op    = ?id.ctrl.csr_op,
        old_val   = %crate::trace::Hex(old),
        new_val   = %crate::trace::Hex(new),
        writes,
        "EX: CSR access"
    );
    let update = writes.then_some(CsrUpdate { addr, old_val: old, new_val: new, applied: false });
    Ok(CsrAccess { old, update })
}

/// Whether the CSR form writes: CSRRS/CSRRC with rs1=x0 and CSRRSI/CSRRCI
/// with uimm=0 only read.
const fn csr_op_writes(id: &RenameIssueEntry) -> bool {
    match id.ctrl.csr_op {
        CsrOp::Rw | CsrOp::Rwi => true,
        CsrOp::Rs | CsrOp::Rc => !id.rs1.is_zero(),
        CsrOp::Rsi | CsrOp::Rci => (id.rs1.as_u8() & 0x1f) != 0,
        CsrOp::None => false,
    }
}

/// True when `mcounteren`/`scounteren` hide the CYCLE, TIME or INSTRET
/// counter `id` reads from the current privilege level.
fn counter_access_denied(state: &StageCtx<'_>, id: &RenameIssueEntry) -> bool {
    let addr = id.ctrl.csr_addr;
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
pub fn evaluate(state: &StageCtx<'_>, id: &RenameIssueEntry, op_a: u64, op_b: u64) -> (u64, u8) {
    let fp_rm = id.ctrl.fp_rm.or_else(|| RoundingMode::from_bits(state.hart().csrs.frm as u8));
    compute_alu(id.ctrl.alu, op_a, op_b, id.rv3, id.ctrl.is_f16, id.ctrl.is_rv32, fp_rm)
}

/// Resolves a conditional branch against its prediction, files the outcome
/// for the predictor to learn from at commit, and returns the redirect a
/// misprediction needs.
pub fn resolve_branch(
    state: &mut StageCtx<'_>,
    rob: &mut Rob,
    id: &RenameIssueEntry,
    op_a: u64,
    op_b: u64,
) -> Option<Redirect> {
    let taken = match (id.inst >> FUNCT3_SHIFT) & FUNCT3_MASK {
        funct3::BEQ => op_a == op_b,
        funct3::BNE => op_a != op_b,
        funct3::BLT => (op_a as i64) < (op_b as i64),
        funct3::BGE => (op_a as i64) >= (op_b as i64),
        funct3::BLTU => op_a < op_b,
        funct3::BGEU => op_a >= op_b,
        _ => false,
    };
    let actual_target = id.pc.wrapping_add(id.imm as u64);
    let fallthrough = next_pc(id);
    let predicted_next_pc = if id.pred_taken { id.pred_target } else { fallthrough };
    let actual_next_pc = if taken { actual_target } else { fallthrough };
    let mispredicted = predicted_next_pc != actual_next_pc;

    rob.set_bp_update(
        id.rob_tag,
        BpOutcome { taken, mispredicted },
        taken.then_some(actual_target),
    );
    trace_branch!(state.config.general.trace_instructions;
        event          = "resolve",
        pc             = %crate::trace::Hex(id.pc),
        rob_tag        = id.rob_tag.0,
        pred_taken     = id.pred_taken,
        pred_target    = %crate::trace::Hex(predicted_next_pc),
        actual_taken   = taken,
        actual_target  = %crate::trace::Hex(actual_next_pc),
        mispredicted,
        "EX: branch resolved"
    );
    let repair = BranchRepair { seq: id.seq, taken, target: actual_target };
    count_prediction(state, mispredicted).then(|| Redirect::mispredict(actual_next_pc, repair))
}

/// Resolves a JAL or JALR against its predicted target and returns the
/// redirect a misprediction needs. Jumps do not train the direction tables.
pub fn resolve_jump(
    state: &mut StageCtx<'_>,
    rob: &mut Rob,
    id: &RenameIssueEntry,
) -> Option<Redirect> {
    use crate::common::constants::OPCODE_MASK;
    let is_jalr = (id.inst & OPCODE_MASK) == opcodes::OP_JALR;
    let actual_target = if is_jalr {
        id.rv1.wrapping_add(id.imm as u64) & JALR_ALIGNMENT_MASK
    } else {
        id.pc.wrapping_add(id.imm as u64)
    };
    let predicted_target = if id.pred_taken { id.pred_target } else { next_pc(id) };
    let mispredicted = actual_target != predicted_target;

    rob.set_bp_target(id.rob_tag, actual_target);
    if is_jalr {
        state.core_mut().branch_predictor.update_btb(id.pc, actual_target);
    }
    let rd_link = id.rd == abi::REG_RA || id.rd == abi::REG_T0;
    let rs1_link = is_jalr && (id.rs1 == abi::REG_RA || id.rs1 == abi::REG_T0);
    trace_branch!(state.config.general.trace_instructions;
        event          = "resolve",
        pc             = %crate::trace::Hex(id.pc),
        rob_tag        = id.rob_tag.0,
        bp_type        = if rs1_link && !rd_link { "JALR/RAS" } else if rd_link { "JAL/call" } else { "JAL/JALR" },
        pred_taken     = id.pred_taken,
        pred_target    = %crate::trace::Hex(predicted_target),
        actual_taken   = true,
        actual_target  = %crate::trace::Hex(actual_target),
        mispredicted,
        "EX: jump resolved"
    );
    let repair = BranchRepair { seq: id.seq, taken: true, target: actual_target };
    count_prediction(state, mispredicted).then(|| Redirect::mispredict(actual_target, repair))
}

/// Counts a resolved prediction and passes `mispredicted` through.
fn count_prediction(state: &mut StageCtx<'_>, mispredicted: bool) -> bool {
    let paths = &state.core().stat_paths.bp;
    let path = if mispredicted { paths.spec_mispredicts } else { paths.spec_hits };
    state.counter(path).inc();
    mispredicted
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
