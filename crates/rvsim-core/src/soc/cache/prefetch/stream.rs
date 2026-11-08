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
use crate::common::VirtAddr;

/// The most confidence a stream builds up.
const MAX_CONFIDENCE: u8 = 3;

/// The confidence from which the prefetcher follows a stream.
const PREFETCH_CONFIDENCE: u8 = 2;

/// Direction of the memory stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    /// Ascending memory addresses.
    Ascending,
    /// Descending memory addresses.
    Descending,
}

/// Stream Prefetcher state.
#[derive(Debug)]
pub struct StreamPrefetcher {
    /// Size of a cache line in bytes.
    line_bytes: u64,
    /// Number of lines to prefetch ahead.
    degree: usize,
    /// The address the previous access touched.
    last_addr: u64,
    /// The current detected stream direction, if any.
    direction: Option<Direction>,
    /// Confidence counter for the current stream.
    confidence: u8,
}

impl StreamPrefetcher {
    /// A stream prefetcher fetching `degree` lines ahead.
    pub const fn new(line_bytes: usize, degree: usize) -> Self {
        Self {
            line_bytes: line_bytes as u64,
            degree: if degree == 0 { 1 } else { degree },
            last_addr: 0,
            direction: None,
            confidence: 0,
        }
    }
}

impl Prefetcher for StreamPrefetcher {
    /// Compares the address with the previous one; once a contiguous
    /// stream is seen with enough confidence, prefetches along it.
    fn observe(&mut self, addr: u64, _pc: Option<VirtAddr>, _hit: bool) -> Vec<u64> {
        let mut prefetches = Vec::new();
        let diff = (addr as i64) - (self.last_addr as i64);
        let line_sz = self.line_bytes as i64;

        let step = if diff == line_sz {
            Some(Direction::Ascending)
        } else if diff == -line_sz {
            Some(Direction::Descending)
        } else {
            None
        };

        match step {
            Some(_) if step == self.direction => {
                if self.confidence < MAX_CONFIDENCE {
                    self.confidence += 1;
                }
            }
            Some(_) => {
                self.direction = step;
                self.confidence = 1;
            }
            None if self.confidence > 0 => self.confidence -= 1,
            None => self.direction = None,
        }

        if let Some(direction) = self.direction.filter(|_| self.confidence >= PREFETCH_CONFIDENCE) {
            for k in 1..=self.degree {
                let offset = if direction == Direction::Ascending {
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
