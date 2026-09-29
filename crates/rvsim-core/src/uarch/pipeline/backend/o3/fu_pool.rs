//! Functional Unit Pool for the O3 backend.
//!
//! Models pipelined and non-pipelined execution units with configurable
//! latencies. Structural hazards are enforced: an instruction cannot issue
//! if all units of the required type are busy.
//!
//! Default latencies are Skylake-class values matching real hardware.

use crate::config::FuConfig;
use crate::exec::signals::{ControlFlow, ControlSignals};
use crate::isa::op::{AluOp, VectorOp};

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
    /// Human-readable name for stats output.
    pub const fn name(self) -> &'static str {
        match self {
            Self::IntAlu => "int_alu",
            Self::IntMul => "int_mul",
            Self::IntDiv => "int_div",
            Self::FpAdd => "fp_add",
            Self::FpMul => "fp_mul",
            Self::FpFma => "fp_fma",
            Self::FpDivSqrt => "fp_div_sqrt",
            Self::Branch => "branch",
            Self::Mem => "mem",
            Self::VecIntAlu => "vec_int_alu",
            Self::VecIntMul => "vec_int_mul",
            Self::VecIntDiv => "vec_int_div",
            Self::VecFpAlu => "vec_fp_alu",
            Self::VecFpFma => "vec_fp_fma",
            Self::VecFpDivSqrt => "vec_fp_div_sqrt",
            Self::VecMem => "vec_mem",
            Self::VecPermute => "vec_permute",
        }
    }

    /// Returns true if this FU type is a vector execution unit.
    pub const fn is_vector(self) -> bool {
        matches!(
            self,
            Self::VecIntAlu
                | Self::VecIntMul
                | Self::VecIntDiv
                | Self::VecFpAlu
                | Self::VecFpFma
                | Self::VecFpDivSqrt
                | Self::VecMem
                | Self::VecPermute
        )
    }

    /// Classify an instruction's FU type from its control signals.
    pub fn classify(ctrl: &ControlSignals) -> Self {
        if ctrl.vec_op != VectorOp::None {
            return Self::classify_vec(ctrl.vec_op);
        }

        if ctrl.mem_read || ctrl.mem_write || ctrl.atomic_op.is_some() {
            return Self::Mem;
        }
        if ctrl.control_flow != ControlFlow::Sequential {
            return Self::Branch;
        }
        match ctrl.alu {
            AluOp::Mul | AluOp::Mulh | AluOp::Mulhsu | AluOp::Mulhu => Self::IntMul,
            AluOp::Div | AluOp::Divu | AluOp::Rem | AluOp::Remu => Self::IntDiv,
            AluOp::FMul => Self::FpMul,
            AluOp::FDiv | AluOp::FSqrt => Self::FpDivSqrt,
            AluOp::FMAdd | AluOp::FMSub | AluOp::FNMAdd | AluOp::FNMSub => Self::FpFma,
            AluOp::FAdd
            | AluOp::FSub
            | AluOp::FMin
            | AluOp::FMax
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
            | AluOp::FMvToF => Self::FpAdd,
            _ => Self::IntAlu,
        }
    }

    /// Classify a vector operation into a vector FU type.
    #[allow(clippy::enum_glob_use)]
    const fn classify_vec(op: VectorOp) -> Self {
        use VectorOp::*;
        match op {
            VLoadUnit | VStoreUnit | VLoadFF | VLoadMask | VStoreMask | VLoadWholeReg
            | VStoreWholeReg | VLoadStride | VStoreStride | VLoadIndexOrd | VStoreIndexOrd
            | VLoadIndexUnord | VStoreIndexUnord => Self::VecMem,

            Vsetvli | Vsetivli | Vsetvl | VAdd | VSub | VRsub | VAnd | VOr | VXor | VSll | VSrl
            | VSra | VMinU | VMin | VMaxU | VMax | VMerge | VMSeq | VMSne | VMSltu | VMSlt
            | VMSleu | VMSle | VMSgtu | VMSgt | VAdc | VMadc | VSbc | VMsbc | VWAddU | VWAdd
            | VWSubU | VWSub | VWAddUW | VWAddW | VWSubUW | VWSubW | VNSrl | VNSra | VNClipU
            | VNClip | VSAddU | VSAdd | VSSubU | VSSub | VAAddU | VAAdd | VASubU | VASub
            | VSmul | VSSrl | VSSra | VZextVf2 | VZextVf4 | VZextVf8 | VSextVf2 | VSextVf4
            | VSextVf8 | VRedSum | VRedAnd | VRedOr | VRedXor | VRedMinU | VRedMin | VRedMaxU
            | VRedMax | VWRedSumU | VWRedSum
            | VAndN | VBrev | VBrev8 | VRev8 | VClz | VCtz | VCpopV | VRol | VRor | VWsll
            | None
            // Crypto ops route to the vector integer ALU (no separate crypto FU is modelled).
            | VAesEm | VAesEf | VAesDm | VAesDf | VAesZ
            | VAesKf1 | VAesKf2
            | VSha2Ms | VSha2Ch | VSha2Cl
            | VSm3Me | VSm3C
            | VSm4R | VSm4K
            | VGhsh | VGmul => Self::VecIntAlu,

            VMul | VMulh | VMulhu | VMulhsu | VMacc | VNMSac | VMadd | VNMSub | VWMulU | VWMul
            | VWMulSU | VWMaccU | VWMacc | VWMaccSU | VWMaccUS
            | VClMul | VClMulH => Self::VecIntMul,

            VDivU | VDiv | VRemU | VRem => Self::VecIntDiv,

            VFAdd | VFSub | VFRSub | VFMin | VFMax | VFSgnj | VFSgnjn | VFSgnjx | VMFEq | VMFNe
            | VMFLt | VMFLe | VMFGt | VMFGe | VFClass | VFCvtXuF | VFCvtXF | VFCvtFXu | VFCvtFX
            | VFCvtRtzXuF | VFCvtRtzXF | VFWAdd | VFWSub | VFWAddW | VFWSubW | VFWCvtXuF
            | VFWCvtXF | VFWCvtFXu | VFWCvtFX | VFWCvtFF | VFWCvtRtzXuF | VFWCvtRtzXF
            | VFNCvtXuF | VFNCvtXF | VFNCvtFXu | VFNCvtFX | VFNCvtFF | VFNCvtRodFF
            | VFNCvtRtzXuF | VFNCvtRtzXF | VFMerge | VFMvSF | VFMvFS | VFRsqrt7 | VFRec7
            | VFRedOSum | VFRedUSum | VFRedMax | VFRedMin | VFWRedOSum | VFWRedUSum => {
                Self::VecFpAlu
            }

            VFMul | VFMacc | VFNMacc | VFMSac | VFNMSac | VFMAdd | VFNMAdd | VFMSub | VFNMSub
            | VFWMul | VFWMacc | VFWNMacc | VFWMSac | VFWNMSac => Self::VecFpFma,

            VFDiv | VFRDiv | VFSqrt => Self::VecFpDivSqrt,

            VFSlide1Up | VFSlide1Down | VMAndMM | VMNandMM | VMAndnMM | VMOrMM | VMNorMM
            | VMOrnMM | VMXorMM | VMXnorMM | VCPopM | VFirstM | VMSbfM | VMSifM | VMSofM
            | VIotaM | VIdV | VMvXS | VMvSX | VSlideUp | VSlideDown | VSlide1Up | VSlide1Down
            | VRgather | VRgatherEi16 | VCompress | VMv1r | VMv2r | VMv4r | VMv8r => {
                Self::VecPermute
            }

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
}

impl FuUnit {
    /// Returns true if this unit can accept a new instruction at cycle `now`.
    #[inline]
    pub const fn is_free(&self, now: u64) -> bool {
        now >= self.busy_until
    }

    /// Acquire the unit for one instruction issued at cycle `now`.
    /// Returns the cycle at which the result will be ready.
    pub const fn acquire(&mut self, now: u64) -> u64 {
        let complete = now + self.latency;
        self.busy_until = if self.is_pipelined { now + 1 } else { complete };
        complete
    }

    /// Acquire the unit with a dynamic latency (for vector ops where latency
    /// depends on VL/lanes). Returns the cycle at which the result will be ready.
    pub const fn acquire_with_latency(&mut self, now: u64, latency: u64) -> u64 {
        let complete = now + latency;
        self.busy_until = if self.is_pipelined { now + 1 } else { complete };
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
                units.push(FuUnit { fu_type, latency, is_pipelined: pipelined, busy_until: 0 });
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
                units.push(FuUnit { fu_type: ft, latency: lat, is_pipelined: pipe, busy_until: 0 });
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
    pub fn acquire(&mut self, unit: FreeUnit, now: u64) -> u64 {
        self.units[unit.0].acquire(now)
    }

    /// Like [`acquire`](Self::acquire) with a latency the instruction sets
    /// (vector ops, whose latency depends on VL and the lane count).
    pub fn acquire_with_latency(&mut self, unit: FreeUnit, now: u64, latency: u64) -> u64 {
        self.units[unit.0].acquire_with_latency(now, latency)
    }

    /// Returns the latency of the first unit of `fu_type`.
    pub fn get_latency(&self, fu_type: FuType) -> u64 {
        self.units.iter().find(|u| u.fu_type == fu_type).map_or(1, |u| u.latency)
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
        use crate::exec::compute::vector::reduction;
        use crate::isa::op::VecSrcEncoding;
        use crate::uarch::vector::lane_model;

        let startup = self.startup_latency(fu_type);
        let pipelined = self.is_pipelined(fu_type);
        let vec_op = ctrl.vec_op;
        if reduction::is_reduction(vec_op) {
            // Ordered FP reductions are sequential; others use the tree model.
            let is_ordered = matches!(vec_op, VectorOp::VFRedOSum | VectorOp::VFWRedOSum);
            return lane_model::compute_reduction_latency(vl, lanes, startup, is_ordered);
        }
        if fu_type != FuType::VecPermute {
            return lane_model::compute_vec_latency(vl, lanes, startup, pipelined);
        }
        let groups = vl.div_ceil(lanes) as u64;
        let latency = match vec_op {
            VectorOp::VRgather | VectorOp::VRgatherEi16
                if ctrl.vec_src_encoding == VecSrcEncoding::VV =>
            {
                startup + groups.saturating_mul(2).saturating_sub(1)
            }
            VectorOp::VRgather | VectorOp::VRgatherEi16 => startup + groups.saturating_sub(1),
            VectorOp::VCompress => startup + groups.saturating_mul(2).saturating_sub(1),
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

    fn default_pool() -> FuPool {
        FuPool::new(&FuConfig::default())
    }

    #[test]
    fn test_pipelined_unit_free_next_cycle() {
        let mut pool = default_pool();
        assert!(pool.free_unit(FuType::IntAlu, 0).is_some());
        let complete = pool.acquire(pool.free_unit(FuType::IntAlu, 0).unwrap(), 0);
        // Latency = 1, so complete = cycle 1
        assert_eq!(complete, 1);
        // Pipelined: unit is free at cycle 1 (busy_until = 0 + 1 = 1, so is_free at cycle 1)
        assert!(pool.free_unit(FuType::IntAlu, 1).is_some());
        // With 4 int ALUs, even cycle 0 has 3 remaining free after 1 acquired
        assert!(pool.free_unit(FuType::IntAlu, 0).is_some());
    }

    #[test]
    fn test_non_pipelined_holds_for_full_latency() {
        let mut pool = default_pool();
        assert!(pool.free_unit(FuType::IntDiv, 0).is_some());
        let complete = pool.acquire(pool.free_unit(FuType::IntDiv, 0).unwrap(), 0);
        // Latency = 35, so complete = cycle 35
        assert_eq!(complete, 35);
        // Non-pipelined: busy_until = 35, NOT free until cycle 35
        assert!(pool.free_unit(FuType::IntDiv, 1).is_none());
        assert!(pool.free_unit(FuType::IntDiv, 34).is_none());
        assert!(pool.free_unit(FuType::IntDiv, 35).is_some());
    }

    #[test]
    fn test_structural_hazard_all_units_busy() {
        let mut pool = default_pool();
        // FpDivSqrt has count=1
        pool.acquire(pool.free_unit(FuType::FpDivSqrt, 0).unwrap(), 0);
        // No more FpDivSqrt units available
        assert!(pool.free_unit(FuType::FpDivSqrt, 0).is_none());
    }

    #[test]
    fn test_classify_int_alu() {
        let ctrl = ControlSignals { alu: AluOp::Add, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::IntAlu);
    }

    #[test]
    fn test_classify_int_div() {
        let ctrl = ControlSignals { alu: AluOp::Div, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::IntDiv);
    }

    #[test]
    fn test_classify_fp_fma() {
        let ctrl = ControlSignals { alu: AluOp::FMAdd, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::FpFma);
    }

    #[test]
    fn test_classify_branch() {
        let ctrl = ControlSignals { control_flow: ControlFlow::Branch, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::Branch);
    }

    #[test]
    fn test_classify_mem() {
        let ctrl = ControlSignals { mem_read: true, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::Mem);
    }

    #[test]
    fn test_classify_vec_int_alu() {
        let ctrl = ControlSignals { vec_op: VectorOp::VAdd, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecIntAlu);
    }

    #[test]
    fn test_classify_vec_int_mul() {
        let ctrl = ControlSignals { vec_op: VectorOp::VMul, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecIntMul);
    }

    #[test]
    fn test_classify_vec_int_div() {
        let ctrl = ControlSignals { vec_op: VectorOp::VDiv, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecIntDiv);
    }

    #[test]
    fn test_classify_vec_fp_alu() {
        let ctrl = ControlSignals { vec_op: VectorOp::VFAdd, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecFpAlu);
    }

    #[test]
    fn test_classify_vec_fp_fma() {
        let ctrl = ControlSignals { vec_op: VectorOp::VFMacc, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecFpFma);
    }

    #[test]
    fn test_classify_vec_fp_div_sqrt() {
        let ctrl = ControlSignals { vec_op: VectorOp::VFDiv, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecFpDivSqrt);
    }

    #[test]
    fn test_classify_vec_mem() {
        let ctrl = ControlSignals { vec_op: VectorOp::VLoadUnit, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecMem);
    }

    #[test]
    fn test_classify_vec_permute() {
        let ctrl = ControlSignals { vec_op: VectorOp::VSlideUp, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecPermute);
    }

    #[test]
    fn test_acquire_with_latency() {
        let mut pool = default_pool();
        let complete =
            pool.acquire_with_latency(pool.free_unit(FuType::VecIntAlu, 10).unwrap(), 10, 5);
        assert_eq!(complete, 15);
        // Pipelined: unit free next cycle
        assert!(pool.free_unit(FuType::VecIntAlu, 11).is_some());
    }

    #[test]
    fn test_vec_fu_pool_created() {
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
    fn test_classify_vec_reduction() {
        let ctrl = ControlSignals { vec_op: VectorOp::VRedSum, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecIntAlu);

        let ctrl = ControlSignals { vec_op: VectorOp::VFRedUSum, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecFpAlu);
    }

    #[test]
    fn test_classify_vec_mask() {
        let ctrl = ControlSignals { vec_op: VectorOp::VMAndMM, ..Default::default() };
        assert_eq!(FuType::classify(&ctrl), FuType::VecPermute);
    }
}
