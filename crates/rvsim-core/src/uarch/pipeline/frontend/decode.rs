//! Decode Stage: decodes the fetched instructions, checks fetch's
//! predictions against their encodings, and reads their source registers.
//!
//! This stage reads from the Fetch2->Decode latch and writes to the
//! Decode->Rename latch. Decoding itself is [`crate::exec::decode`];
//! here the in-order backend also stops a bundle at an intra-bundle
//! register dependency.

use crate::common::VirtAddr;
use crate::common::trace::{Hex, Hex32};
use crate::exec::decode::requirement::requirement;
use crate::exec::decode::{DecodeFault, DecodedInst, decode_inst};
use crate::exec::inst::Inst;
use crate::exec::signals::ControlSignals;
use crate::exec::state::ArchState;
use crate::isa::instruction::InstructionBits;
use crate::isa::op::SystemOp;
use crate::isa::reg::{self, ArchReg};
use crate::isa::rvv::VectorConfig;
use crate::trace_decode;
use crate::uarch::bpred::ControlInst;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::engine::BundleHazard;
use crate::uarch::pipeline::latches::{IdExEntry, IfIdEntry};
use crate::uarch::pipeline::trace::Stage;

/// Executes the decode stage.
///
/// Consumes Fetch2->Decode entries (`IfIdEntry`) and produces
/// Decode->Rename entries (`IdExEntry`). Vector instructions are decoded
/// under `vector`, the configuration the youngest `vsetvl` ahead of them
/// produced; a `vsetvl` ends the group, and returns `true`, because what
/// follows it needs its result.
pub fn decode_stage(
    state: &mut StageCtx<'_>,
    input: &mut Vec<IfIdEntry>,
    output: &mut Vec<IdExEntry>,
    bundle_hazard: BundleHazard,
    vector: VectorConfig,
) -> DecodeOutcome {
    let mut consumed_count = 0;
    let mut ended_at_vsetvl = false;
    let mut redirect = None;
    let mut bundle_writes: Vec<ArchReg> = Vec::with_capacity(state.config.pipeline.width);

    for if_entry in input.iter().take(state.config.pipeline.decode_width()) {
        if let Some(trap) = &if_entry.trap {
            state.trace_decoded(if_entry.seq, if_entry.bits);
            state.trace_stage(if_entry.seq, Stage::Decode);
            output.push(IdExEntry {
                inst: Inst {
                    pc: if_entry.pc,
                    bits: if_entry.bits,
                    size: if_entry.inst_size,
                    ..Default::default()
                },
                trap: Some(trap.clone()),
                seq: if_entry.seq,
                ..Default::default()
            });
            consumed_count += 1;
            break;
        }

        let inst = if_entry.bits;

        let DecodedInst { fields: mut d, mut ctrl, mut fault } = decode_inst(inst, vector.vtype);
        // An instruction of an extension the hart does not implement is
        // illegal on it, as on hardware without the extension.
        if fault.is_none() && !state.core().isa.permits(requirement(&ctrl, inst)) {
            ctrl = ControlSignals::default();
            fault = Some(DecodeFault::ExtensionNotImplemented);
        }
        if let Some(fault) = fault {
            trace_decode!(state.config.general.trace_instructions;
                event  = "trap",
                pc     = %Hex(if_entry.pc.val()),
                inst   = %Hex32(inst),
                reason = ?fault,
                "instruction decodes to a trap"
            );
        }
        let trap = fault.map(|fault| fault.trap(inst, if_entry.pc));
        // In SE mode a user program's ecall returns its syscall's result in
        // a0, which is its destination so that dependents wait for it.
        if ctrl.system_op == SystemOp::Ecall && state.serves_syscalls() {
            d.rd = reg::REG_A0;
            ctrl.reg_write = true;
        }

        let rs3_idx = inst.rs3();
        if bundle_hazard == BundleHazard::EndsBundle {
            let sources = [Some(ctrl.src1(d.rs1)), Some(ctrl.src2(d.rs2)), ctrl.src3(rs3_idx)];
            if sources.into_iter().flatten().any(|src| bundle_writes.contains(&src)) {
                break;
            }
        }

        if let Some(dest) = ctrl.dest(d.rd) {
            bundle_writes.push(dest);
        }

        let has_trap = trap.is_some();

        let mut decoded = IdExEntry {
            inst: Inst {
                pc: if_entry.pc,
                bits: inst,
                size: if_entry.inst_size,
                rs1: d.rs1,
                rs2: d.rs2,
                rs3: rs3_idx,
                rd: d.rd,
                imm: d.imm,
                // Operands are read at issue.
                rv1: 0,
                rv2: 0,
                rv3: 0,
                ctrl,
            },
            trap,
            predicted_target: if_entry.predicted_target,
            seq: if_entry.seq,
        };
        state.trace_decoded(decoded.seq, if_entry.bits);
        state.trace_stage(decoded.seq, Stage::Decode);
        if !has_trap {
            redirect = check_fetch_prediction(state, &mut decoded);
        }
        output.push(decoded);

        consumed_count += 1;

        if has_trap || redirect.is_some() {
            break;
        }
        if ctrl.is_vector_config() {
            ended_at_vsetvl = true;
            break;
        }
    }

    let _ = input.drain(..consumed_count);
    DecodeOutcome { ended_at_vsetvl, redirect }
}

/// What a cycle of decode found beyond the instructions it decoded.
#[derive(Clone, Copy, Debug, Default)]
pub struct DecodeOutcome {
    /// Decode stopped after a `vsetvl`, whose result later instructions
    /// are decoded under.
    pub ended_at_vsetvl: bool,
    /// Fetch went the wrong way after the last decoded instruction and must
    /// restart here; what was fetched after it is discarded.
    pub redirect: Option<VirtAddr>,
}

/// Checks the prediction fetch made for `entry` from the BTB alone against
/// what the encoding shows, as a real front end's decode does, and returns
/// where fetch must restart when the path it fetched is wrong: a control
/// instruction the BTB missed is predicted here, a stale BTB target is
/// corrected, and a BTB entry for an instruction that is not a control
/// instruction is dropped. Decode overrides fetch only where the path
/// differs; predictions fetch made past an instruction decode predicts are
/// redone as decode reaches them, so the histories stay in program order
/// without refetching a path fetch already took.
fn check_fetch_prediction(state: &mut StageCtx<'_>, entry: &mut IdExEntry) -> Option<VirtAddr> {
    let size = entry.inst.size.as_u64();
    let fallthrough = entry.inst.pc.wrapping_add(size);
    let fetched_next = entry.predicted_target.unwrap_or(fallthrough);
    let control = ControlInst::of(&entry.inst);
    let predictor = &mut state.core_mut().branch_predictor;
    let predicted_by_fetch = predictor.is_predicted(entry.seq);

    let redirect = match (control, predicted_by_fetch) {
        (None, false) if entry.predicted_target.is_none() => None,
        (None, _) => {
            predictor.forget(entry.seq, entry.inst.pc);
            entry.predicted_target = None;
            Some(fallthrough)
        }
        (Some(control), true) => {
            let direct_target = match control {
                ControlInst::Branch { target } | ControlInst::Jump { target, .. } => Some(target),
                ControlInst::IndirectJump { .. } => None,
            };
            let must_take = matches!(control, ControlInst::Jump { .. });
            let fixed_target = direct_target
                .filter(|&target| entry.predicted_target.map_or(must_take, |p| p != target));
            fixed_target.inspect(|&target| {
                predictor.correct_target(entry.seq, target);
                entry.predicted_target = Some(target);
            })
        }
        (Some(control), false) => {
            let target = predictor.discover(entry.seq, entry.inst.pc, control);
            entry.predicted_target = target;
            let next = target.unwrap_or(fallthrough);
            (next != fetched_next).then_some(next)
        }
    };
    if redirect.is_some() {
        state.counter(state.core().stat_paths.bp.decode_redirects).inc();
    }
    redirect
}
