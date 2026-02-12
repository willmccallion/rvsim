//! Stride Prefetcher.
//!
//! A prefetcher that detects constant stride patterns in memory accesses.
//! It maintains a Reference Prediction Table (RPT) to track the last address
//! and stride for different instruction streams (hashed by address).
//!
//! Prefetching is triggered only when a stable stride pattern is established
//! (confidence threshold is met).
//!
//! # Performance
//!
//! - **Time Complexity:**
//!   - `update()`: O(1)
//!   - `get_prefetch_candidates()`: O(D) where D is the prefetch degree
//! - **Space Complexity:** O(T) where T is the table size (typically 64-256 entries)
//! - **Hardware Cost:** Moderate - small table, simple arithmetic
//! - **Best Case:** Regular strided patterns (array traversals, matrix operations)
//! - **Worst Case:** Irregular or random access patterns (linked lists, hash tables)

use super::Prefetcher;

/// Entry in the Reference Prediction Table.
#[derive(Default, Clone, Copy)]
struct StreamEntry {
    /// The last address accessed by this stream.
    last_addr: u64,
    /// The detected stride (difference between consecutive accesses).
    stride: i64,
    /// Confidence counter (2-bit saturating).
    confidence: u8,
}

/// Stride Prefetcher state.
pub struct StridePrefetcher {
    /// Reference Prediction Table.
    table: Vec<StreamEntry>,
    /// Size of a cache line in bytes.
    line_bytes: u64,
    /// Mask used to index the table.
    table_mask: usize,
    /// Number of lines to prefetch ahead.
    degree: usize,
}

impl StridePrefetcher {
    /// Creates a new Stride prefetcher.
    ///
    /// # Arguments
    ///
    /// * `line_bytes` - The size of a cache line in bytes.
    /// * `table_size` - Number of entries in the tracking table (must be power of 2).
    /// * `degree` - The number of strides to prefetch ahead.
    pub fn new(line_bytes: usize, table_size: usize, degree: usize) -> Self {
        let safe_size = if table_size > 0 && (table_size & (table_size - 1)) == 0 {
            table_size
        } else {
            64
        };

        Self {
            table: vec![StreamEntry::default(); safe_size],
            line_bytes: line_bytes as u64,
            table_mask: safe_size - 1,
            degree: if degree == 0 { 1 } else { degree },
        }
    }
}

impl Prefetcher for StridePrefetcher {
    /// Observes a memory access and generates prefetch candidates.
    ///
    /// Updates the tracking table with the current address. If a consistent
    /// stride is detected (confidence > 1), generates prefetch requests
    /// for future addresses based on that stride.
    ///
    /// # Arguments
    ///
    /// * `addr` - The memory address being accessed.
    /// * `_hit` - Whether the access was a cache hit (ignored).
    ///
    /// # Returns
    ///
    /// A vector of addresses to prefetch.
    fn observe(&mut self, addr: u64, _hit: bool) -> Vec<u64> {
        let idx = ((addr >> 6) as usize) & self.table_mask;
        let entry = &mut self.table[idx];

        let current_stride = (addr as i64) - (entry.last_addr as i64);
        let mut prefetches = Vec::new();

        if current_stride == entry.stride {
            if entry.confidence < 3 {
                entry.confidence += 1;
            } else {
                for k in 1..=self.degree {
                    let lookahead = entry.stride * k as i64;
                    let target = (addr as i64 + lookahead) as u64;

                    let aligned = target & !(self.line_bytes - 1);
                    prefetches.push(aligned);
                }
            }
        } else if entry.confidence > 0 {
            entry.confidence -= 1;
        } else {
            entry.stride = current_stride;
        }

        entry.last_addr = addr;
        prefetches
    }
}
