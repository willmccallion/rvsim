//! Functional Unit Pool for the O3 backend.
//!
//! Models pipelined and non-pipelined execution units with configurable
//! latencies. Structural hazards are enforced: an instruction cannot issue
//! if all units of the required type are busy.
//!
//! Default latencies are Skylake-class values matching real hardware.

use crate::core::pipeline::signals::{AluOp, ControlFlow, ControlSignals, VectorOp};
use serde::Deserialize;

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

    /// Classify an instruction's FU type from its control signals.
    pub fn classify(ctrl: &ControlSignals) -> Self {
        // Check vector operations first
        if ctrl.vec_op != VectorOp::None {
            return Self::classify_vec(ctrl.vec_op);
        }

        if ctrl.mem_read
            || ctrl.mem_write
            || ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None
        {
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
            // Vector memory operations
            VLoadUnit | VStoreUnit | VLoadFF | VLoadMask | VStoreMask | VLoadWholeReg
            | VStoreWholeReg | VLoadStride | VStoreStride | VLoadIndexOrd | VStoreIndexOrd
            | VLoadIndexUnord | VStoreIndexUnord => Self::VecMem,

            // Integer ALU: configuration + add/sub/logic/shift/compare/merge/ext/
            // widening-add/sub/narrowing-shift/saturating/averaging/fixed-point/reductions
            Vsetvli | Vsetivli | Vsetvl | VAdd | VSub | VRsub | VAnd | VOr | VXor | VSll | VSrl
            | VSra | VMinU | VMin | VMaxU | VMax | VMerge | VMSeq | VMSne | VMSltu | VMSlt
            | VMSleu | VMSle | VMSgtu | VMSgt | VAdc | VMadc | VSbc | VMsbc | VWAddU | VWAdd
            | VWSubU | VWSub | VWAddUW | VWAddW | VWSubUW | VWSubW | VNSrl | VNSra | VNClipU
            | VNClip | VSAddU | VSAdd | VSSubU | VSSub | VAAddU | VAAdd | VASubU | VASub
            | VSmul | VSSrl | VSSra | VZextVf2 | VZextVf4 | VZextVf8 | VSextVf2 | VSextVf4
            | VSextVf8 | VRedSum | VRedAnd | VRedOr | VRedXor | VRedMinU | VRedMin | VRedMaxU
            | VRedMax | VWRedSumU | VWRedSum | None => Self::VecIntAlu,

            // Integer multiply: mul/mulh/macc/madd/widening mul
            VMul | VMulh | VMulhu | VMulhsu | VMacc | VNMSac | VMadd | VNMSub | VWMulU | VWMul
            | VWMulSU | VWMaccU | VWMacc | VWMaccSU | VWMaccUS => Self::VecIntMul,

            // Integer divide/remainder
            VDivU | VDiv | VRemU | VRem => Self::VecIntDiv,

            // FP ALU: fadd/fsub/fmin/fmax/fcmp/fcvt/fclass/fsgnj + widening/narrowing add/sub/cvt
            // + FP reductions (ordered and unordered)
            VFAdd | VFSub | VFRSub | VFMin | VFMax | VFSgnj | VFSgnjn | VFSgnjx | VMFEq | VMFNe
            | VMFLt | VMFLe | VMFGt | VMFGe | VFClass | VFCvtXuF | VFCvtXF | VFCvtFXu | VFCvtFX
            | VFCvtRtzXuF | VFCvtRtzXF | VFWAdd | VFWSub | VFWAddW | VFWSubW | VFWCvtXuF
            | VFWCvtXF | VFWCvtFXu | VFWCvtFX | VFWCvtFF | VFWCvtRtzXuF | VFWCvtRtzXF
            | VFNCvtXuF | VFNCvtXF | VFNCvtFXu | VFNCvtFX | VFNCvtFF | VFNCvtRodFF
            | VFNCvtRtzXuF | VFNCvtRtzXF | VFMerge | VFMvSF | VFMvFS | VFRsqrt7 | VFRec7
            | VFRedOSum | VFRedUSum | VFRedMax | VFRedMin | VFWRedOSum | VFWRedUSum => {
                Self::VecFpAlu
            }

            // FP FMA: fmul/fmadd/fmsub/fnmadd/fnmsub/widening fmul/fmacc
            VFMul | VFMacc | VFNMacc | VFMSac | VFNMSac | VFMAdd | VFNMAdd | VFMSub | VFNMSub
            | VFWMul | VFWMacc | VFWNMacc | VFWMSac | VFWNMSac => Self::VecFpFma,

            // FP div/sqrt
            VFDiv | VFRDiv | VFSqrt => Self::VecFpDivSqrt,

            // Permutation: FP slides + mask logical/population/set-before/iota/id
            // + integer slides/gathers/compress/whole-register moves/scalar↔vector moves
            VFSlide1Up | VFSlide1Down | VMAndMM | VMNandMM | VMAndnMM | VMOrMM | VMNorMM
            | VMOrnMM | VMXorMM | VMXnorMM | VCPopM | VFirstM | VMSbfM | VMSifM | VMSofM
            | VIotaM | VIdV | VMvXS | VMvSX | VSlideUp | VSlideDown | VSlide1Up | VSlide1Down
            | VRgather | VRgatherEi16 | VCompress | VMv1r | VMv2r | VMv4r | VMv8r => {
                Self::VecPermute
            }
        }
    }
}

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
        self.busy_until = if self.is_pipelined {
            now + 1 // pipelined: free next cycle
        } else {
            complete // non-pipelined: blocked until done
        };
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

/// Configuration for the functional unit pool.
#[derive(Clone, Debug, Deserialize)]
pub struct FuConfig {
    /// Number of integer ALU units.
    pub num_int_alu: usize,
    /// Latency of integer ALU operations in cycles.
    pub int_alu_latency: u64,
    /// Number of integer multiplier units.
    pub num_int_mul: usize,
    /// Latency of integer multiply operations in cycles.
    pub int_mul_latency: u64,
    /// Number of integer divider units.
    pub num_int_div: usize,
    /// Latency of integer divide operations in cycles.
    pub int_div_latency: u64,
    /// Number of floating-point adder units.
    pub num_fp_add: usize,
    /// Latency of floating-point add operations in cycles.
    pub fp_add_latency: u64,
    /// Number of floating-point multiplier units.
    pub num_fp_mul: usize,
    /// Latency of floating-point multiply operations in cycles.
    pub fp_mul_latency: u64,
    /// Number of floating-point fused multiply-add units.
    pub num_fp_fma: usize,
    /// Latency of floating-point FMA operations in cycles.
    pub fp_fma_latency: u64,
    /// Number of floating-point divide/sqrt units.
    pub num_fp_div_sqrt: usize,
    /// Latency of floating-point divide/sqrt operations in cycles.
    pub fp_div_sqrt_latency: u64,
    /// Number of branch units.
    pub num_branch: usize,
    /// Latency of branch operations in cycles.
    pub branch_latency: u64,
    /// Number of memory (load/store) units.
    pub num_mem: usize,
    /// Latency of memory operations in cycles.
    pub mem_latency: u64,
    /// Number of vector integer ALU units.
    #[serde(default = "default_num_vec_int_alu")]
    pub num_vec_int_alu: usize,
    /// Startup latency of vector integer ALU operations.
    #[serde(default = "default_vec_int_alu_latency")]
    pub vec_int_alu_latency: u64,
    /// Number of vector integer multiplier units.
    #[serde(default = "default_num_vec_int_mul")]
    pub num_vec_int_mul: usize,
    /// Startup latency of vector integer multiply operations.
    #[serde(default = "default_vec_int_mul_latency")]
    pub vec_int_mul_latency: u64,
    /// Number of vector integer divider units.
    #[serde(default = "default_num_vec_int_div")]
    pub num_vec_int_div: usize,
    /// Per-element latency of vector integer divide operations.
    #[serde(default = "default_vec_int_div_latency")]
    pub vec_int_div_latency: u64,
    /// Number of vector FP ALU units.
    #[serde(default = "default_num_vec_fp_alu")]
    pub num_vec_fp_alu: usize,
    /// Startup latency of vector FP ALU operations.
    #[serde(default = "default_vec_fp_alu_latency")]
    pub vec_fp_alu_latency: u64,
    /// Number of vector FP FMA units.
    #[serde(default = "default_num_vec_fp_fma")]
    pub num_vec_fp_fma: usize,
    /// Startup latency of vector FP FMA operations.
    #[serde(default = "default_vec_fp_fma_latency")]
    pub vec_fp_fma_latency: u64,
    /// Number of vector FP div/sqrt units.
    #[serde(default = "default_num_vec_fp_div_sqrt")]
    pub num_vec_fp_div_sqrt: usize,
    /// Per-element latency of vector FP div/sqrt operations.
    #[serde(default = "default_vec_fp_div_sqrt_latency")]
    pub vec_fp_div_sqrt_latency: u64,
    /// Number of vector memory units.
    #[serde(default = "default_num_vec_mem")]
    pub num_vec_mem: usize,
    /// Startup latency of vector memory operations.
    #[serde(default = "default_vec_mem_latency")]
    pub vec_mem_latency: u64,
    /// Number of vector permute units.
    #[serde(default = "default_num_vec_permute")]
    pub num_vec_permute: usize,
    /// Startup latency of vector permute operations.
    #[serde(default = "default_vec_permute_latency")]
    pub vec_permute_latency: u64,
}

// Serde default functions for vector FU config
const fn default_num_vec_int_alu() -> usize {
    1
}
const fn default_vec_int_alu_latency() -> u64 {
    1
}
const fn default_num_vec_int_mul() -> usize {
    1
}
const fn default_vec_int_mul_latency() -> u64 {
    3
}
const fn default_num_vec_int_div() -> usize {
    1
}
const fn default_vec_int_div_latency() -> u64 {
    20
}
const fn default_num_vec_fp_alu() -> usize {
    1
}
const fn default_vec_fp_alu_latency() -> u64 {
    4
}
const fn default_num_vec_fp_fma() -> usize {
    1
}
const fn default_vec_fp_fma_latency() -> u64 {
    5
}
const fn default_num_vec_fp_div_sqrt() -> usize {
    1
}
const fn default_vec_fp_div_sqrt_latency() -> u64 {
    20
}
const fn default_num_vec_mem() -> usize {
    1
}
const fn default_vec_mem_latency() -> u64 {
    1
}
const fn default_num_vec_permute() -> usize {
    1
}
const fn default_vec_permute_latency() -> u64 {
    1
}

impl Default for FuConfig {
    fn default() -> Self {
        Self {
            num_int_alu: 4,
            int_alu_latency: 1,
            num_int_mul: 1,
            int_mul_latency: 3,
            num_int_div: 1,
            int_div_latency: 35,
            num_fp_add: 2,
            fp_add_latency: 4,
            num_fp_mul: 2,
            fp_mul_latency: 5,
            num_fp_fma: 2,
            fp_fma_latency: 5,
            num_fp_div_sqrt: 1,
            fp_div_sqrt_latency: 21,
            num_branch: 2,
            branch_latency: 1,
            num_mem: 2,
            mem_latency: 1,
            num_vec_int_alu: default_num_vec_int_alu(),
            vec_int_alu_latency: default_vec_int_alu_latency(),
            num_vec_int_mul: default_num_vec_int_mul(),
            vec_int_mul_latency: default_vec_int_mul_latency(),
            num_vec_int_div: default_num_vec_int_div(),
            vec_int_div_latency: default_vec_int_div_latency(),
            num_vec_fp_alu: default_num_vec_fp_alu(),
            vec_fp_alu_latency: default_vec_fp_alu_latency(),
            num_vec_fp_fma: default_num_vec_fp_fma(),
            vec_fp_fma_latency: default_vec_fp_fma_latency(),
            num_vec_fp_div_sqrt: default_num_vec_fp_div_sqrt(),
            vec_fp_div_sqrt_latency: default_vec_fp_div_sqrt_latency(),
            num_vec_mem: default_num_vec_mem(),
            vec_mem_latency: default_vec_mem_latency(),
            num_vec_permute: default_num_vec_permute(),
            vec_permute_latency: default_vec_permute_latency(),
        }
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

        // Vector FUs
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

        Self { units }
    }

    /// Returns true if at least one unit of `fu_type` is free at cycle `now`.
    pub fn has_free(&self, fu_type: FuType, now: u64) -> bool {
        self.units.iter().any(|u| u.fu_type == fu_type && u.is_free(now))
    }

    /// Acquire a free unit of `fu_type` at cycle `now`.
    /// Returns the cycle at which the result is ready.
    ///
    /// # Panics
    ///
    /// Panics if no unit of `fu_type` is free (caller must call `has_free` first).
    pub fn acquire(&mut self, fu_type: FuType, now: u64) -> u64 {
        for unit in &mut self.units {
            if unit.fu_type == fu_type && unit.is_free(now) {
                return unit.acquire(now);
            }
        }
        panic!("acquire called with no free unit of type {fu_type:?}");
    }

    /// Acquire a free unit with a dynamic latency (for vector ops).
    /// Returns the cycle at which the result is ready.
    ///
    /// # Panics
    ///
    /// Panics if no unit of `fu_type` is free.
    pub fn acquire_with_latency(&mut self, fu_type: FuType, now: u64, latency: u64) -> u64 {
        for unit in &mut self.units {
            if unit.fu_type == fu_type && unit.is_free(now) {
                return unit.acquire_with_latency(now, latency);
            }
        }
        panic!("acquire_with_latency called with no free unit of type {fu_type:?}");
    }

    /// Returns the latency of the first unit of `fu_type`.
    pub fn get_latency(&self, fu_type: FuType) -> u64 {
        self.units.iter().find(|u| u.fu_type == fu_type).map_or(1, |u| u.latency)
    }

    /// Returns whether the first unit of `fu_type` is pipelined.
    pub fn is_pipelined(&self, fu_type: FuType) -> bool {
        self.units.iter().find(|u| u.fu_type == fu_type).is_none_or(|u| u.is_pipelined)
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
        assert!(pool.has_free(FuType::IntAlu, 0));
        let complete = pool.acquire(FuType::IntAlu, 0);
        // Latency = 1, so complete = cycle 1
        assert_eq!(complete, 1);
        // Pipelined: unit is free at cycle 1 (busy_until = 0 + 1 = 1, so is_free at cycle 1)
        assert!(pool.has_free(FuType::IntAlu, 1));
        // With 4 int ALUs, even cycle 0 has 3 remaining free after 1 acquired
        assert!(pool.has_free(FuType::IntAlu, 0));
    }

    #[test]
    fn test_non_pipelined_holds_for_full_latency() {
        let mut pool = default_pool();
        assert!(pool.has_free(FuType::IntDiv, 0));
        let complete = pool.acquire(FuType::IntDiv, 0);
        // Latency = 35, so complete = cycle 35
        assert_eq!(complete, 35);
        // Non-pipelined: busy_until = 35, NOT free until cycle 35
        assert!(!pool.has_free(FuType::IntDiv, 1));
        assert!(!pool.has_free(FuType::IntDiv, 34));
        assert!(pool.has_free(FuType::IntDiv, 35));
    }

    #[test]
    fn test_structural_hazard_all_units_busy() {
        let mut pool = default_pool();
        // FpDivSqrt has count=1
        pool.acquire(FuType::FpDivSqrt, 0);
        // No more FpDivSqrt units available
        assert!(!pool.has_free(FuType::FpDivSqrt, 0));
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
        let complete = pool.acquire_with_latency(FuType::VecIntAlu, 10, 5);
        assert_eq!(complete, 15);
        // Pipelined: unit free next cycle
        assert!(pool.has_free(FuType::VecIntAlu, 11));
    }

    #[test]
    fn test_vec_fu_pool_created() {
        let pool = default_pool();
        assert!(pool.has_free(FuType::VecIntAlu, 0));
        assert!(pool.has_free(FuType::VecIntMul, 0));
        assert!(pool.has_free(FuType::VecIntDiv, 0));
        assert!(pool.has_free(FuType::VecFpAlu, 0));
        assert!(pool.has_free(FuType::VecFpFma, 0));
        assert!(pool.has_free(FuType::VecFpDivSqrt, 0));
        assert!(pool.has_free(FuType::VecMem, 0));
        assert!(pool.has_free(FuType::VecPermute, 0));
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
