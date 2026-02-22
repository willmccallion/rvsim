//! Functional Unit Pool for the O3 backend.
//!
//! Models pipelined and non-pipelined execution units with configurable
//! latencies. Structural hazards are enforced: an instruction cannot issue
//! if all units of the required type are busy.
//!
//! Default latencies are Skylake-class values matching real hardware.

use crate::core::pipeline::signals::{AluOp, ControlSignals};
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
}

/// Number of distinct FU types.
pub const FU_TYPE_COUNT: usize = 9;

impl FuType {
    /// Human-readable name for stats output.
    pub fn name(self) -> &'static str {
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
        }
    }

    /// Classify an instruction's FU type from its control signals.
    pub fn classify(ctrl: &ControlSignals) -> Self {
        if ctrl.mem_read
            || ctrl.mem_write
            || ctrl.atomic_op != crate::core::pipeline::signals::AtomicOp::None
        {
            return Self::Mem;
        }
        if ctrl.branch || ctrl.jump {
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
            | AluOp::FMvToX
            | AluOp::FMvToF => Self::FpAdd,
            _ => Self::IntAlu,
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
    pub fn is_free(&self, now: u64) -> bool {
        now >= self.busy_until
    }

    /// Acquire the unit for one instruction issued at cycle `now`.
    /// Returns the cycle at which the result will be ready.
    pub fn acquire(&mut self, now: u64) -> u64 {
        let complete = now + self.latency;
        self.busy_until = if self.is_pipelined {
            now + 1 // pipelined: free next cycle
        } else {
            complete // non-pipelined: blocked until done
        };
        complete
    }
}

/// Configuration for the functional unit pool.
#[derive(Clone, Debug, Deserialize)]
pub struct FuConfig {
    pub num_int_alu: usize,
    pub int_alu_latency: u64,
    pub num_int_mul: usize,
    pub int_mul_latency: u64,
    pub num_int_div: usize,
    pub int_div_latency: u64,
    pub num_fp_add: usize,
    pub fp_add_latency: u64,
    pub num_fp_mul: usize,
    pub fp_mul_latency: u64,
    pub num_fp_fma: usize,
    pub fp_fma_latency: u64,
    pub num_fp_div_sqrt: usize,
    pub fp_div_sqrt_latency: u64,
    pub num_branch: usize,
    pub branch_latency: u64,
    pub num_mem: usize,
    pub mem_latency: u64,
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
        }
    }
}

/// Pool of heterogeneous functional units.
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
                });
            }
        };

        add(
            &mut units,
            FuType::IntAlu,
            config.num_int_alu,
            config.int_alu_latency,
            true,
        );
        add(
            &mut units,
            FuType::IntMul,
            config.num_int_mul,
            config.int_mul_latency,
            true,
        );
        add(
            &mut units,
            FuType::IntDiv,
            config.num_int_div,
            config.int_div_latency,
            false,
        );
        add(
            &mut units,
            FuType::FpAdd,
            config.num_fp_add,
            config.fp_add_latency,
            true,
        );
        add(
            &mut units,
            FuType::FpMul,
            config.num_fp_mul,
            config.fp_mul_latency,
            true,
        );
        add(
            &mut units,
            FuType::FpFma,
            config.num_fp_fma,
            config.fp_fma_latency,
            true,
        );
        add(
            &mut units,
            FuType::FpDivSqrt,
            config.num_fp_div_sqrt,
            config.fp_div_sqrt_latency,
            false,
        );
        add(
            &mut units,
            FuType::Branch,
            config.num_branch,
            config.branch_latency,
            true,
        );
        add(
            &mut units,
            FuType::Mem,
            config.num_mem,
            config.mem_latency,
            true,
        );

        Self { units }
    }

    /// Returns true if at least one unit of `fu_type` is free at cycle `now`.
    pub fn has_free(&self, fu_type: FuType, now: u64) -> bool {
        self.units
            .iter()
            .any(|u| u.fu_type == fu_type && u.is_free(now))
    }

    /// Acquire a free unit of `fu_type` at cycle `now`.
    /// Returns the cycle at which the result is ready.
    /// **Panics** if no unit is free (caller must call `has_free` first).
    pub fn acquire(&mut self, fu_type: FuType, now: u64) -> u64 {
        for unit in &mut self.units {
            if unit.fu_type == fu_type && unit.is_free(now) {
                return unit.acquire(now);
            }
        }
        panic!("acquire called with no free unit of type {:?}", fu_type);
    }

    /// Returns the latency of the first unit of `fu_type`.
    pub fn get_latency(&self, fu_type: FuType) -> u64 {
        self.units
            .iter()
            .find(|u| u.fu_type == fu_type)
            .map(|u| u.latency)
            .unwrap_or(1)
    }

    /// Returns whether the first unit of `fu_type` is pipelined.
    pub fn is_pipelined(&self, fu_type: FuType) -> bool {
        self.units
            .iter()
            .find(|u| u.fu_type == fu_type)
            .map(|u| u.is_pipelined)
            .unwrap_or(true)
    }
}

#[cfg(test)]
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
        let ctrl = ControlSignals {
            alu: AluOp::Add,
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::IntAlu);
    }

    #[test]
    fn test_classify_int_div() {
        let ctrl = ControlSignals {
            alu: AluOp::Div,
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::IntDiv);
    }

    #[test]
    fn test_classify_fp_fma() {
        let ctrl = ControlSignals {
            alu: AluOp::FMAdd,
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::FpFma);
    }

    #[test]
    fn test_classify_branch() {
        let ctrl = ControlSignals {
            branch: true,
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::Branch);
    }

    #[test]
    fn test_classify_mem() {
        let ctrl = ControlSignals {
            mem_read: true,
            ..Default::default()
        };
        assert_eq!(FuType::classify(&ctrl), FuType::Mem);
    }
}
