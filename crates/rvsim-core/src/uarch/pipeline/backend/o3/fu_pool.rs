//! Functional Unit Pool for the O3 backend.
//!
//! Models pipelined and non-pipelined execution units with configurable
//! latencies. Structural hazards are enforced: an instruction cannot issue
//! if all units of the required type are busy.
//!
//! Default latencies are Skylake-class values matching real hardware.

use crate::config::FuConfig;
use crate::exec::signals::{ControlFlow, ControlSignals};
use crate::isa::op::{
    AluOpGroup, FpReduceOp, FpWidenReduceOp, PermuteOp, ReduceOp, VecOpGroup, VectorOp,
};
use crate::uarch::pipeline::rob::RobTag;

/// Identifies which type of functional unit an instruction uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FuType {
    /// Integer ALU: add, sub, logic, shift, compare, set-less-than.
    IntAlu = 0,
    /// Integer multiplier: mul, mulh, mulhsu, mulhu.
    IntMul = 1,
    /// Integer divider: div, divu, rem, remu. Non-pipelined.
    IntDiv = 2,
    /// FP adder: fadd, fsub, fmin, fmax, fcmp, fcvt.
    FpAdd = 3,
    /// FP multiplier: fmul.
    FpMul = 4,
    /// FP fused multiply-add: fmadd, fmsub, fnmadd, fnmsub.
    FpFma = 5,
    /// FP divider/sqrt: fdiv, fsqrt. Non-pipelined.
    FpDivSqrt = 6,
    /// Branch/jump unit: all conditional branches, jal, jalr.
    Branch = 7,
    /// Memory address calculation for loads and stores.
    Mem = 8,
    /// Vector integer ALU: add/sub/logic/shift/compare/merge/ext.
    VecIntAlu = 9,
    /// Vector integer multiplier: mul/mulh/macc/madd/widening mul.
    VecIntMul = 10,
    /// Vector integer divider: div/rem. Non-pipelined.
    VecIntDiv = 11,
    /// Vector FP ALU: fadd/fsub/fmin/fmax/fcmp/fcvt/fclass/fsgnj.
    VecFpAlu = 12,
    /// Vector FP FMA: fmul/fmadd/fmsub/fnmadd/fnmsub/widening fmul.
    VecFpFma = 13,
    /// Vector FP div/sqrt. Non-pipelined.
    VecFpDivSqrt = 14,
    /// Vector memory: unit/strided/indexed/segment loads and stores.
    VecMem = 15,
    /// Vector permute: slide/gather/compress/vmv/mask-logical/viota/vid.
    VecPermute = 16,
}

/// Number of distinct FU types.
pub const FU_TYPE_COUNT: usize = 17;

impl FuType {
    /// Classify an instruction's FU type from its control signals.
    pub fn classify(ctrl: &ControlSignals) -> Self {
        if let Some(op) = ctrl.vec_op {
            return Self::classify_vec(op);
        }

        if ctrl.mem_read || ctrl.mem_write || ctrl.atomic_op.is_some() {
            return Self::Mem;
        }
        if ctrl.control_flow != ControlFlow::Sequential {
            return Self::Branch;
        }
        match ctrl.alu.group() {
            AluOpGroup::IntAlu => Self::IntAlu,
            AluOpGroup::IntMul => Self::IntMul,
            AluOpGroup::IntDiv => Self::IntDiv,
            AluOpGroup::FpArith => Self::FpAdd,
            AluOpGroup::FpMul => Self::FpMul,
            AluOpGroup::FpDivSqrt => Self::FpDivSqrt,
            AluOpGroup::FpFma => Self::FpFma,
        }
    }

    /// Classify a vector operation into a vector FU type. Crypto runs on
    /// the integer ALU: no separate crypto unit is modelled.
    const fn classify_vec(op: VectorOp) -> Self {
        match op.group() {
            VecOpGroup::Load | VecOpGroup::Store => Self::VecMem,
            VecOpGroup::Config | VecOpGroup::IntAlu | VecOpGroup::Crypto => Self::VecIntAlu,
            VecOpGroup::IntMul => Self::VecIntMul,
            VecOpGroup::IntDiv => Self::VecIntDiv,
            VecOpGroup::FpAlu => Self::VecFpAlu,
            VecOpGroup::FpFma => Self::VecFpFma,
            VecOpGroup::FpDivSqrt => Self::VecFpDivSqrt,
            VecOpGroup::FpSlide | VecOpGroup::Permute => Self::VecPermute,
        }
    }
}

/// A unit [`FuPool::free_unit`] found free, which [`FuPool::acquire`] then
/// occupies. Units are never removed from a pool, so the index stays valid.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct FreeUnit(usize);

/// One instance of a functional unit.
#[derive(Clone, Debug)]
pub struct FuUnit {
    /// The functional unit class.
    pub fu_type: FuType,
    /// Number of cycles from issue to result ready.
    pub latency: u64,
    /// If true, a new instruction can be issued every cycle (pipelined).
    /// If false, the unit is busy for the full latency (non-pipelined).
    pub is_pipelined: bool,
    /// Simulation cycle at which this unit is free again (0 = free now).
    pub busy_until: u64,
    /// The instruction a non-pipelined unit is working on, which frees the
    /// unit if it is squashed.
    pub holder: Option<RobTag>,
}

impl FuUnit {
    /// Returns true if this unit can accept a new instruction at cycle `now`.
    #[inline]
    pub const fn is_free(&self, now: u64) -> bool {
        now >= self.busy_until
    }

    /// Acquire the unit for one instruction issued at cycle `now`.
    /// Returns the cycle at which the result will be ready.
    pub const fn acquire(&mut self, now: u64, holder: RobTag) -> u64 {
        self.acquire_with_latency(now, self.latency, holder)
    }

    /// Acquire the unit with a dynamic latency (for vector ops where latency
    /// depends on VL/lanes). Returns the cycle at which the result will be ready.
    pub const fn acquire_with_latency(&mut self, now: u64, latency: u64, holder: RobTag) -> u64 {
        let complete = now + latency;
        if self.is_pipelined {
            self.busy_until = now + 1;
        } else {
            self.busy_until = complete;
            self.holder = Some(holder);
        }
        complete
    }
}

/// Pool of heterogeneous functional units.
#[derive(Debug)]
pub struct FuPool {
    units: Vec<FuUnit>,
}

impl FuPool {
    /// Create a new pool from the given config.
    pub fn new(config: &FuConfig) -> Self {
        let mut units = Vec::new();

        let add = |units: &mut Vec<FuUnit>, fu_type, count, latency, pipelined| {
            for _ in 0..count {
                units.push(FuUnit {
                    fu_type,
                    latency,
                    is_pipelined: pipelined,
                    busy_until: 0,
                    holder: None,
                });
            }
        };

        add(&mut units, FuType::IntAlu, config.num_int_alu, config.int_alu_latency, true);
        add(&mut units, FuType::IntMul, config.num_int_mul, config.int_mul_latency, true);
        add(&mut units, FuType::IntDiv, config.num_int_div, config.int_div_latency, false);
        add(&mut units, FuType::FpAdd, config.num_fp_add, config.fp_add_latency, true);
        add(&mut units, FuType::FpMul, config.num_fp_mul, config.fp_mul_latency, true);
        add(&mut units, FuType::FpFma, config.num_fp_fma, config.fp_fma_latency, true);
        add(
            &mut units,
            FuType::FpDivSqrt,
            config.num_fp_div_sqrt,
            config.fp_div_sqrt_latency,
            false,
        );
        add(&mut units, FuType::Branch, config.num_branch, config.branch_latency, true);
        add(&mut units, FuType::Mem, config.num_mem, config.mem_latency, true);

        add(
            &mut units,
            FuType::VecIntAlu,
            config.num_vec_int_alu,
            config.vec_int_alu_latency,
            true,
        );
        add(
            &mut units,
            FuType::VecIntMul,
            config.num_vec_int_mul,
            config.vec_int_mul_latency,
            true,
        );
        add(
            &mut units,
            FuType::VecIntDiv,
            config.num_vec_int_div,
            config.vec_int_div_latency,
            false,
        );
        add(&mut units, FuType::VecFpAlu, config.num_vec_fp_alu, config.vec_fp_alu_latency, true);
        add(&mut units, FuType::VecFpFma, config.num_vec_fp_fma, config.vec_fp_fma_latency, true);
        add(
            &mut units,
            FuType::VecFpDivSqrt,
            config.num_vec_fp_div_sqrt,
            config.vec_fp_div_sqrt_latency,
            false,
        );
        add(&mut units, FuType::VecMem, config.num_vec_mem, config.vec_mem_latency, true);
        add(
            &mut units,
            FuType::VecPermute,
            config.num_vec_permute,
            config.vec_permute_latency,
            true,
        );

        // Guarantee at least one of each vector FU; scalar-only configs would deadlock vec ops.
        let d = FuConfig::default();
        let vec_defaults: &[(FuType, u64, bool)] = &[
            (FuType::VecIntAlu, d.vec_int_alu_latency, true),
            (FuType::VecIntMul, d.vec_int_mul_latency, true),
            (FuType::VecIntDiv, d.vec_int_div_latency, false),
            (FuType::VecFpAlu, d.vec_fp_alu_latency, true),
            (FuType::VecFpFma, d.vec_fp_fma_latency, true),
            (FuType::VecFpDivSqrt, d.vec_fp_div_sqrt_latency, false),
            (FuType::VecMem, d.vec_mem_latency, true),
            (FuType::VecPermute, d.vec_permute_latency, true),
        ];
        for &(ft, lat, pipe) in vec_defaults {
            if !units.iter().any(|u| u.fu_type == ft) {
                units.push(FuUnit {
                    fu_type: ft,
                    latency: lat,
                    is_pipelined: pipe,
                    busy_until: 0,
                    holder: None,
                });
            }
        }

        Self { units }
    }

    /// A unit of `fu_type` free at cycle `now`, if there is one.
    pub fn free_unit(&self, fu_type: FuType, now: u64) -> Option<FreeUnit> {
        self.free_units(fu_type, now).next()
    }

    /// Every unit of `fu_type` free at cycle `now`.
    pub fn free_units(&self, fu_type: FuType, now: u64) -> impl Iterator<Item = FreeUnit> + '_ {
        self.units
            .iter()
            .enumerate()
            .filter(move |(_, u)| u.fu_type == fu_type && u.is_free(now))
            .map(|(index, _)| FreeUnit(index))
    }

    /// Occupies `unit` for one instruction issued at cycle `now` and returns
    /// the cycle its result is ready.
    pub fn acquire(&mut self, unit: FreeUnit, now: u64, holder: RobTag) -> u64 {
        self.units[unit.0].acquire(now, holder)
    }

    /// Like [`acquire`](Self::acquire) with a latency the instruction sets
    /// (vector ops, whose latency depends on VL and the lane count).
    pub fn acquire_with_latency(
        &mut self,
        unit: FreeUnit,
        now: u64,
        latency: u64,
        holder: RobTag,
    ) -> u64 {
        self.units[unit.0].acquire_with_latency(now, latency, holder)
    }

    /// Frees each non-pipelined unit whose instruction `squashed` says is
    /// gone, as an iterative divider is killed with its instruction (the
    /// `kill` input of Rocket's and BOOM's `MulDiv`).
    pub fn release_squashed(&mut self, now: u64, squashed: impl Fn(RobTag) -> bool) {
        for unit in &mut self.units {
            if unit.busy_until > now && unit.holder.is_some_and(&squashed) {
                unit.busy_until = now;
                unit.holder = None;
            }
        }
    }

    /// Cycles from `now` until `unit` can take another instruction: one for
    /// a pipelined unit just acquired, its whole latency for one that is not.
    #[must_use]
    pub fn busy_cycles(&self, unit: FreeUnit, now: u64) -> u64 {
        self.units[unit.0].busy_until.saturating_sub(now)
    }

    /// Returns whether the first unit of `fu_type` is pipelined.
    pub fn is_pipelined(&self, fu_type: FuType) -> bool {
        self.units.iter().find(|u| u.fu_type == fu_type).is_none_or(|u| u.is_pipelined)
    }

    /// Cycles a vector arithmetic op on `fu_type` takes for `vl` elements
    /// across `lanes` lanes until its whole result is ready.
    #[must_use]
    pub fn vector_op_latency(
        &self,
        fu_type: FuType,
        ctrl: &ControlSignals,
        vl: usize,
        lanes: usize,
    ) -> u64 {
        use crate::isa::op::VecSrcEncoding;
        use crate::uarch::vector::lane_model;

        let startup = self.startup_latency(fu_type);
        let pipelined = self.is_pipelined(fu_type);
        if let Some(VectorOp::Reduce(op)) = ctrl.vec_op {
            // Ordered FP reductions are sequential; others use the tree model.
            let is_ordered = matches!(
                op,
                ReduceOp::Fp(FpReduceOp::OSum) | ReduceOp::FpWiden(FpWidenReduceOp::OSum)
            );
            return lane_model::compute_reduction_latency(vl, lanes, startup, is_ordered);
        }
        if fu_type != FuType::VecPermute {
            return lane_model::compute_vec_latency(vl, lanes, startup, pipelined);
        }
        let groups = vl.div_ceil(lanes) as u64;
        let latency = match ctrl.vec_op {
            Some(VectorOp::Permute(PermuteOp::Rgather | PermuteOp::RgatherEi16))
                if ctrl.vec_src_encoding == VecSrcEncoding::VV =>
            {
                startup + groups.saturating_mul(2).saturating_sub(1)
            }
            Some(VectorOp::Permute(PermuteOp::Rgather | PermuteOp::RgatherEi16)) => {
                startup + groups.saturating_sub(1)
            }
            Some(VectorOp::Permute(PermuteOp::Compress)) => {
                startup + groups.saturating_mul(2).saturating_sub(1)
            }
            _ => lane_model::compute_vec_latency(vl, lanes, startup, pipelined),
        };
        latency.max(1)
    }

    /// Returns the startup latency (pipeline depth) for the given FU type.
    /// This is the per-unit latency configured at pool creation time.
    #[must_use]
    pub fn startup_latency(&self, fu_type: FuType) -> u64 {
        self.units.iter().find(|u| u.fu_type == fu_type).map_or(1, |u| u.latency)
    }
}

#[cfg(test)]
#[allow(unused_results)]
mod tests {
    use super::*;
    use crate::exec::signals::BranchCond;
    use crate::isa::op::SlideOffset;
    use crate::isa::op::{
        AluOp, ArithOp, DivOp, FmaAddend, FmaSigns, FpBinaryOp, FpOp, IntAluOp, IntOp, IntReduceOp,
        MaskLogicalOp, MaskOp, VecAluOp, VecFpOp, VecLoadOp,
    };

    fn default_pool() -> FuPool {
        FuPool::new(&FuConfig::default())
    }

    #[test]
    fn a_pipelined_unit_is_free_again_the_next_cycle() {
        let mut pool = default_pool();
        assert!(pool.free_unit(FuType::IntAlu, 0).is_some());
        let complete = pool.acquire(pool.free_unit(FuType::IntAlu, 0).unwrap(), 0, RobTag::new(1));
        // Latency = 1, so complete = cycle 1
        assert_eq!(complete, 1);
        // Pipelined: unit is free at cycle 1 (busy_until = 0 + 1 = 1, so is_free at cycle 1)
        assert!(pool.free_unit(FuType::IntAlu, 1).is_some());
        // With 4 int ALUs, even cycle 0 has 3 remaining free after 1 acquired
        assert!(pool.free_unit(FuType::IntAlu, 0).is_some());
    }

    #[test]
    fn a_non_pipelined_unit_stays_busy_for_its_full_latency() {
        let mut pool = default_pool();
        assert!(pool.free_unit(FuType::IntDiv, 0).is_some());
        let complete = pool.acquire(pool.free_unit(FuType::IntDiv, 0).unwrap(), 0, RobTag::new(1));
        // Latency = 35, so complete = cycle 35
        assert_eq!(complete, 35);
        // Non-pipelined: busy_until = 35, NOT free until cycle 35
        assert!(pool.free_unit(FuType::IntDiv, 1).is_none());
        assert!(pool.free_unit(FuType::IntDiv, 34).is_none());
        assert!(pool.free_unit(FuType::IntDiv, 35).is_some());
    }

    #[test]
    fn no_unit_is_free_once_all_of_its_type_are_busy() {
        let mut pool = default_pool();
        // FpDivSqrt has count=1
        pool.acquire(pool.free_unit(FuType::FpDivSqrt, 0).unwrap(), 0, RobTag::new(1));
        // No more FpDivSqrt units available
        assert!(pool.free_unit(FuType::FpDivSqrt, 0).is_none());
    }

    #[test]
    fn an_integer_add_runs_on_the_int_alu() {
        let ctrl =
            ControlSignals { alu: AluOp::Int(IntAluOp::Arith(ArithOp::Add)), ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::IntAlu);
    }

    #[test]
    fn an_integer_divide_runs_on_the_int_divider() {
        let ctrl =
            ControlSignals { alu: AluOp::Int(IntAluOp::Div(DivOp::Div)), ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::IntDiv);
    }

    #[test]
    fn an_fp_fma_runs_on_the_fp_fma_unit() {
        let ctrl =
            ControlSignals { alu: AluOp::Fp(FpOp::Fma(FmaSigns::Macc)), ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::FpFma);
    }

    #[test]
    fn a_conditional_branch_runs_on_the_branch_unit() {
        let ctrl = ControlSignals {
            control_flow: ControlFlow::Branch(BranchCond::Eq),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::Branch);
    }

    #[test]
    fn a_memory_read_runs_on_the_mem_unit() {
        let ctrl = ControlSignals { mem_read: true, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::Mem);
    }

    #[test]
    fn a_vector_integer_add_runs_on_the_vec_int_alu() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Alu(VecAluOp::Int(IntOp::Add))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecIntAlu);
    }

    #[test]
    fn a_vector_integer_multiply_runs_on_the_vec_int_mul_unit() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Alu(VecAluOp::Int(IntOp::Mul))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecIntMul);
    }

    #[test]
    fn a_vector_integer_divide_runs_on_the_vec_int_divider() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Alu(VecAluOp::Int(IntOp::Div))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecIntDiv);
    }

    #[test]
    fn a_vector_fp_add_runs_on_the_vec_fp_alu() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Add))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecFpAlu);
    }

    #[test]
    fn a_vector_fp_fma_runs_on_the_vec_fp_fma_unit() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Fp(VecFpOp::Fma(FmaSigns::Macc, FmaAddend::Vd))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecFpFma);
    }

    #[test]
    fn a_vector_fp_divide_runs_on_the_vec_fp_div_sqrt_unit() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Fp(VecFpOp::Binary(FpBinaryOp::Div))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecFpDivSqrt);
    }

    #[test]
    fn a_vector_load_runs_on_the_vec_mem_unit() {
        let ctrl =
            ControlSignals { vec_op: Some(VectorOp::Load(VecLoadOp::Unit)), ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecMem);
    }

    #[test]
    fn a_vector_slide_up_runs_on_the_vec_permute_unit() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Permute(PermuteOp::SlideUp(SlideOffset::Rs1))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecPermute);
    }

    #[test]
    fn acquire_with_latency_completes_after_the_given_latency() {
        let mut pool = default_pool();
        let complete = pool.acquire_with_latency(
            pool.free_unit(FuType::VecIntAlu, 10).unwrap(),
            10,
            5,
            RobTag::new(1),
        );
        assert_eq!(complete, 15);
        // Pipelined: unit free next cycle
        assert!(pool.free_unit(FuType::VecIntAlu, 11).is_some());
    }

    #[test]
    fn the_default_pool_has_every_vector_unit_type() {
        let pool = default_pool();
        assert!(pool.free_unit(FuType::VecIntAlu, 0).is_some());
        assert!(pool.free_unit(FuType::VecIntMul, 0).is_some());
        assert!(pool.free_unit(FuType::VecIntDiv, 0).is_some());
        assert!(pool.free_unit(FuType::VecFpAlu, 0).is_some());
        assert!(pool.free_unit(FuType::VecFpFma, 0).is_some());
        assert!(pool.free_unit(FuType::VecFpDivSqrt, 0).is_some());
        assert!(pool.free_unit(FuType::VecMem, 0).is_some());
        assert!(pool.free_unit(FuType::VecPermute, 0).is_some());
    }

    #[test]
    fn vector_reductions_run_on_the_int_or_fp_vector_alu() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Reduce(ReduceOp::Int(IntReduceOp::Sum))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecIntAlu);

        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Reduce(ReduceOp::Fp(FpReduceOp::USum))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecFpAlu);
    }

    #[test]
    fn a_mask_logical_op_runs_on_the_vec_permute_unit() {
        let ctrl = ControlSignals {
            vec_op: Some(VectorOp::Mask(MaskOp::Logical(MaskLogicalOp::And))),
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::VecPermute);
    }
}
