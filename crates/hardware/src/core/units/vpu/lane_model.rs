//! Vector lane execution model.
//!
//! Computes realistic execution latencies for vector operations based on
//! vector length, number of lanes, and pipeline depth (startup latency).
//! Follows the Patterson/Ara2/Saturn model.

/// Compute total execution latency for a vector operation.
///
/// **Pipelined units** (ALU, multiply, FP add, FP mul/FMA):
/// `total = startup_latency + ceil(VL / num_lanes) - 1`
///
/// **Non-pipelined units** (integer div, FP div/sqrt):
/// `total = ceil(VL / num_lanes) * per_element_latency`
///
/// # Arguments
/// * `vl` - current vector length (number of active elements)
/// * `num_lanes` - number of parallel execution lanes
/// * `startup_latency` - pipeline depth / per-element latency
/// * `is_pipelined` - whether the FU accepts one element-group per cycle
pub const fn compute_vec_latency(
    vl: usize,
    num_lanes: usize,
    startup_latency: u64,
    is_pipelined: bool,
) -> u64 {
    if vl == 0 {
        return 1;
    }
    let groups = vl.div_ceil(num_lanes) as u64;
    if is_pipelined { startup_latency + groups - 1 } else { groups * startup_latency }
}

/// Compute reduction latency.
///
/// **Unordered reductions** (vredsum, vfredsum, etc.):
/// `total = startup + ceil(VL / num_lanes) + log2(num_lanes)`
/// Phase 1: Each lane reduces its local elements sequentially.
/// Phase 2: Inter-lane tree reduction in `log2(num_lanes)` cycles.
///
/// **Ordered FP reductions** (vfredosum, vfredomax, etc.):
/// `total = VL * fp_add_latency`
/// Fully sequential — each element depends on the previous result.
pub fn compute_reduction_latency(
    vl: usize,
    num_lanes: usize,
    startup_latency: u64,
    is_ordered: bool,
) -> u64 {
    if vl == 0 {
        return 1;
    }
    if is_ordered {
        // Fully sequential: each element depends on previous
        vl as u64 * startup_latency
    } else {
        // Intra-lane sequential + inter-lane tree reduction
        let intra = vl.div_ceil(num_lanes) as u64;
        let inter = if num_lanes > 1 { (num_lanes as f64).log2().ceil() as u64 } else { 0 };
        startup_latency + intra + inter
    }
}

/// Cycle at which first element-group result is ready (for chaining).
///
/// Used to enable chaining: dependents can start executing once the first
/// element-group of the producer is ready.
#[inline]
pub const fn first_group_ready(issue_cycle: u64, startup_latency: u64) -> u64 {
    issue_cycle + startup_latency
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipelined_basic() {
        // 4 lanes, startup=1 (int ALU), VL=4 → 1 group → 1 + 1 - 1 = 1
        assert_eq!(compute_vec_latency(4, 4, 1, true), 1);
        // 4 lanes, startup=1, VL=8 → 2 groups → 1 + 2 - 1 = 2
        assert_eq!(compute_vec_latency(8, 4, 1, true), 2);
        // 4 lanes, startup=4 (FP add), VL=16 → 4 groups → 4 + 4 - 1 = 7
        assert_eq!(compute_vec_latency(16, 4, 4, true), 7);
    }

    #[test]
    fn test_non_pipelined() {
        // 4 lanes, per_element=20 (div), VL=8 → 2 groups → 2 * 20 = 40
        assert_eq!(compute_vec_latency(8, 4, 20, false), 40);
        // VL=1 → 1 group → 1 * 20 = 20
        assert_eq!(compute_vec_latency(1, 4, 20, false), 20);
    }

    #[test]
    fn test_vl_zero() {
        assert_eq!(compute_vec_latency(0, 4, 5, true), 1);
        assert_eq!(compute_vec_latency(0, 4, 5, false), 1);
    }

    #[test]
    fn test_partial_lanes() {
        // 4 lanes, startup=1, VL=5 → ceil(5/4)=2 groups → 1 + 2 - 1 = 2
        assert_eq!(compute_vec_latency(5, 4, 1, true), 2);
        // 4 lanes, startup=1, VL=3 → ceil(3/4)=1 group → 1 + 1 - 1 = 1
        assert_eq!(compute_vec_latency(3, 4, 1, true), 1);
    }

    #[test]
    fn test_reduction_unordered() {
        // 4 lanes, startup=1, VL=16 → intra=4, inter=2 → 1 + 4 + 2 = 7
        assert_eq!(compute_reduction_latency(16, 4, 1, false), 7);
        // 1 lane, startup=1, VL=8 → intra=8, inter=0 → 1 + 8 + 0 = 9
        assert_eq!(compute_reduction_latency(8, 1, 1, false), 9);
    }

    #[test]
    fn test_reduction_ordered() {
        // Ordered: VL=8, startup=4 (FP add) → 8 * 4 = 32
        assert_eq!(compute_reduction_latency(8, 4, 4, true), 32);
        // VL=1 → 1 * 4 = 4
        assert_eq!(compute_reduction_latency(1, 4, 4, true), 4);
    }

    #[test]
    fn test_reduction_vl_zero() {
        assert_eq!(compute_reduction_latency(0, 4, 4, true), 1);
        assert_eq!(compute_reduction_latency(0, 4, 4, false), 1);
    }

    #[test]
    fn test_first_group_ready() {
        assert_eq!(first_group_ready(10, 4), 14);
        assert_eq!(first_group_ready(0, 1), 1);
    }

    #[test]
    fn test_single_lane() {
        // 1 lane, startup=3 (int mul), VL=8 → 8 groups → 3 + 8 - 1 = 10
        assert_eq!(compute_vec_latency(8, 1, 3, true), 10);
    }
}
