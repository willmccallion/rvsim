//! Least Recently Used (LRU) Replacement Policy.
//!
//! Uses a packed-rank encoding to avoid heap allocations on every access.
//!
//! ## Encoding
//!
//! For a W-way cache, each way is assigned a rank in `[0, W)` where 0 = LRU
//! and W-1 = MRU.  All W ranks for one set are packed into a single `u64`:
//! way `i` occupies bits `[i*BITS .. i*BITS + BITS)`.  `BITS = ceil(log2(W))`
//! rounded up to the nearest value in {1,2,3,4} so that W ≤ 16.
//!
//! ## Update algorithm
//!
//! On access to `way`:
//! 1. Read `accessed_rank = rank(way)`.
//! 2. For every other way whose rank is **greater** than `accessed_rank`,
//!    decrement its rank by 1 (it moves one step toward LRU).
//! 3. Set `rank(way) = W - 1`  (promote to MRU).
//!
//! All operations are pure integer arithmetic on one `u64`; no heap traffic.
//!
//! ## Fallback
//!
//! Ways > 16 (or bits-per-rank > 4) fall back to the original `Vec<Vec<usize>>`
//! implementation.  Real caches never exceed 16-way associativity, so this path
//! exists only for completeness.
//!
//! # Performance
//!
//! - **`update()`**: O(W) arithmetic ops on a single register — no allocation,
//!   no pointer chasing, no memmove.
//! - **`get_victim()`**: O(W) rank comparisons, same single-register arithmetic.
//! - **Space**: 8 bytes per set vs. `W * (8 + ptr)` bytes for the Vec approach.

use super::ReplacementPolicy;

// ── packed implementation ─────────────────────────────────────────────────────

/// Returns bits-per-rank for `ways`, or `None` if ways > 16.
const fn bits_for_ways(ways: usize) -> Option<u32> {
    match ways {
        1 => Some(1),
        2 => Some(1),
        3 | 4 => Some(2),
        5..=8 => Some(3),
        9..=16 => Some(4),
        _ => None,
    }
}

#[inline(always)]
fn get_rank(packed: u64, way: usize, bits: u32) -> u64 {
    let mask = (1u64 << bits) - 1;
    (packed >> (way as u32 * bits)) & mask
}

#[inline(always)]
fn set_rank(packed: u64, way: usize, bits: u32, rank: u64) -> u64 {
    let mask = (1u64 << bits) - 1;
    let shift = way as u32 * bits;
    (packed & !(mask << shift)) | (rank << shift)
}

/// Update the packed state for one set on access to `way`.
#[inline(always)]
fn packed_update(mut state: u64, ways: usize, bits: u32, way: usize) -> u64 {
    let accessed_rank = get_rank(state, way, bits);
    // Demote every way ranked above the accessed way.
    for w in 0..ways {
        if w != way {
            let r = get_rank(state, w, bits);
            if r > accessed_rank {
                state = set_rank(state, w, bits, r - 1);
            }
        }
    }
    // Promote accessed way to MRU.
    set_rank(state, way, bits, (ways - 1) as u64)
}

/// Find the way with rank 0 (LRU victim).
#[inline(always)]
fn packed_victim(state: u64, ways: usize, bits: u32) -> usize {
    for w in 0..ways {
        if get_rank(state, w, bits) == 0 {
            return w;
        }
    }
    0 // unreachable for valid state
}

/// Build initial packed state where way 0 is MRU and way W-1 is LRU.
/// (Arbitrary but consistent ordering at cold start.)
fn initial_packed(ways: usize, bits: u32) -> u64 {
    let mut state = 0u64;
    for w in 0..ways {
        // way 0 gets rank W-1 (MRU), way W-1 gets rank 0 (LRU)
        state = set_rank(state, w, bits, (ways - 1 - w) as u64);
    }
    state
}

// ── policy struct ─────────────────────────────────────────────────────────────

enum LruState {
    /// Packed single-u64 per set; used when ways ≤ 16.
    Packed { data: Vec<u64>, ways: usize, bits: u32 },
    /// Fallback for unusual associativities.
    Stacks { usage: Vec<Vec<usize>> },
}

/// LRU replacement policy with packed-rank encoding for low-overhead updates.
pub struct LruPolicy {
    state: LruState,
}

impl LruPolicy {
    /// Creates a new LRU policy for a cache with `sets` sets and `ways` ways.
    pub fn new(sets: usize, ways: usize) -> Self {
        let safe_ways = ways.max(1);
        let state = if let Some(bits) = bits_for_ways(safe_ways) {
            let init = initial_packed(safe_ways, bits);
            LruState::Packed {
                data: vec![init; sets],
                ways: safe_ways,
                bits,
            }
        } else {
            let mut usage = Vec::with_capacity(sets);
            for _ in 0..sets {
                usage.push((0..safe_ways).collect());
            }
            LruState::Stacks { usage }
        };
        Self { state }
    }
}

impl std::fmt::Debug for LruPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.state {
            LruState::Packed { ways, bits, .. } => {
                f.debug_struct("LruPolicy")
                    .field("kind", &"packed")
                    .field("ways", ways)
                    .field("bits_per_rank", bits)
                    .finish()
            }
            LruState::Stacks { usage } => {
                f.debug_struct("LruPolicy")
                    .field("kind", &"stacks")
                    .field("sets", &usage.len())
                    .finish()
            }
        }
    }
}

impl ReplacementPolicy for LruPolicy {
    #[inline(always)]
    fn update(&mut self, set: usize, way: usize) {
        match &mut self.state {
            LruState::Packed { data, ways, bits } => {
                data[set] = packed_update(data[set], *ways, *bits, way);
            }
            LruState::Stacks { usage } => {
                let stack = &mut usage[set];
                if let Some(pos) = stack.iter().position(|&x| x == way) {
                    let _ = stack.remove(pos);
                }
                stack.insert(0, way);
            }
        }
    }

    #[inline(always)]
    fn get_victim(&mut self, set: usize) -> usize {
        match &mut self.state {
            LruState::Packed { data, ways, bits } => {
                packed_victim(data[set], *ways, *bits)
            }
            LruState::Stacks { usage } => {
                usage[set].last().copied().unwrap_or(0)
            }
        }
    }
}
