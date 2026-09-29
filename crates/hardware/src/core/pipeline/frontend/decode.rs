//! Decode Stage: decodes the fetched instructions, checks fetch's
//! predictions against their encodings, and reads their source registers.
//!
//! This stage reads from the Fetch2->Decode latch and writes to the
//! Decode->Rename latch. Decoding itself is [`crate::core::exec::decode`];
//! here the in-order backend also stops a bundle at an intra-bundle
//! register dependency.

use crate::common::RegIdx;
use crate::common::error::ExceptionStage;
use crate::core::exec::decode::{DecodedInst, decode_inst};
use crate::core::pipeline::latches::{IdExEntry, IfIdEntry};
use crate::core::units::bru::ControlInst;
use crate::core::units::vpu::types::VectorConfig;
use crate::isa::instruction::InstructionBits;
use crate::sim::StageCtx;

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
    has_register_renaming: bool,
    vector: VectorConfig,
) -> DecodeOutcome {
    let mut consumed_count = 0;
    let mut ended_at_vsetvl = false;
    let mut redirect = None;
    let mut bundle_writes: Vec<(RegIdx, bool)> = Vec::with_capacity(state.config.pipeline.width);

    for if_entry in input.iter().take(state.config.pipeline.decode_width()) {
        if let Some(trap) = &if_entry.trap {
            output.push(IdExEntry {
                pc: if_entry.pc,
                inst: if_entry.inst,
                inst_size: if_entry.inst_size,
                trap: Some(trap.clone()),
                exception_stage: if_entry.exception_stage,
                ..Default::default()
            });
            consumed_count += 1;
            break;
        }

        let inst = if_entry.inst;

        let DecodedInst { fields: d, ctrl, trap } = decode_inst(inst, if_entry.pc, vector.vtype);
        let ex_stage = trap.as_ref().map(|_| ExceptionStage::Decode);

        // O3 rename resolves RAW hazards via PRF, so this check applies only in-order.
        let rs3_idx = inst.rs3();
        if !has_register_renaming {
            let hazard = ((!d.rs1.is_zero() || ctrl.rs1_fp)
                && bundle_writes.contains(&(d.rs1, ctrl.rs1_fp)))
                || ((!d.rs2.is_zero() || ctrl.rs2_fp)
                    && bundle_writes.contains(&(d.rs2, ctrl.rs2_fp)))
                || (ctrl.rs3_fp && bundle_writes.contains(&(rs3_idx, true)));

            if hazard {
                break;
            }
        }

        if ctrl.reg_write && !d.rd.is_zero() {
            bundle_writes.push((d.rd, false));
        }
        if ctrl.fp_reg_write {
            bundle_writes.push((d.rd, true));
        }

        let rv1 = if ctrl.rs1_fp {
            state.hart().regs.read_f(d.rs1)
        } else {
            state.hart().regs.read(d.rs1)
        };
        let rv2 = if ctrl.rs2_fp {
            state.hart().regs.read_f(d.rs2)
        } else {
            state.hart().regs.read(d.rs2)
        };
        let rv3 = if ctrl.rs3_fp { state.hart().regs.read_f(rs3_idx) } else { 0 };

        let has_trap = trap.is_some();

        let mut decoded = IdExEntry {
            pc: if_entry.pc,
            inst,
            inst_size: if_entry.inst_size,
            rs1: d.rs1,
            rs2: d.rs2,
            rs3: rs3_idx,
            rd: d.rd,
            imm: d.imm,
            rv1,
            rv2,
            rv3,
            ctrl,
            trap,
            exception_stage: ex_stage,
            pred_taken: if_entry.pred_taken,
            pred_target: if_entry.pred_target,
            seq: if_entry.seq,
        };
        if !has_trap {
            redirect = check_fetch_prediction(state, &mut decoded);
        }
        output.push(decoded);

        consumed_count += 1;

        if has_trap || redirect.is_some() {
            break;
        }
        if ctrl.vec_op.is_config() {
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
    pub redirect: Option<u64>,
}

/// Checks the prediction fetch made for `entry` from the BTB alone against
/// what the encoding shows, as a real front end's decode does, and returns
/// where fetch must restart when the path it fetched is wrong: a control
/// instruction the BTB missed is predicted here, a stale BTB target is
/// corrected, and a BTB entry for an instruction that is not a control
/// instruction is dropped.
fn check_fetch_prediction(state: &mut StageCtx<'_>, entry: &mut IdExEntry) -> Option<u64> {
    let size = entry.inst_size.as_u64();
    let fallthrough = entry.pc.wrapping_add(size);
    let fetched_next = if entry.pred_taken { entry.pred_target } else { fallthrough };
    let control = ControlInst::from_encoding(entry.pc, size, entry.inst);
    let predictor = &mut state.core_mut().branch_predictor;
    let predicted_by_fetch = predictor.is_predicted(entry.seq);

    let redirect = match (control, predicted_by_fetch) {
        (None, false) => None,
        (None, true) => {
            predictor.forget(entry.seq, entry.pc);
            entry.pred_taken = false;
            entry.pred_target = 0;
            Some(fallthrough)
        }
        (Some(control), true) => {
            let direct_target = match control {
                ControlInst::Branch { target } | ControlInst::Jump { target, .. } => Some(target),
                ControlInst::IndirectJump { .. } => None,
            };
            let must_take = matches!(control, ControlInst::Jump { .. });
            let fixed_target = direct_target.filter(|&target| {
                (entry.pred_taken && entry.pred_target != target)
                    || (must_take && !entry.pred_taken)
            });
            fixed_target.inspect(|&target| {
                predictor.correct_target(entry.seq, target);
                entry.pred_taken = true;
                entry.pred_target = target;
            })
        }
        (Some(control), false) => {
            let (target, squashed_younger) = predictor.discover(entry.seq, entry.pc, control);
            entry.pred_taken = target.is_some();
            entry.pred_target = target.unwrap_or(0);
            let next = target.unwrap_or(fallthrough);
            (squashed_younger || next != fetched_next).then_some(next)
        }
    };
    if redirect.is_some() {
        state.counter(state.core().stat_paths.bp.decode_redirects).inc();
    }
    redirect
}
