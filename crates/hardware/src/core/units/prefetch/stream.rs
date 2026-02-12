//! Stream Prefetcher.
//!
//! A prefetcher designed to detect and lock onto sequential access streams.
//! Unlike the Stride prefetcher which looks for arbitrary deltas, the Stream
//! prefetcher specifically optimizes for contiguous forward or backward
//! memory access patterns (stride +1 or -1 cache lines).
//!
//! It maintains a history of the last access to determine direction. Once a
//! direction is established, it prefetches multiple lines ahead.

use super::Prefetcher;

/// Direction of the memory stream.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// No stable direction detected.
    None,
    /// Ascending memory addresses.
    Ascending,
    /// Descending memory addresses.
    Descending,
}

/// Stream Prefetcher state.
pub struct StreamPrefetcher {
    /// Size of a cache line in bytes.
    line_bytes: u64,
    /// Number of lines to prefetch ahead.
    degree: usize,
    /// The address accessed in the previous cycle.
    last_addr: u64,
    /// The current detected stream direction.
    direction: Direction,
    /// Confidence counter for the current stream.
    confidence: u8,
}

impl StreamPrefetcher {
    /// Creates a new Stream prefetcher.
    ///
    /// # Arguments
    ///
    /// * `line_bytes` - The size of a cache line in bytes.
    /// * `degree` - The number of lines to prefetch ahead.
    pub fn new(line_bytes: usize, degree: usize) -> Self {
        Self {
            line_bytes: line_bytes as u64,
            degree: if degree == 0 { 1 } else { degree },
            last_addr: 0,
            direction: Direction::None,
            confidence: 0,
        }
    }
}

impl Prefetcher for StreamPrefetcher {
    /// Observes a memory access and generates prefetch candidates.
    ///
    /// Compares the current address with the previous address to determine
    /// linearity. If a contiguous stream is detected with sufficient confidence,
    /// it generates prefetch requests in the direction of the stream.
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
        let mut prefetches = Vec::new();
        let diff = (addr as i64) - (self.last_addr as i64);
        let line_sz = self.line_bytes as i64;

        let current_dir = if diff == line_sz {
            Direction::Ascending
        } else if diff == -line_sz {
            Direction::Descending
        } else {
            Direction::None
        };

        if current_dir != Direction::None {
            if current_dir == self.direction {
                if self.confidence < 3 {
                    self.confidence += 1;
                }
            } else {
                self.direction = current_dir;
                self.confidence = 1;
            }
        } else {
            if self.confidence > 0 {
                self.confidence -= 1;
            } else {
                self.direction = Direction::None;
            }
        }

        if self.confidence >= 2 {
            for k in 1..=self.degree {
                let offset = if self.direction == Direction::Ascending {
                    (k as u64) * self.line_bytes
                } else {
                    ((k as i64) * -(line_sz)) as u64
                };

                let target = (addr & !(self.line_bytes - 1)).wrapping_add(offset);
                prefetches.push(target);
            }
        }

        self.last_addr = addr;
        prefetches
    }
}
